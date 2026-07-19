use super::*;
use crate::tlc_entry::eval_tlc_analysis;

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

/// Evaluate a `.zt` source string and serialize the final value to JSON.
pub fn eval_file_to_json(source: &str) -> Result<serde_json::Value, EvalError> {
    eval_with_base(source, None)?.to_json()
}

/// Evaluate a `.zt` file on disk and serialize the final value to JSON,
/// resolving `import`s relative to the file's directory.
pub fn eval_path_to_json(path: impl AsRef<Path>) -> Result<serde_json::Value, EvalError> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path)
        .map_err(|_| EvalError::Internal("failed to read source for JSON evaluation"))?;
    let base = path.parent();
    eval_with_base(&source, base)?.to_json()
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

pub(super) fn eval_default_analysis(
    analysis: &zutai_semantic::Analysis,
) -> Result<Value, EvalError> {
    if root_has_runtime_type_values(analysis) || analysis.reflection_builtin_program().is_some() {
        // Concrete `schema` applications fold to data during THIR→TLC
        // elaboration. When every reflection use folded (here and in imports),
        // the `Type`-typed THIR subexpressions never reach TLC and the module
        // runs on the TLC path — including combined with source effects.
        if analysis.tlc_reflection_folded() && analysis.reflection_builtin_program().is_none() {
            return eval_tlc_analysis(analysis);
        }
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
    let mut module_paths: FxHashMap<PathBuf, ModuleId> = FxHashMap::default();
    let mut imports: FxHashMap<ImportKey, Value> = FxHashMap::default();
    let caches = EvalCaches::default();
    eval_analysis_into(
        analysis,
        &mut registry,
        &mut module_paths,
        &mut imports,
        false,
        &caches,
    )?;
    let witnesses = runtime_witnesses(analysis, &module_paths);
    // The root module is the last entry in the registry.
    let root_id = ModuleId(registry.len() - 1);
    let root_file = Arc::clone(registry.last().unwrap());
    let evaluator = Evaluator::new(
        &root_file, &registry, root_id, &imports, &witnesses, &caches,
    );
    let top = evaluator.build_top_env();
    let result = evaluator.eval(root_file.final_expr, &top)?;
    force_deep(result, &evaluator)
}

fn eval_analysis_allow_repointed_print(
    analysis: &zutai_semantic::Analysis,
) -> Result<Value, EvalError> {
    let mut registry: ModuleRegistry = Vec::new();
    let mut module_paths: FxHashMap<PathBuf, ModuleId> = FxHashMap::default();
    let mut imports: FxHashMap<ImportKey, Value> = FxHashMap::default();
    let caches = EvalCaches::default();
    eval_analysis_into(
        analysis,
        &mut registry,
        &mut module_paths,
        &mut imports,
        true,
        &caches,
    )?;
    let witnesses = runtime_witnesses(analysis, &module_paths);
    let root_id = ModuleId(registry.len() - 1);
    let root_file = Arc::clone(registry.last().unwrap());
    let evaluator = Evaluator::new(
        &root_file, &registry, root_id, &imports, &witnesses, &caches,
    );
    let top = evaluator.build_top_env();
    let result = evaluator.eval(root_file.final_expr, &top)?;
    force_deep(result, &evaluator)
}

pub(super) fn root_has_runtime_type_values(analysis: &zutai_semantic::Analysis) -> bool {
    analysis
        .thir
        .as_ref()
        .and_then(|thir| thir.file.as_ref())
        .is_some_and(|file| {
            file.expr_arena.iter().any(|(_, expr)| {
                matches!(
                    file.type_arena[expr.ty.0 as usize].kind,
                    zutai_thir::TypeKind::Type(_)
                )
            })
        })
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
    module_paths: &mut FxHashMap<PathBuf, ModuleId>,
    imports: &mut FxHashMap<ImportKey, Value>,
    allow_repointed_print: bool,
    caches: &EvalCaches,
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
                caches,
            )?;
            // Now evaluate the dependency's final expression in its own module.
            let dep_file = Arc::clone(&registry[dep_id.0]);
            let witnesses = runtime_witnesses(module, module_paths);
            let dep_ev = Evaluator::new(&dep_file, registry, dep_id, imports, &witnesses, caches);
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
    module_paths: &FxHashMap<PathBuf, ModuleId>,
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
/// Evaluate a pre-analyzed, gate-checked `ThirFile` with no imports.
pub fn eval_thir(file: &ThirFile) -> Result<Value, EvalError> {
    eval_thir_with_imports(file, &FxHashMap::default())
}

/// Evaluate a pre-analyzed, gate-checked `ThirFile` with resolved import values.
///
/// For single-file evaluation (no cross-module closures).  The file is
/// registered as module 0 in a single-entry registry.
pub fn eval_thir_with_imports(
    file: &ThirFile,
    imports: &FxHashMap<ImportKey, Value>,
) -> Result<Value, EvalError> {
    let registry: ModuleRegistry = vec![Arc::new(file.clone())];
    let witnesses = Vec::new();
    let caches = EvalCaches::default();
    let evaluator = Evaluator::new(file, &registry, ModuleId(0), imports, &witnesses, &caches);
    let top = evaluator.build_top_env();
    let result = evaluator.eval(file.final_expr, &top)?;
    force_deep(result, &evaluator)
}
