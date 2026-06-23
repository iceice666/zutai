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

/// Witness tables threaded through dependency evaluation: concrete operator
/// methods (`(method, key) -> value`), concrete dictionaries
/// (`(constraint, key) -> Record`, for component resolution), and imported
/// conditional witnesses (instantiated on demand at the root's call sites).
#[derive(Default)]
struct WitnessTables {
    operators: FxHashMap<(String, String), Value>,
    concrete_dicts: FxHashMap<(String, String), Value>,
    conditionals: Vec<ConditionalRuntimeWitness>,
}

pub(super) fn eval_tlc_analysis(analysis: &zutai_semantic::Analysis) -> Result<Value, EvalError> {
    let mut registry = Vec::new();
    let mut imports = FxHashMap::default();
    let mut tables = WitnessTables::default();
    let root_id = eval_tlc_analysis_into(analysis, &mut registry, &mut imports, &mut tables)?;

    let (thir_file, root_module) = completed_tlc_inputs(analysis)?;

    // Instantiate imported conditional witnesses for the root's concrete call
    // sites: every dispatch key the root needs that a parametric witness covers
    // becomes a concrete dictionary whose methods join `operators`, so the
    // unchanged `imported_method` dispatch finds them by `(method, key)`.
    if !tables.conditionals.is_empty() {
        let mat_ev =
            eval_tlc::TlcEvaluator::new_in_registry(registry.as_slice(), root_id, &imports)?;
        let needed: Vec<String> = root_module.dict_dispatch_keys.values().cloned().collect();
        let WitnessTables {
            operators,
            concrete_dicts,
            conditionals,
        } = &mut tables;
        let conds: &[ConditionalRuntimeWitness] = conditionals;
        for cw in conds {
            for key in &needed {
                let Some(Value::Record(fields)) = materialize_conditional_dict(
                    &mat_ev,
                    &cw.constraint,
                    key,
                    conds,
                    concrete_dicts,
                    0,
                ) else {
                    continue;
                };
                for (name, thunk) in fields.iter() {
                    let value = thunk.force_tlc(&mat_ev)?;
                    operators
                        .entry((name.to_string(), key.clone()))
                        .or_insert(value);
                }
            }
        }
    }

    let ev = eval_tlc::TlcEvaluator::new_in_registry_with_operator_witnesses(
        registry.as_slice(),
        root_id,
        &imports,
        &tables.operators,
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
    tables: &mut WitnessTables,
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
        let dep_id = eval_tlc_analysis_into(imported_analysis.as_ref(), registry, imports, tables)?;
        let (dep_thir_file, dep_module) = completed_tlc_inputs(imported_analysis.as_ref())?;
        let dep_ev = eval_tlc::TlcEvaluator::new_in_registry(registry.as_slice(), dep_id, imports)?;
        let dep_top = seed_tlc_prelude(dep_thir_file, env::Env::empty());
        let dep_top = dep_ev.build_top_env_from(dep_top)?;
        let final_id = dep_module
            .final_expr
            .ok_or(EvalError::Internal("TLC module has no final expression"))?;
        let dep_result = dep_ev.eval_expr(final_id, &dep_top)?;
        collect_tlc_operator_witnesses(
            dep_thir_file,
            &dep_ev,
            &dep_top,
            &mut tables.operators,
            &mut tables.concrete_dicts,
            &mut tables.conditionals,
        )?;
        let dep_value = eval_tlc::tlc_force_deep(dep_result, &dep_ev)?;
        imports.insert(key.clone(), dep_value);
    }

    let id = ModuleId(registry.len());
    registry.push(module);
    Ok(id)
}

/// An imported parametric (conditional) witness, captured for on-demand
/// instantiation at the importer's concrete call sites.
struct ConditionalRuntimeWitness {
    constraint: String,
    pattern: zutai_thir::WitnessPattern,
    /// Per-parameter component-constraint names, parallel to the pattern's holes.
    param_bounds: Vec<Vec<String>>,
    /// The witness's dictionary function value (`\d0. \d1. Record` after type
    /// erasure), applied to recursively-resolved component dicts at a call site.
    func: Value,
}

