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
use std::path::Path;
use std::sync::Arc;

use zutai_hir::BindingId;
use zutai_thir::{ImportKey, ThirExprKind, ThirFile};

pub use value::Value;

use eval::{Evaluator, ModuleRegistry};
use value::ModuleId;

// ─── errors ───────────────────────────────────────────────────────────────────

fn indent_msgs(msgs: &[String]) -> String {
    msgs.iter().map(|m| format!("\n  {m}")).collect()
}

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum EvalError {
    /// Source program has parse or HIR errors.
    #[error("program has errors and cannot be evaluated:{}", indent_msgs(.0))]
    NotRunnable(Vec<String>),
    /// THIR type checking failed or is incomplete.
    #[error("type checking failed:{}", indent_msgs(.0))]
    TypeCheckFailed(Vec<String>),
    /// A `ThirExprKind::Error` node was reachable in a nominally-complete THIR.
    #[error("internal: reachable Error node in type-checked THIR")]
    ErrorNodeReachable,
    /// Runtime black-hole: a non-productive recursive binding was forced.
    #[error("runtime error: non-productive recursive definition (black hole)")]
    BlackHole,
    /// Division by zero in integer division.
    #[error("runtime error: integer division by zero")]
    DivByZero,
    /// Integer overflow.
    #[error("runtime error: integer overflow in `{0}`")]
    IntOverflow(&'static str),
    /// No clause of a function matched the arguments.
    #[error("runtime error: no matching clause (non-exhaustive pattern match)")]
    NoMatchingClause,
    /// An unbound `BindingId` was looked up.
    ///
    /// Unreachable in fully-evaluated well-typed code **except** for constraint
    /// method calls with no matching witness in scope: dispatch is attempted at
    /// the `Apply` node using the instantiation's type key, but when no witness
    /// field matches the interpreter refuses rather than guessing a value.
    #[error("internal: unbound binding {0:?}")]
    UnboundBinding(BindingId),
    /// Runtime type mismatch (unreachable in well-typed code).
    #[error("internal: type mismatch — expected {expected}, found {found}")]
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    /// Internal invariant violated (always a bug in the interpreter).
    #[error("internal error: {0}")]
    Internal(&'static str),
}

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
        TypeConstructorArityMismatch {
            name,
            expected,
            found,
        } => {
            format!("type constructor `{name}` expects {expected} argument(s), found {found}")
        }
        TypeLevelEvalLimitExceeded => {
            "type-level computation exceeded evaluation limit".to_string()
        }
        WitnessFieldTypeMismatch {
            name,
            expected,
            found,
        } => {
            format!("witness field `{name}` has type {found}, expected {expected}")
        }
        MissingWitnessField { name } => format!("missing witness field `{name}`"),
        UnknownWitnessField { name } => format!("unknown witness field `{name}`"),
        ConflictingWitness { constraint, target } => {
            format!("conflicting witnesses for constraint `{constraint}` at type `{target}`")
        }
    }
}

/// Walk all reachable expressions in `file` and check for `Error` nodes.
fn has_reachable_error(file: &ThirFile) -> bool {
    // Check the final expression and all top-level declaration expressions.
    let mut to_visit: Vec<zutai_thir::ThirExprId> = vec![file.final_expr];
    for (_, decl) in file.decl_arena.iter() {
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
            // Constraint decls have no expr nodes to walk.
            zutai_thir::ThirDeclKind::Constraint { .. } => {}
            // Witness field values must be error-walked: a malformed field should
            // refuse evaluation just like a malformed top-level binding.
            zutai_thir::ThirDeclKind::Witness { fields, .. } => {
                for f in fields {
                    to_visit.push(f.value);
                }
            }
        }
    }

    let mut visited = std::collections::HashSet::new();
    let mut stack = to_visit;
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let expr = &file.expr_arena[id];
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
            ThirExprKind::TaggedValue { payload, .. } => stack.push(*payload),
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
    eval_with_base(source, None)
}

/// Evaluate a `.zt` file on disk, resolving imports relative to its directory.
pub fn eval_path(path: &Path) -> Result<Value, EvalError> {
    let analysis = zutai_semantic::analyze_path(path)
        .map_err(|err| EvalError::NotRunnable(vec![format!("cannot read {path:?}: {err}")]))?;
    eval_analysis(&analysis)
}

