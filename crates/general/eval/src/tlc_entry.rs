use super::*;
use crate::analysis_eval::has_runtime_type_values;

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

pub(super) fn eval_tlc_analysis(analysis: &zutai_semantic::Analysis) -> Result<Value, EvalError> {
    let mut registry = Vec::new();
    let mut imports = FxHashMap::default();
    let mut operator_witnesses = FxHashMap::default();
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
    imports: &mut FxHashMap<ImportKey, Value>,
    operator_witnesses: &mut FxHashMap<(String, String), Value>,
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
    out: &mut FxHashMap<(String, String), Value>,
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
    thir_runtime_target_key_inner(thir_file, target, &mut Vec::new())
}

fn thir_runtime_target_key_inner(
    thir_file: &ThirFile,
    target: zutai_thir::TypeId,
    seen: &mut Vec<BindingId>,
) -> Option<String> {
    match &thir_file.type_arena[target.0 as usize].kind {
        TypeKind::Bool | TypeKind::True | TypeKind::False => Some("Bool".to_string()),
        TypeKind::Text => Some("Text".to_string()),
        TypeKind::Int => Some("Int".to_string()),
        TypeKind::Float => Some("Float".to_string()),
        TypeKind::FixedNum(fw) => Some(fw.name().to_string()),
        TypeKind::Posit(spec) => Some(spec.type_name()),
        TypeKind::Atom(name) => Some(format!("#{name}")),
        TypeKind::List(inner) => Some(format!(
            "[{}]",
            thir_runtime_target_key_inner(thir_file, *inner, seen)?
        )),
        TypeKind::Optional(inner) => Some(format!(
            "{}?",
            thir_runtime_target_key_inner(thir_file, *inner, seen)?
        )),
        TypeKind::Maybe(inner) => Some(format!(
            "Maybe[{}]",
            thir_runtime_target_key_inner(thir_file, *inner, seen)?
        )),
        TypeKind::Patch { target, .. } => thir_patch_target_key(thir_file, *target, seen),
        TypeKind::Record(fields, tail) => {
            thir_record_target_key(thir_file, fields, *tail, false, seen)
        }
        TypeKind::Union(variants, tail) => {
            let parts: Vec<String> = variants
                .iter()
                .map(|variant| match variant.payload {
                    Some(payload) => Some(format!(
                        "{}({})",
                        variant.name,
                        thir_runtime_target_key_inner(thir_file, payload, seen)?
                    )),
                    None => Some(variant.name.clone()),
                })
                .collect::<Option<_>>()?;
            Some(format!("<{}{}>", parts.join("|"), thir_row_tail_key(*tail)))
        }
        TypeKind::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    TypeTupleItem::Named { name, ty, .. } => Some(format!(
                        "{}:{}",
                        name,
                        thir_runtime_target_key_inner(thir_file, *ty, seen)?
                    )),
                    TypeTupleItem::Positional(ty) => {
                        thir_runtime_target_key_inner(thir_file, *ty, seen)
                    }
                })
                .collect::<Option<_>>()?;
            Some(format!("({})", parts.join(",")))
        }
        TypeKind::Function { from, to } => Some(format!(
            "({}->{})",
            thir_runtime_target_key_inner(thir_file, *from, seen)?,
            thir_runtime_target_key_inner(thir_file, *to, seen)?
        )),
        TypeKind::Effect { base, .. } => thir_runtime_target_key_inner(thir_file, *base, seen),
        TypeKind::Never => Some("Never".to_string()),
        TypeKind::Alias(binding) => {
            if seen.contains(binding) {
                return None;
            }
            seen.push(*binding);
            let body = thir_type_alias_body(thir_file, *binding)?;
            let key = thir_runtime_target_key_inner(thir_file, body, seen);
            seen.pop();
            key
        }
        _ => None,
    }
}

fn thir_record_target_key(
    thir_file: &ThirFile,
    fields: &[zutai_thir::TypeRecordField],
    tail: RowTail,
    force_optional: bool,
    seen: &mut Vec<BindingId>,
) -> Option<String> {
    let mut parts: Vec<String> = fields
        .iter()
        .map(|field| {
            let key = thir_runtime_target_key_inner(thir_file, field.ty, seen)?;
            let marker = if force_optional || field.optional {
                "?:"
            } else {
                ":"
            };
            Some(format!("{}{}{}", field.name, marker, key))
        })
        .collect::<Option<_>>()?;
    parts.sort();
    Some(format!(
        "{{{}{}}}",
        parts.join(","),
        thir_row_tail_key(tail)
    ))
}

fn thir_patch_target_key(
    thir_file: &ThirFile,
    target: zutai_thir::TypeId,
    seen: &mut Vec<BindingId>,
) -> Option<String> {
    match &thir_file.type_arena[target.0 as usize].kind {
        TypeKind::Record(fields, tail) => {
            thir_record_target_key(thir_file, fields, *tail, true, seen)
        }
        TypeKind::Alias(binding) => {
            if seen.contains(binding) {
                return None;
            }
            seen.push(*binding);
            let body = thir_type_alias_body(thir_file, *binding)?;
            let key = thir_patch_target_key(thir_file, body, seen);
            seen.pop();
            key
        }
        _ => None,
    }
}

fn thir_type_alias_body(thir_file: &ThirFile, binding: BindingId) -> Option<zutai_thir::TypeId> {
    thir_file.decls.iter().find_map(|&decl_id| {
        let decl = &thir_file.decl_arena[decl_id];
        match &decl.kind {
            ThirDeclKind::TypeAlias { params, ty }
                if decl.binding == binding && params.is_empty() =>
            {
                Some(*ty)
            }
            _ => None,
        }
    })
}

fn thir_row_tail_key(tail: RowTail) -> &'static str {
    match tail {
        RowTail::Closed => "",
        RowTail::Open => ";...",
        RowTail::Param(_) | RowTail::Infer(_) => ";...$",
    }
}
