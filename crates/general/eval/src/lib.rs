//! Reference evaluators for Zutai general mode (`.zt`).
//!
//! ## Design
//! This crate is a *semantics oracle*: it REFUSES to evaluate any program that
//! is not fully type-checked by THIR. The pre-flight gates guarantee that no
//! `ThirExprKind::Error` node is reachable before evaluation begins, so a
//! returned `Value` is always a faithful representation of what the program's
//! final expression evaluates to.
//!
//! ## IR-agnostic core
//! The modules `value`, `thunk`, and `env` remain independent of any specific IR.
//! `eval_tlc` is the default evaluator for executable value programs. `eval` is
//! the THIR regression oracle and the runtime `Type`/reflection boundary.
//! `eval_file`/`eval_with_base`/`eval_path` are TLC-first defaults; `eval_thir_*`
//! are explicit oracle APIs; `eval_tlc_*` are strict TLC APIs.
//!
//! ## Note on resource management
//! Top-level evaluation builds a `letrec` environment where closures capture
//! the environment and the environment contains closures, creating `Rc` cycles.
//! This is an intentional per-run leak: the entire env graph is dropped at the
//! end of `eval_file`, which is acceptable for an interactive/batch tool.

pub mod env;
pub mod eval;
pub mod eval_tlc;
pub mod thunk;
pub mod value;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zutai_hir::BindingId;
use zutai_thir::{ImportKey, ThirDeclKind, ThirExprKind, ThirFile, TypeKind};

pub use value::Value;

use eval::{Evaluator, ModuleRegistry, RuntimeWitness};
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
    /// Program uses an effect form unsupported by the selected evaluator.
    #[error("cannot run: {0}")]
    EffectfulNotExecutable(String),
    /// Runtime reflection was asked to inspect a type outside the supported,
    /// serializable Phase 17 subset.
    #[error("reflection unsupported: {0}")]
    ReflectionUnsupported(String),
    /// An effect operation escaped all source handlers and the host boundary.
    #[error("runtime error: unhandled effect `{0}`")]
    UnhandledEffect(String),
    /// `resume` reached runtime without an operation continuation.
    #[error("runtime error: resume outside an operation handler")]
    ResumeOutsideHandler,
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
    /// A constraint method was called inside a polymorphic function but no
    /// witness could be resolved — the function was likely called indirectly,
    /// where witness injection via env is not yet supported.
    ///
    /// This is a deliberate limitation of the oracle, not a bug in the user's
    /// program. Full dictionary-passing is deferred to the TLC elaboration layer.
    #[error(
        "eval limitation: cannot resolve witness for method `{method}` in indirect call (dictionary-passing deferred to TLC)"
    )]
    UnresolvedWitness { method: String },
}

// ─── pre-flight gate ──────────────────────────────────────────────────────────

