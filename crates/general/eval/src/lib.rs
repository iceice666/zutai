//! Interim THIR reference interpreter for Zutai general mode (`.zt`).
//!
//! ## Design
//! This crate is a *semantics oracle*: it REFUSES to evaluate any program that
//! is not fully type-checked by THIR.  The pre-flight gate (`check_runnable`)
//! guarantees that no `ThirExprKind::Error` node is reachable before evaluation
//! begins, so a returned `Value` is always a faithful representation of what
//! the program's final expression evaluates to.
//!
//! ## IR-agnostic core
//! The modules `value`, `thunk`, and `env` are independent of any specific IR.
//! Only `eval` imports THIR types.  When the TLC crate exists, a parallel
//! `eval_tlc` module can be added and the public `eval_file` function updated
//! to drive it — no changes required in the runtime-core modules.
//!
//! ## Note on resource management
//! Top-level evaluation builds a `letrec` environment where closures capture
//! the environment and the environment contains closures, creating `Rc` cycles.
//! This is an intentional per-run leak: the entire env graph is dropped at the
//! end of `eval_file`, which is acceptable for an interactive/batch tool.

pub mod env;
pub mod eval;
pub mod thunk;
pub mod value;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use zutai_hir::BindingId;
use zutai_thir::{ThirDeclId, ThirExprKind, ThirFile};

pub use value::Value;