/// Evaluate a `.zt` source string, resolving `import`s relative to `base`.
pub fn eval_with_base(source: &str, base: Option<&Path>) -> Result<Value, EvalError> {
    let analysis =
        zutai_semantic::analyze_with_base(source, base, zutai_semantic::AnalysisOptions::default());
    eval_analysis(&analysis)
}

/// Gate-check an analyzed module and evaluate it, recursively evaluating its
/// imports first.  `.zti` imports become data values; `.zt` imports are
/// evaluated depth-first.  The import graph is acyclic by the time evaluation
/// runs (the analyzer refuses cyclic imports), so this terminates.
///
/// Each `.zt` dependency is assigned a `ModuleId` in the registry so closures
/// stamped with that id can re-enter the correct file's arenas when forced or
/// applied across a module boundary.
fn eval_analysis(analysis: &zutai_semantic::Analysis) -> Result<Value, EvalError> {
    let mut registry: ModuleRegistry = Vec::new();
    let mut imports: HashMap<ImportKey, Value> = HashMap::new();
    eval_analysis_into(analysis, &mut registry, &mut imports)?;
    // The root module is the last entry in the registry.
    let root_id = ModuleId(registry.len() - 1);
    let root_file = Arc::clone(registry.last().unwrap());
    let evaluator = Evaluator::new(&root_file, &registry, root_id, &imports);
    let top = evaluator.build_top_env();
    let result = evaluator.eval(root_file.final_expr, &top)?;
    force_deep(result, &evaluator)
}

/// Recursive helper: populate `registry` and `imports` depth-first, then
/// register the current module and return its `ModuleId`.
fn eval_analysis_into(
    analysis: &zutai_semantic::Analysis,
    registry: &mut ModuleRegistry,
    imports: &mut HashMap<ImportKey, Value>,
) -> Result<ModuleId, EvalError> {
    let file = check_runnable(analysis)?;

    // Populate `.zti` import values.
    for (key, value) in &analysis.import_values {
        if !imports.contains_key(key) {
            imports.insert(key.clone(), Value::from_immediate(value));
        }
    }

    // Recursively register imported `.zt` modules depth-first.
    for (key, module) in &analysis.import_modules {
        if !imports.contains_key(key) {
            // Recurse: register the dependency, building its top-env stamped
            // with its own ModuleId so closures carry the correct home.
            let dep_id = eval_analysis_into(module, registry, imports)?;
            // Now evaluate the dependency's final expression in its own module.
            let dep_file = Arc::clone(&registry[dep_id.0]);
            let dep_ev = Evaluator::new(&dep_file, registry, dep_id, imports);
            let dep_top = dep_ev.build_top_env();
            let dep_result = dep_ev.eval(dep_file.final_expr, &dep_top)?;
            let dep_value = force_deep(dep_result, &dep_ev)?;
            imports.insert(key.clone(), dep_value);
        }
    }

    // Register this module and return its id.
    let id = ModuleId(registry.len());
    registry.push(Arc::new(file.clone()));
    Ok(id)
}

/// Evaluate a pre-analyzed, gate-checked `ThirFile` with no imports.
pub fn eval_thir(file: &ThirFile) -> Result<Value, EvalError> {
    eval_thir_with_imports(file, &HashMap::new())
}

/// Evaluate a pre-analyzed, gate-checked `ThirFile` with resolved import values.
///
/// For single-file evaluation (no cross-module closures).  The file is
/// registered as module 0 in a single-entry registry.
pub fn eval_thir_with_imports(
    file: &ThirFile,
    imports: &HashMap<ImportKey, Value>,
) -> Result<Value, EvalError> {
    let registry: ModuleRegistry = vec![Arc::new(file.clone())];
    let evaluator = Evaluator::new(file, &registry, ModuleId(0), imports);
    let top = evaluator.build_top_env();
    let result = evaluator.eval(file.final_expr, &top)?;
    force_deep(result, &evaluator)
}

/// Recursively force all lazy thunks inside a value so it can be displayed.
///
/// Only descends into finite structures.  Recursive lambdas can produce
/// infinite data (e.g. an infinite list); `force_deep` will loop on them.
/// This is acceptable for the reference interpreter — a non-terminating
/// program already diverges at eval time before reaching this function.
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