/// Check that `analysis` is fully typed and has no reachable error node.
///
/// Returns a reference to the completed `ThirFile` or an `EvalError`
/// describing exactly why evaluation is blocked.
pub fn check_well_typed(analysis: &zutai_semantic::Analysis) -> Result<&ThirFile, EvalError> {
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
            .map(|lt| lt.diagnostics.iter().map(format_thir_diagnostic).collect())
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

/// Check that `analysis` is safe for the legacy THIR evaluator.
pub fn check_runnable(analysis: &zutai_semantic::Analysis) -> Result<&ThirFile, EvalError> {
    let file = check_well_typed(analysis)?;
    if let Some(reason) = analysis.effectful_program() {
        return Err(EvalError::EffectfulNotExecutable(reason.to_string()));
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
        DeriveConstraintNotDerivable { constraint } => {
            format!("constraint `{constraint}` does not support derive")
        }
        DeriveComponentMissingWitness {
            constraint,
            component,
        } => {
            format!(
                "cannot derive `{constraint}` because component type `{component}` has no witness"
            )
        }
        DeriveUnsupportedMethod { constraint, method } => {
            format!(
                "cannot derive `{constraint}`: method `{method}` has no structural derivation recipe"
            )
        }
        ConflictingWitness { constraint, target } => {
            format!("conflicting witnesses for constraint `{constraint}` at type `{target}`")
        }
        RecursiveWitness { constraint } => {
            format!(
                "recursive witness for constraint `{constraint}`: target is one of its own type parameters"
            )
        }
        WitnessTargetKindMismatch { constraint, target } => {
            format!("witness target `{target}` has the wrong kind for constraint `{constraint}`")
        }
        UnsupportedMultiParamConstraint { name } => {
            format!("multi-param constraint `{name}` is not yet supported")
        }
        OverlappingRowField { name } => {
            format!("row tail introduces a field already declared: `{name}`")
        }
        RowAnnotationRequired => {
            "row-polymorphic inference is not principal here; add a type annotation".to_string()
        }
        EffectNotInRow { op } => {
            format!("effect `{op}` is not declared in the current effect row")
        }
        MalformedEffectOp { op, reason } => format!("malformed effect operation `{op}`: {reason}"),
        ResumeTypeMismatch { expected, found } => {
            format!("resume type mismatch: expected {expected}, found {found}")
        }
        HandlerClauseArityMismatch {
            op,
            expected,
            found,
        } => {
            format!("handler clause `{op}` expects {expected} parameter(s), found {found}")
        }
        MultipleResume { op } => {
            format!("handler clause `{op}` may resume more than once on one path")
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
            ThirExprKind::Perform { arg, .. } => stack.push(*arg),
            ThirExprKind::Resume { value } => stack.push(*value),
            ThirExprKind::Handle { expr, value, ops } => {
                stack.push(*expr);
                if let Some(value) = value {
                    stack.push(*value);
                }
                for op in ops {
                    stack.push(op.body);
                }
            }
            ThirExprKind::Sequence(items) => stack.extend(items.iter().copied()),
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
    eval_default_analysis(&analysis)
}

/// Evaluate a `.zt` source string, resolving `import`s relative to `base`.
pub fn eval_with_base(source: &str, base: Option<&Path>) -> Result<Value, EvalError> {
    let analysis =
        zutai_semantic::analyze_with_base(source, base, zutai_semantic::AnalysisOptions::default());
    eval_default_analysis(&analysis)
}

/// Evaluate a `.zt` source string with the explicit THIR regression oracle.
pub fn eval_thir_file(source: &str) -> Result<Value, EvalError> {
    eval_thir_with_base(source, None)
}

/// Evaluate a `.zt` file on disk with the explicit THIR regression oracle.
pub fn eval_thir_path(path: &Path) -> Result<Value, EvalError> {
    let analysis = zutai_semantic::analyze_path(path)
        .map_err(|err| EvalError::NotRunnable(vec![format!("cannot read {path:?}: {err}")]))?;
    eval_analysis(&analysis)
}

/// Evaluate a `.zt` source string with the explicit THIR regression oracle.
pub fn eval_thir_with_base(source: &str, base: Option<&Path>) -> Result<Value, EvalError> {
    let analysis =
        zutai_semantic::analyze_with_base(source, base, zutai_semantic::AnalysisOptions::default());
    eval_analysis(&analysis)
}

fn eval_default_analysis(analysis: &zutai_semantic::Analysis) -> Result<Value, EvalError> {
    if has_runtime_type_values(analysis) || analysis.reflection_builtin_program().is_some() {
        if has_tlc_effect_syntax_recursive(analysis) {
            return Err(EvalError::EffectfulNotExecutable(
                "program combines runtime Type values/reflection with source effect syntax; TLC does not yet represent Type values and the THIR oracle cannot execute source effects"
                    .to_string(),
            ));
        }
        return eval_analysis_allow_repointed_print(analysis);
    }

    eval_tlc_analysis(analysis)
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
    let mut module_paths: HashMap<PathBuf, ModuleId> = HashMap::new();
    let mut imports: HashMap<ImportKey, Value> = HashMap::new();
    eval_analysis_into(
        analysis,
        &mut registry,
        &mut module_paths,
        &mut imports,
        false,
    )?;
    let witnesses = runtime_witnesses(analysis, &module_paths);
    // The root module is the last entry in the registry.
    let root_id = ModuleId(registry.len() - 1);
    let root_file = Arc::clone(registry.last().unwrap());
    let evaluator = Evaluator::new(&root_file, &registry, root_id, &imports, &witnesses);
    let top = evaluator.build_top_env();
    let result = evaluator.eval(root_file.final_expr, &top)?;
    force_deep(result, &evaluator)
}

fn eval_analysis_allow_repointed_print(
    analysis: &zutai_semantic::Analysis,
) -> Result<Value, EvalError> {
    let mut registry: ModuleRegistry = Vec::new();
    let mut module_paths: HashMap<PathBuf, ModuleId> = HashMap::new();
    let mut imports: HashMap<ImportKey, Value> = HashMap::new();
    eval_analysis_into(
        analysis,
        &mut registry,
        &mut module_paths,
        &mut imports,
        true,
    )?;
    let witnesses = runtime_witnesses(analysis, &module_paths);
    let root_id = ModuleId(registry.len() - 1);
    let root_file = Arc::clone(registry.last().unwrap());
    let evaluator = Evaluator::new(&root_file, &registry, root_id, &imports, &witnesses);
    let top = evaluator.build_top_env();
    let result = evaluator.eval(root_file.final_expr, &top)?;
    force_deep(result, &evaluator)
}

fn has_runtime_type_values(analysis: &zutai_semantic::Analysis) -> bool {
    let root_has_type_values = analysis
        .thir
        .as_ref()
        .and_then(|thir| thir.file.as_ref())
        .is_some_and(|file| {
            file.expr_arena
                .iter()
                .any(|(_, expr)| matches!(expr.kind, ThirExprKind::TypeValue(_)))
        });
    root_has_type_values
        || analysis
            .import_modules
            .values()
            .any(|module| has_runtime_type_values(module.as_ref()))
}

fn has_tlc_effect_syntax(analysis: &zutai_semantic::Analysis) -> bool {
    analysis
        .thir
        .as_ref()
        .and_then(|thir| thir.file.as_ref())
        .is_some_and(|file| {
            file.expr_arena.iter().any(|(_, expr)| {
                matches!(
                    expr.kind,
                    ThirExprKind::Perform { .. }
                        | ThirExprKind::Handle { .. }
                        | ThirExprKind::Resume { .. }
                )
            })
        })
}

fn has_tlc_effect_syntax_recursive(analysis: &zutai_semantic::Analysis) -> bool {
    has_tlc_effect_syntax(analysis)
        || analysis
            .import_modules
            .values()
            .any(|module| has_tlc_effect_syntax_recursive(module.as_ref()))
}

/// Recursive helper: populate `registry` and `imports` depth-first, then
/// register the current module and return its `ModuleId`.
fn eval_analysis_into(
    analysis: &zutai_semantic::Analysis,
    registry: &mut ModuleRegistry,
    module_paths: &mut HashMap<PathBuf, ModuleId>,
    imports: &mut HashMap<ImportKey, Value>,
    allow_repointed_print: bool,
) -> Result<ModuleId, EvalError> {
    let file = if allow_repointed_print {
        if has_tlc_effect_syntax(analysis) {
            return Err(EvalError::EffectfulNotExecutable(
                "source effect syntax requires the TLC effect evaluator".to_string(),
            ));
        }
        check_well_typed(analysis)?
    } else {
        check_runnable(analysis)?
    };

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
            let dep_id = eval_analysis_into(
                module,
                registry,
                module_paths,
                imports,
                allow_repointed_print,
            )?;
            // Now evaluate the dependency's final expression in its own module.
            let dep_file = Arc::clone(&registry[dep_id.0]);
            let witnesses = runtime_witnesses(module, module_paths);
            let dep_ev = Evaluator::new(&dep_file, registry, dep_id, imports, &witnesses);
            let dep_top = dep_ev.build_top_env();
            let dep_result = dep_ev.eval(dep_file.final_expr, &dep_top)?;
            let dep_value = force_deep(dep_result, &dep_ev)?;
            imports.insert(key.clone(), dep_value);
        }
    }

    // Register this module and return its id.
    let id = ModuleId(registry.len());
    registry.push(Arc::new(file.clone()));
    for witness in &analysis.witness_exports {
        module_paths.entry(witness.origin.clone()).or_insert(id);
    }
    Ok(id)
}

fn runtime_witnesses(
    analysis: &zutai_semantic::Analysis,
    module_paths: &HashMap<PathBuf, ModuleId>,
) -> Vec<RuntimeWitness> {
    analysis
        .witness_exports
        .iter()
        .filter_map(|witness| {
            let module = module_paths.get(&witness.origin).copied()?;
            Some(RuntimeWitness {
                module,
                constraint: witness.constraint.clone(),
                target_key: witness.target_key.clone(),
            })
        })
        .collect()
}

/// Evaluate a `.zt` source string using the TLC eager evaluator.
///
/// Runs the full pipeline through TLC elaboration, then evaluates the TLC
/// module's final expression with `eval_tlc::TlcEvaluator`. This is the
/// compiler-path parity oracle for dictionary-passing, witnessed operators, and
/// other TLC-only elaboration behavior.
pub fn eval_tlc_file(source: &str) -> Result<Value, EvalError> {
    eval_tlc_with_base(source, None)
}

/// Evaluate a `.zt` file on disk with the strict TLC evaluator.
pub fn eval_tlc_path(path: &Path) -> Result<Value, EvalError> {
    let analysis = zutai_semantic::analyze_path(path)
        .map_err(|err| EvalError::NotRunnable(vec![format!("cannot read {path:?}: {err}")]))?;
    eval_tlc_analysis(&analysis)
}

/// Evaluate a `.zt` source string with the strict TLC evaluator.
pub fn eval_tlc_with_base(source: &str, base: Option<&Path>) -> Result<Value, EvalError> {
    let analysis =
        zutai_semantic::analyze_with_base(source, base, zutai_semantic::AnalysisOptions::default());
    eval_tlc_analysis(&analysis)
}

fn seed_tlc_prelude(thir_file: &ThirFile, top: env::Env) -> env::Env {
    // TLC carries no binding kinds, so resolve prelude binding ids from the THIR
    // binding-name table. HIR seeds builtins first, so the lowest-id match is
    // the prelude one.
    for &name in zutai_hir::BUILTIN_VALUE_NAMES {
        if let Some(builtin) = value::BuiltinFn::from_name(name)
            && let Some(index) = thir_file.binding_names.iter().position(|n| n == name)
        {
            top.insert(
                BindingId(index as u32),
                thunk::Thunk::ready(Value::Builtin(builtin)),
            );
        }
    }
    top
}

fn completed_tlc_inputs(
    analysis: &zutai_semantic::Analysis,
) -> Result<(&ThirFile, &zutai_tlc::TlcModule), EvalError> {
    let thir_file = check_well_typed(analysis)?;
    if has_runtime_type_values(analysis) {
        return Err(EvalError::ReflectionUnsupported(
            "runtime Type values are not represented in the TLC evaluator yet".to_string(),
        ));
    }
    let module = analysis.tlc.as_ref().ok_or(EvalError::Internal(
        "semantic analysis did not produce TLC for complete THIR",
    ))?;
    Ok((thir_file, module))
}

fn eval_tlc_analysis(analysis: &zutai_semantic::Analysis) -> Result<Value, EvalError> {
    let mut registry = Vec::new();
    let mut imports = HashMap::new();
    let mut operator_witnesses = HashMap::new();
    let root_id = eval_tlc_analysis_into(
        analysis,
        &mut registry,
        &mut imports,
        &mut operator_witnesses,
    )?;

    let (thir_file, root_module) = completed_tlc_inputs(analysis)?;
    let ev = eval_tlc::TlcEvaluator::new_in_registry_with_operator_witnesses(
        registry.as_slice(),
        root_id,
        &imports,
        &operator_witnesses,
    )?;
    let top = seed_tlc_prelude(thir_file, env::Env::empty());
    let top = ev.build_top_env_from(top)?;
    let final_id = root_module
        .final_expr
        .ok_or(EvalError::Internal("TLC module has no final expression"))?;
    let result = ev.eval_expr(final_id, &top)?;
    eval_tlc::tlc_force_deep(result, &ev)
}

fn eval_tlc_analysis_into<'a>(
    analysis: &'a zutai_semantic::Analysis,
    registry: &mut eval_tlc::TlcModuleRegistry<'a>,
    imports: &mut HashMap<ImportKey, Value>,
    operator_witnesses: &mut HashMap<(String, String), Value>,
) -> Result<ModuleId, EvalError> {
    let (_thir_file, module) = completed_tlc_inputs(analysis)?;

    for (key, value) in &analysis.import_values {
        imports
            .entry(key.clone())
            .or_insert_with(|| Value::from_immediate(value));
    }

    for (key, imported_analysis) in &analysis.import_modules {
        if imports.contains_key(key) {
            continue;
        }
        let dep_id = eval_tlc_analysis_into(
            imported_analysis.as_ref(),
            registry,
            imports,
            operator_witnesses,
        )?;
        let (dep_thir_file, dep_module) = completed_tlc_inputs(imported_analysis.as_ref())?;
        let dep_ev = eval_tlc::TlcEvaluator::new_in_registry(registry.as_slice(), dep_id, imports)?;
        let dep_top = seed_tlc_prelude(dep_thir_file, env::Env::empty());
        let dep_top = dep_ev.build_top_env_from(dep_top)?;
        let final_id = dep_module
            .final_expr
            .ok_or(EvalError::Internal("TLC module has no final expression"))?;
        let dep_result = dep_ev.eval_expr(final_id, &dep_top)?;
        collect_tlc_operator_witnesses(dep_thir_file, &dep_ev, &dep_top, operator_witnesses)?;
        let dep_value = eval_tlc::tlc_force_deep(dep_result, &dep_ev)?;
        imports.insert(key.clone(), dep_value);
    }

    let id = ModuleId(registry.len());
    registry.push(module);
    Ok(id)
}