// ─── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    /// Source program has parse or HIR errors.
    NotRunnable(Vec<String>),
    /// THIR type checking failed or is incomplete.
    TypeCheckFailed(Vec<String>),
    /// A `ThirExprKind::Error` node was reachable in a nominally-complete THIR.
    ErrorNodeReachable,
    /// Runtime black-hole: a non-productive recursive binding was forced.
    BlackHole,
    /// Division by zero in integer division.
    DivByZero,
    /// Integer overflow.
    IntOverflow(&'static str),
    /// No clause of a function matched the arguments.
    NoMatchingClause,
    /// An unbound `BindingId` was looked up (unreachable in well-typed code).
    UnboundBinding(BindingId),
    /// Runtime type mismatch (unreachable in well-typed code).
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    /// Internal invariant violated (always a bug in the interpreter).
    Internal(&'static str),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::NotRunnable(msgs) => {
                write!(f, "program has errors and cannot be evaluated:")?;
                for m in msgs {
                    write!(f, "\n  {m}")?;
                }
                Ok(())
            }
            EvalError::TypeCheckFailed(msgs) => {
                write!(f, "type checking failed:")?;
                for m in msgs {
                    write!(f, "\n  {m}")?;
                }
                Ok(())
            }
            EvalError::ErrorNodeReachable => {
                write!(f, "internal: reachable Error node in type-checked THIR")
            }
            EvalError::BlackHole => write!(
                f,
                "runtime error: non-productive recursive definition (black hole)"
            ),
            EvalError::DivByZero => write!(f, "runtime error: integer division by zero"),
            EvalError::IntOverflow(op) => {
                write!(f, "runtime error: integer overflow in `{op}`")
            }
            EvalError::NoMatchingClause => {
                write!(
                    f,
                    "runtime error: no matching clause (non-exhaustive pattern match)"
                )
            }
            EvalError::UnboundBinding(id) => {
                write!(f, "internal: unbound binding {:?}", id)
            }
            EvalError::TypeMismatch { expected, found } => {
                write!(
                    f,
                    "internal: type mismatch — expected {expected}, found {found}"
                )
            }
            EvalError::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for EvalError {}

// ─── pre-flight gate ──────────────────────────────────────────────────────────

/// Check that `analysis` is safe to evaluate.
///
/// Returns a reference to the completed `ThirFile` or an `EvalError`
/// describing exactly why evaluation is blocked.
pub fn check_runnable(analysis: &zutai_semantic::Analysis) -> Result<&ThirFile, EvalError> {
    // 1. Reject if parse or HIR diagnostics are present.
    let blocking: Vec<String> = analysis
        .blocking_diagnostics()
        .map(|d| format!("{:?}", d.kind))
        .collect();
    if !blocking.is_empty() {
        return Err(EvalError::NotRunnable(blocking));
    }

    // 2. Reject if THIR type checking failed or is incomplete.
    if !analysis.is_thir_complete() {
        let thir_msgs: Vec<String> = analysis
            .thir
            .as_ref()
            .map(|lt| {
                lt.diagnostics
                    .iter()
                    .map(|d| format_thir_diagnostic(d))
                    .collect()
            })
            .unwrap_or_default();
        return Err(EvalError::TypeCheckFailed(thir_msgs));
    }

    // 3. Unwrap the ThirFile (guaranteed Some by is_thir_complete).
    let file = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();

    // 4. Belt-and-suspenders: walk reachable exprs for Error nodes.
    if has_reachable_error(file) {
        return Err(EvalError::ErrorNodeReachable);
    }

    Ok(file)
}

fn format_thir_diagnostic(d: &zutai_thir::ThirDiagnostic) -> String {
    use zutai_thir::ThirDiagnosticKind::*;
    match &d.kind {
        TypeMismatch { expected, found } => {
            format!("type mismatch: expected {expected}, found {found}")
        }
        UnsupportedFeature { feature } => {
            format!("unsupported feature: {feature}")
        }
        ExpectedFunction { found } => format!("expected function, found {found}"),
        FunctionClauseArityMismatch { expected, found } => {
            format!("function clause arity mismatch: expected {expected} params, found {found}")
        }
        ExpectedRecord { found } => format!("expected record, found {found}"),
        ExpectedList { found } => format!("expected list, found {found}"),
        ExpectedTuple { found } => format!("expected tuple, found {found}"),
        ExpectedOptional { found } => format!("expected optional, found {found}"),
        EmptyListNeedsType => "empty list needs a type annotation".to_string(),
        TupleArityMismatch { expected, found } => {
            format!("tuple arity mismatch: expected {expected}, found {found}")
        }
        TupleFieldNameMismatch { expected, found } => {
            format!("tuple field name mismatch: expected {expected}, found {found}")
        }
        InvalidBinaryOperands { op, lhs, rhs } => {
            format!("invalid binary operands for `{op}`: {lhs} and {rhs}")
        }
        MissingRecordField { name } => format!("missing required record field `{name}`"),
        UnexpectedRecordField { name } => format!("unexpected record field `{name}`"),
        UnknownField { name } => format!("unknown field `{name}`"),
        AliasCycle { name } => format!("type alias cycle involving `{name}`"),
        ValueTypeUnavailable { name } => format!("type of `{name}` is unavailable"),
        InvalidTypeExpression { reason } => format!("invalid type expression: {reason}"),
        TypeCheckerNotImplemented => "type checker not yet implemented for this form".to_string(),
        LambdaNeedsTypeContext => "lambda expression requires type context".to_string(),
        MatchArmPatternCountMismatch { found } => {
            format!("match arm must have exactly 1 pattern, found {found}")
        }
        NonExhaustiveMatch { witness } => {
            format!("non-exhaustive patterns: `{witness}` not covered")
        }
        UnreachableMatchArm => "unreachable match arm".to_string(),
    }
}

/// Walk all reachable expressions in `file` and check for `Error` nodes.
fn has_reachable_error(file: &ThirFile) -> bool {
    // Check the final expression and all top-level declaration expressions.
    let mut to_visit: Vec<zutai_thir::ThirExprId> = vec![file.final_expr];
    for decl in &file.decl_arena {
        match &decl.kind {
            zutai_thir::ThirDeclKind::Value { value, .. } => to_visit.push(*value),
            zutai_thir::ThirDeclKind::Function { clauses, .. } => {
                for clause in clauses {
                    to_visit.push(clause.body);
                    if let Some(g) = clause.guard {
                        to_visit.push(g);
                    }
                }
            }
            zutai_thir::ThirDeclKind::TypeAlias { .. } => {}
        }
    }

    let mut visited = std::collections::HashSet::new();
    let mut stack = to_visit;
    while let Some(id) = stack.pop() {
        if !visited.insert(id.0) {
            continue;
        }
        let expr = &file.expr_arena[id.0 as usize];
        match &expr.kind {
            ThirExprKind::Error => return true,
            ThirExprKind::Block { bindings, result } => {
                for b in bindings {
                    stack.push(b.value);
                }
                stack.push(*result);
            }
            ThirExprKind::Lambda { body, .. } => stack.push(*body),
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                stack.extend([*cond, *then_branch, *else_branch]);
            }
            ThirExprKind::Match { scrutinee, arms } => {
                stack.push(*scrutinee);
                for arm in arms {
                    stack.push(arm.body);
                    if let Some(g) = arm.guard {
                        stack.push(g);
                    }
                }
            }
            ThirExprKind::Apply { func, arg, .. } => stack.extend([*func, *arg]),
            ThirExprKind::Binary { lhs, rhs, .. } => stack.extend([*lhs, *rhs]),
            ThirExprKind::Access { receiver, .. }
            | ThirExprKind::OptionalAccess { receiver, .. } => stack.push(*receiver),
            ThirExprKind::Record(fields) => {
                for f in fields {
                    stack.push(f.value);
                }
            }
            ThirExprKind::Tuple(items) => {
                for item in items {
                    match item {
                        zutai_thir::ThirTupleItem::Named { value, .. } => stack.push(*value),
                        zutai_thir::ThirTupleItem::Positional(e) => stack.push(*e),
                    }
                }
            }
            ThirExprKind::List(items) => stack.extend(items.iter().copied()),
            // Leaves — no sub-expressions.
            ThirExprKind::True
            | ThirExprKind::False
            | ThirExprKind::Integer(_)
            | ThirExprKind::Float(_)
            | ThirExprKind::String(_)
            | ThirExprKind::Atom(_)
            | ThirExprKind::BindingRef(_)
            | ThirExprKind::Import(_)
            | ThirExprKind::TypeValue(_) => {}
        }
    }
    false
}

