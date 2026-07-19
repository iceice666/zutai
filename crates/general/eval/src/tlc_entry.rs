use super::*;
use std::rc::Rc;

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

fn completed_tlc_inputs_strict(
    analysis: &zutai_semantic::Analysis,
) -> Result<(&ThirFile, &zutai_tlc::TlcModule), EvalError> {
    let inputs = completed_tlc_inputs_for_session(analysis)?;
    // Gate on the lowered module rather than THIR: a `Type`-typed THIR
    // subexpression inside a folded `schema` application never reaches TLC, so
    // only placeholders actually erased during lowering block the TLC path.
    if inputs.1.residual_type_values {
        return Err(EvalError::ReflectionUnsupported(
            "runtime Type values are not represented in the TLC evaluator yet".to_string(),
        ));
    }
    Ok(inputs)
}

/// Completed typed inputs for a persistent TLC session.
///
/// TLC intentionally erases `TypeValue` expressions to `Nothing`. Long-lived
/// browser sessions need that erasure because imported stdlib modules export
/// type names next to callable values, while the browser only retains/calls the
/// value fields. Strict legacy `eval_tlc_*` entry points remain gated by
/// [`completed_tlc_inputs_strict`] before constructing a session.
fn completed_tlc_inputs_for_session(
    analysis: &zutai_semantic::Analysis,
) -> Result<(&ThirFile, &zutai_tlc::TlcModule), EvalError> {
    let thir_file = check_well_typed(analysis)?;
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

/// Persistent, owned TLC evaluator state for a long-lived application.
///
/// Modules are cloned out of semantic analysis into stable `Rc` storage while
/// runtime closures retain compact `ModuleId` handles. The entry thunk, imports,
/// witness methods, and memoized top-level environments therefore survive
/// repeated event dispatch without reparsing or re-analysis.
///
/// A session accepts modules that export type names beside runtime values; TLC
/// erases those type-valued fields to `Nothing`. Strict `eval_tlc_*` APIs keep
/// rejecting runtime Type observation and reflection.
pub struct TlcSession {
    modules: Vec<Rc<zutai_tlc::TlcModule>>,
    root: ModuleId,
    imports: FxHashMap<ImportKey, Value>,
    operator_witnesses: FxHashMap<(String, String), Value>,
    entry: thunk::Thunk,
}

impl TlcSession {
    pub fn from_analysis(analysis: &zutai_semantic::Analysis) -> Result<Self, EvalError> {
        Self::build(analysis, None)
    }

    pub fn from_analysis_with_handler(
        analysis: &zutai_semantic::Analysis,
        handler: &dyn EffectHandler,
    ) -> Result<Self, EvalError> {
        Self::build(analysis, Some(handler))
    }

    fn build(
        analysis: &zutai_semantic::Analysis,
        handler: Option<&dyn EffectHandler>,
    ) -> Result<Self, EvalError> {
        let mut modules = Vec::new();
        let mut imports = FxHashMap::default();
        let mut tables = WitnessTables::default();
        let root =
            build_tlc_session_into(analysis, &mut modules, &mut imports, &mut tables, handler)?;
        let (thir_file, root_module) = completed_tlc_inputs_for_session(analysis)?;
        let registry: Vec<&zutai_tlc::TlcModule> = modules.iter().map(Rc::as_ref).collect();

        // Instantiate imported conditional and bare constructor witnesses for
        // the concrete dispatch keys needed by the root module.
        if !tables.conditionals.is_empty() || !tables.concrete_dicts.is_empty() {
            let mat_ev = evaluator_with_handler(&registry, root, &imports, None, handler)?;
            let needed: Vec<String> = root_module.dict_dispatch_keys.values().cloned().collect();
            let WitnessTables {
                operators,
                concrete_dicts,
                conditionals,
            } = &mut tables;
            let conds: &[ConditionalRuntimeWitness] = conditionals;
            for key in &needed {
                let mut constraints: Vec<String> = conds
                    .iter()
                    .map(|witness| witness.constraint.clone())
                    .chain(
                        concrete_dicts
                            .keys()
                            .map(|(constraint, _)| constraint.clone()),
                    )
                    .collect();
                constraints.sort();
                constraints.dedup();
                for constraint in constraints {
                    let Some(Value::Record(fields)) = materialize_conditional_dict(
                        &mat_ev,
                        &constraint,
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

        let ev =
            evaluator_with_handler(&registry, root, &imports, Some(&tables.operators), handler)?;
        let top = seed_tlc_prelude(thir_file, env::Env::empty());
        let top = ev.build_top_env_from(top)?;
        let final_id = root_module
            .final_expr
            .ok_or(EvalError::Internal("TLC module has no final expression"))?;
        let entry = thunk::Thunk::tlc_deferred(final_id, top, root);

        Ok(Self {
            modules,
            root,
            imports,
            operator_witnesses: tables.operators,
            entry,
        })
    }

    /// Evaluate and recursively force the root module's final expression.
    pub fn entry(&self) -> Result<Value, EvalError> {
        let value = self.force_thunk(&self.entry)?;
        self.force(value)
    }

    pub fn entry_with_handler(&self, handler: &dyn EffectHandler) -> Result<Value, EvalError> {
        let value = self.force_thunk_with_handler(&self.entry, handler)?;
        self.force_with_handler(value, handler)
    }

    /// Apply one argument. The result is settled but remains lazy internally.
    pub fn apply(&self, function: Value, argument: Value) -> Result<Value, EvalError> {
        self.apply_inner(function, argument, None)
    }

    pub fn apply_with_handler(
        &self,
        function: Value,
        argument: Value,
        handler: &dyn EffectHandler,
    ) -> Result<Value, EvalError> {
        self.apply_inner(function, argument, Some(handler))
    }

    pub fn apply2(&self, function: Value, first: Value, second: Value) -> Result<Value, EvalError> {
        let partial = self.apply(function, first)?;
        self.apply(partial, second)
    }

    pub fn apply2_with_handler(
        &self,
        function: Value,
        first: Value,
        second: Value,
        handler: &dyn EffectHandler,
    ) -> Result<Value, EvalError> {
        let partial = self.apply_with_handler(function, first, handler)?;
        self.apply_with_handler(partial, second, handler)
    }

    fn apply_inner(
        &self,
        function: Value,
        argument: Value,
        handler: Option<&dyn EffectHandler>,
    ) -> Result<Value, EvalError> {
        let registry = self.registry();
        let ev = evaluator_with_handler(
            &registry,
            self.root,
            &self.imports,
            Some(&self.operator_witnesses),
            handler,
        )?;
        ev.apply_to_value(function, argument)
    }

    /// Recursively force all lazy fields in `value`.
    pub fn force(&self, value: Value) -> Result<Value, EvalError> {
        self.force_inner(value, None)
    }

    pub fn force_with_handler(
        &self,
        value: Value,
        handler: &dyn EffectHandler,
    ) -> Result<Value, EvalError> {
        self.force_inner(value, Some(handler))
    }

    fn force_inner(
        &self,
        value: Value,
        handler: Option<&dyn EffectHandler>,
    ) -> Result<Value, EvalError> {
        let registry = self.registry();
        let ev = evaluator_with_handler(
            &registry,
            self.root,
            &self.imports,
            Some(&self.operator_witnesses),
            handler,
        )?;
        eval_tlc::tlc_force_deep(value, &ev)
    }

    /// Force one thunk without recursively forcing the returned value.
    pub fn force_thunk(&self, value: &thunk::Thunk) -> Result<Value, EvalError> {
        self.force_thunk_inner(value, None)
    }

    pub fn force_thunk_with_handler(
        &self,
        value: &thunk::Thunk,
        handler: &dyn EffectHandler,
    ) -> Result<Value, EvalError> {
        self.force_thunk_inner(value, Some(handler))
    }

    fn force_thunk_inner(
        &self,
        value: &thunk::Thunk,
        handler: Option<&dyn EffectHandler>,
    ) -> Result<Value, EvalError> {
        let registry = self.registry();
        let ev = evaluator_with_handler(
            &registry,
            self.root,
            &self.imports,
            Some(&self.operator_witnesses),
            handler,
        )?;
        value.force_tlc(&ev)
    }

    fn registry(&self) -> Vec<&zutai_tlc::TlcModule> {
        self.modules.iter().map(Rc::as_ref).collect()
    }
}

pub(super) fn eval_tlc_analysis(analysis: &zutai_semantic::Analysis) -> Result<Value, EvalError> {
    completed_tlc_inputs_strict(analysis)?;
    TlcSession::from_analysis(analysis)?.entry()
}

fn build_tlc_session_into(
    analysis: &zutai_semantic::Analysis,
    modules: &mut Vec<Rc<zutai_tlc::TlcModule>>,
    imports: &mut FxHashMap<ImportKey, Value>,
    tables: &mut WitnessTables,
    handler: Option<&dyn EffectHandler>,
) -> Result<ModuleId, EvalError> {
    let (_thir_file, module) = completed_tlc_inputs_for_session(analysis)?;

    for (key, value) in &analysis.import_values {
        imports
            .entry(key.clone())
            .or_insert_with(|| Value::from_immediate(value));
    }

    for (key, imported_analysis) in &analysis.import_modules {
        if imports.contains_key(key) {
            continue;
        }
        let dep_id = build_tlc_session_into(
            imported_analysis.as_ref(),
            modules,
            imports,
            tables,
            handler,
        )?;
        let (dep_thir_file, dep_module) =
            completed_tlc_inputs_for_session(imported_analysis.as_ref())?;
        let registry: Vec<&zutai_tlc::TlcModule> = modules.iter().map(Rc::as_ref).collect();
        let dep_ev = evaluator_with_handler(&registry, dep_id, imports, None, handler)?;
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

    let id = ModuleId(modules.len());
    modules.push(Rc::new(module.clone()));
    Ok(id)
}

fn evaluator_with_handler<'a>(
    registry: &'a [&'a zutai_tlc::TlcModule],
    active_module: ModuleId,
    imports: &'a FxHashMap<ImportKey, Value>,
    operator_witnesses: Option<&'a FxHashMap<(String, String), Value>>,
    handler: Option<&'a dyn EffectHandler>,
) -> Result<eval_tlc::TlcEvaluator<'a>, EvalError> {
    let mut evaluator = match operator_witnesses {
        Some(witnesses) => eval_tlc::TlcEvaluator::new_in_registry_with_operator_witnesses(
            registry,
            active_module,
            imports,
            witnesses,
        )?,
        None => eval_tlc::TlcEvaluator::new_in_registry(registry, active_module, imports)?,
    };
    if let Some(handler) = handler {
        evaluator = evaluator.with_effect_handler(handler);
    }
    Ok(evaluator)
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
    if let Some((_, dict)) = concrete_dicts
        .iter()
        .find(|((dict_constraint, dict_key), _)| {
            dict_constraint == constraint
                && key
                    .strip_prefix(dict_key.as_str())
                    .is_some_and(|suffix| suffix.starts_with('['))
        })
    {
        return Some(dict.clone());
    }
    for cw in conditionals {
        if cw.constraint != constraint {
            continue;
        }
        let Some(sub_keys) = zutai_thir::match_pattern_key(&cw.pattern, key, cw.param_bounds.len())
        else {
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
        TypeKind::Con(binding) => thir_file.binding_names.get(binding.0 as usize).cloned(),
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