fn collect_tlc_operator_witnesses(
    thir_file: &ThirFile,
    ev: &eval_tlc::TlcEvaluator<'_>,
    top: &env::Env,
    out: &mut HashMap<(String, String), Value>,
) -> Result<(), EvalError> {
    for &decl_id in &thir_file.decls {
        let decl = &thir_file.decl_arena[decl_id];
        let ThirDeclKind::Witness { target, fields, .. } = &decl.kind else {
            continue;
        };
        let Some(target_key) = thir_runtime_target_key(thir_file, *target) else {
            continue;
        };
        let dict = top.lookup(decl.binding)?.force_tlc(ev)?;
        let Value::Record(dict_fields) = dict else {
            return Err(EvalError::TypeMismatch {
                expected: "Record",
                found: "non-record witness dictionary",
            });
        };
        for field in fields {
            let Some((_, thunk)) = dict_fields
                .iter()
                .find(|(name, _)| name.as_ref() == field.name.as_str())
            else {
                continue;
            };
            out.insert(
                (field.name.clone(), target_key.clone()),
                thunk.force_tlc(ev)?,
            );
        }
    }
    Ok(())
}

fn thir_runtime_target_key(thir_file: &ThirFile, target: zutai_thir::TypeId) -> Option<String> {
    match &thir_file.type_arena[target.0 as usize].kind {
        TypeKind::Bool | TypeKind::True | TypeKind::False => Some("Bool".to_string()),
        TypeKind::Text => Some("Text".to_string()),
        TypeKind::Int => Some("Int".to_string()),
        TypeKind::Float => Some("Float".to_string()),
        TypeKind::Atom(name) => Some(format!("#{name}")),
        _ => None,
    }
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
    let witnesses = Vec::new();
    let evaluator = Evaluator::new(file, &registry, ModuleId(0), imports, &witnesses);
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
        Value::TaggedValue { tag, payload } => {
            let forced: Result<Vec<_>, _> = payload
                .iter()
                .map(|(name, t)| {
                    let inner = t.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok((name.clone(), thunk::Thunk::ready(deep)))
                })
                .collect();
            Ok(Value::TaggedValue {
                tag,
                payload: std::rc::Rc::new(forced?),
            })
        }
        other => Ok(other),
    }
}