// ─── public entry points ──────────────────────────────────────────────────────

/// Evaluate a `.zt` source string and return the forced final value.
///
/// This is the full pipeline: `semantic::analyze` → gate → build env →
/// eval final expression → deep-force.
pub fn eval_file(source: &str) -> Result<Value, EvalError> {
    let analysis = zutai_semantic::analyze(source);
    let file = check_runnable(&analysis)?;
    eval_thir(file)
}

/// Evaluate a pre-analyzed, gate-checked `ThirFile`.
pub fn eval_thir(file: &ThirFile) -> Result<Value, EvalError> {
    let decls_by_binding: HashMap<BindingId, ThirDeclId> = file
        .decls
        .iter()
        .map(|&id| (file.decl_arena[id.0 as usize].binding, id))
        .collect();

    let evaluator = eval::Evaluator::new(file, &decls_by_binding);
    let top = evaluator.build_top_env();
    let result = evaluator.eval(file.final_expr, &top)?;
    Ok(force_deep(result, &evaluator)?)
}

/// Recursively force all lazy thunks inside a value so it can be displayed.
///
/// Only descends into finite structures.  Infinite lazy lists are not handled
/// and would loop; they cannot be produced by the current THIR gate since
/// infinite data requires `match`/`Lambda` which are gated out.
pub fn force_deep(v: Value, ev: &eval::Evaluator<'_>) -> Result<Value, EvalError> {
    match v {
        Value::List(thunks) => {
            let forced: Result<Vec<_>, _> = thunks
                .iter()
                .map(|t| {
                    let inner = t.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok(thunk::Thunk::ready(deep))
                })
                .collect();
            Ok(Value::List(forced?.into()))
        }
        Value::Tuple(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|f| {
                    let inner = f.value.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok(value::TupleField {
                        name: f.name.clone(),
                        value: thunk::Thunk::ready(deep),
                    })
                })
                .collect();
            Ok(Value::Tuple(forced?.into()))
        }
        Value::Record(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|(name, t)| {
                    let inner = t.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok((name.clone(), thunk::Thunk::ready(deep)))
                })
                .collect();
            Ok(Value::Record(std::rc::Rc::new(forced?)))
        }
        other => Ok(other),
    }
}