fn collect_tlc_operator_witnesses(
    thir_file: &ThirFile,
    ev: &eval_tlc::TlcEvaluator<'_>,
    top: &env::Env,
    out: &mut FxHashMap<(String, String), Value>,
    concrete_dicts: &mut FxHashMap<(String, String), Value>,
    conditionals: &mut Vec<ConditionalRuntimeWitness>,
) -> Result<(), EvalError> {
    for &decl_id in &thir_file.decls {
        let decl = &thir_file.decl_arena[decl_id];
        let ThirDeclKind::Witness {
            constraint,
            target,
            params,
            param_bounds,
            fields,
            ..
        } = &decl.kind
        else {
            continue;
        };
        let constraint_name = constraint
            .and_then(|b| thir_file.binding_names.get(b.0 as usize))
            .cloned();
        // A parametric witness (`Eq @(List A)`): capture its structural matcher,
        // component bounds, and dictionary function for on-demand instantiation.
        if !params.is_empty() {
            let (Some(constraint_name), Some(pattern)) = (
                constraint_name,
                zutai_thir::export_witness_pattern(thir_file, *target, params),
            ) else {
                continue;
            };
            let bound_names = param_bounds
                .iter()
                .map(|bounds| {
                    bounds
                        .iter()
                        .filter_map(|b| thir_file.binding_names.get(b.0 as usize).cloned())
                        .collect()
                })
                .collect();
            let func = top.lookup(decl.binding)?.force_tlc(ev)?;
            conditionals.push(ConditionalRuntimeWitness {
                constraint: constraint_name,
                pattern,
                param_bounds: bound_names,
                func,
            });
            continue;
        }
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
        if let Some(constraint_name) = &constraint_name {
            concrete_dicts.insert(
                (constraint_name.clone(), target_key.clone()),
                Value::Record(dict_fields.clone()),
            );
        }
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

/// Instantiate an imported conditional witness dictionary for a concrete type
/// `key` under `constraint`, memoizing into `concrete_dicts`. Matches the
/// witness pattern against `key`, recursively materializes each component dict,
/// and applies the witness function to them. Returns `None` when no conditional
/// witness matches or a required component cannot be resolved.
fn materialize_conditional_dict(
    ev: &eval_tlc::TlcEvaluator<'_>,
    constraint: &str,
    key: &str,
    conditionals: &[ConditionalRuntimeWitness],
    concrete_dicts: &mut FxHashMap<(String, String), Value>,
    depth: u32,
) -> Option<Value> {
    if depth > 64 {
        return None;
    }
    if let Some(dict) = concrete_dicts.get(&(constraint.to_string(), key.to_string())) {
        return Some(dict.clone());
    }
    for cw in conditionals {
        if cw.constraint != constraint {
            continue;
        }
        let Some(sub_keys) = match_pattern_key(&cw.pattern, key, cw.param_bounds.len()) else {
            continue;
        };
        let mut cur = cw.func.clone();
        let mut ok = true;
        'param: for (i, bounds) in cw.param_bounds.iter().enumerate() {
            for bound in bounds {
                let Some(component) = materialize_conditional_dict(
                    ev,
                    bound,
                    &sub_keys[i],
                    conditionals,
                    concrete_dicts,
                    depth + 1,
                ) else {
                    ok = false;
                    break 'param;
                };
                let Ok(applied) = ev.apply_to_value(cur.clone(), component) else {
                    ok = false;
                    break 'param;
                };
                cur = applied;
            }
        }
        if !ok {
            continue;
        }
        concrete_dicts.insert((constraint.to_string(), key.to_string()), cur.clone());
        return Some(cur);
    }
    None
}

/// Match a conditional witness `pattern` against a concrete dispatch `key`
/// string (the `structural_witness_key` format the lowerer records in
/// `dict_dispatch_keys`), recovering the sub-key bound to each of `num_holes`
/// parameter holes. Returns `None` unless the whole key is consumed and every
/// hole is bound consistently.
fn match_pattern_key(
    pattern: &zutai_thir::WitnessPattern,
    key: &str,
    num_holes: usize,
) -> Option<Vec<String>> {
    let mut holes: Vec<Option<String>> = vec![None; num_holes];
    let rest = pattern_match_at(pattern, key, &mut holes)?;
    if !rest.is_empty() {
        return None;
    }
    holes.into_iter().collect()
}

fn pattern_match_at<'k>(
    pattern: &zutai_thir::WitnessPattern,
    s: &'k str,
    holes: &mut [Option<String>],
) -> Option<&'k str> {
    use zutai_thir::{WitnessPattern as P, WitnessPatternTupleItem as TI};
    match pattern {
        P::Hole(i) => {
            let (token, rest) = split_balanced(s)?;
            match holes.get_mut(*i)? {
                slot @ None => *slot = Some(token.to_string()),
                Some(prev) if prev == token => {}
                Some(_) => return None,
            }
            Some(rest)
        }
        P::Leaf(k) => s.strip_prefix(k.as_str()),
        P::List(inner) => {
            let s = s.strip_prefix('[')?;
            let s = pattern_match_at(inner, s, holes)?;
            s.strip_prefix(']')
        }
        P::Optional(inner) => {
            // The `?` marker is a postfix at this level (`<inner>?`), so reserve it
            // before matching the inner — otherwise a bare `Hole` inner greedily
            // consumes the `?` and the strip below fails.
            let (token, rest) = split_balanced(s)?;
            let inner_key = token.strip_suffix('?')?;
            if !pattern_match_at(inner, inner_key, holes)?.is_empty() {
                return None;
            }
            Some(rest)
        }
        P::Maybe(inner) => {
            let s = s.strip_prefix("Maybe[")?;
            let s = pattern_match_at(inner, s, holes)?;
            s.strip_prefix(']')
        }
        P::Record(fields) => {
            let mut s = s.strip_prefix('{')?;
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    s = s.strip_prefix(',')?;
                }
                s = s.strip_prefix(f.name.as_str())?;
                s = s.strip_prefix(if f.optional { "?:" } else { ":" })?;
                s = pattern_match_at(&f.ty, s, holes)?;
            }
            s.strip_prefix('}')
        }
        P::Tuple(items) => {
            let mut s = s.strip_prefix('(')?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    s = s.strip_prefix(',')?;
                }
                s = match item {
                    TI::Positional(p) => pattern_match_at(p, s, holes)?,
                    TI::Named { name, ty } => {
                        let s = s.strip_prefix(name.as_str())?.strip_prefix(':')?;
                        pattern_match_at(ty, s, holes)?
                    }
                };
            }
            s.strip_prefix(')')
        }
        P::Union(variants) => {
            let mut s = s.strip_prefix('<')?;
            for (i, v) in variants.iter().enumerate() {
                if i > 0 {
                    s = s.strip_prefix('|')?;
                }
                s = s.strip_prefix(v.name.as_str())?;
                if let Some(payload) = &v.payload {
                    s = s.strip_prefix('(')?;
                    s = pattern_match_at(payload, s, holes)?;
                    s = s.strip_prefix(')')?;
                }
            }
            s.strip_prefix('>')
        }
        P::Function(from, to) => {
            let s = s.strip_prefix('(')?;
            let s = pattern_match_at(from, s, holes)?;
            let s = s.strip_prefix("->")?;
            let s = pattern_match_at(to, s, holes)?;
            s.strip_prefix(')')
        }
    }
}

/// Split off the leading balanced type-key token from `s`, returning it and the
/// remainder. Stops at a top-level separator (`,` `|` `->`) or a closing
/// bracket, tracking `[] {} () <>` nesting so nested keys stay intact.
fn split_balanced(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        // `->` is an atomic arrow: a top-level one ends the token, and a nested
        // one must not let its `>` decrement bracket depth.
        if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'>') {
            if depth == 0 {
                break;
            }
            i += 2;
            continue;
        }
        match bytes[i] {
            b'[' | b'{' | b'(' | b'<' => depth += 1,
            b']' | b'}' | b')' | b'>' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            b',' | b'|' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    if i == 0 {
        return None;
    }
    Some((&s[..i], &s[i..]))
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
