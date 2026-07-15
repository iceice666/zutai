use super::*;

impl<'hir> Lowerer<'hir> {
    /// Check each non-derive witness's fields against the corresponding constraint's
    /// method signatures, with the constraint's type param substituted by the witness
    /// target type. Emits WitnessFieldTypeMismatch, MissingWitnessField, and
    /// UnknownWitnessField diagnostics. Must run after the entire cw-lowering loop
    /// (D7) and before zonk_type_arena() so infer-var solutions get zonked.
    pub(in crate::lower) fn check_witnesses(&mut self) {
        // Phase 1: immutable scan — collect owned data to avoid borrow conflicts.

        #[derive(Clone)]
        struct ConstraintInfo {
            name: String,
            params: Vec<BindingId>,
            methods: Vec<(String, bool, bool, TypeId)>,
            method_params: FxHashMap<String, Vec<BindingId>>,
            derivable: bool,
            has_recipe: bool,
            has_code_recipe: bool,
            code_recipe_type: Option<TypeId>,
            /// Source span of the constraint declaration — the "expansion
            /// definition" location threaded into derive/recipe diagnostics.
            definition: Span,
        }

        let mut constraint_map: FxHashMap<BindingId, ConstraintInfo> = FxHashMap::default();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Constraint {
                params,
                methods,
                derivable,
                recipe,
                ..
            } = &decl.kind
            {
                let owned_methods: Vec<(String, bool, bool, TypeId)> = methods
                    .iter()
                    .map(|m| (m.name.clone(), m.optional, m.default.is_some(), m.sig))
                    .collect();
                let owned_method_params: FxHashMap<String, Vec<BindingId>> = methods
                    .iter()
                    .map(|m| (m.name.clone(), m.params.clone()))
                    .collect();
                let name = self.hir.bindings[decl.binding.0 as usize].name.clone();
                let code_recipe_type = recipe.as_ref().and_then(|recipe| {
                    let recipe_ty = self.expr(recipe.body).ty;
                    match self.ty(recipe_ty).kind {
                        TypeKind::Code(inner) => Some(inner),
                        _ => None,
                    }
                });
                constraint_map.insert(
                    decl.binding,
                    ConstraintInfo {
                        name,
                        params: params.clone(),
                        methods: owned_methods,
                        method_params: owned_method_params,
                        derivable: *derivable,
                        has_recipe: recipe.is_some(),
                        has_code_recipe: code_recipe_type.is_some_and(|inner| {
                            matches!(self.ty(inner).kind, TypeKind::Record(_, _))
                        }),
                        code_recipe_type,
                        definition: decl.span,
                    },
                );
            }
        }
        struct WitnessTask {
            span: Span,
            target: TypeId,
            constraint_param: BindingId,
            constraint_name: String,
            methods: Vec<(String, bool, bool, TypeId)>,
            method_params: FxHashMap<String, Vec<BindingId>>,
            fields: Vec<(String, ThirExprId, Span)>,
        }
        struct DeriveTask {
            span: Span,
            target: TypeId,
            constraint: BindingId,
            constraint_name: String,
            methods: Vec<(String, bool, bool, TypeId)>,
            derivable: bool,
            has_recipe: bool,
            has_code_recipe: bool,
            code_recipe_type: Option<TypeId>,
            definition: Span,
        }
        let mut tasks: Vec<WitnessTask> = Vec::new();
        let mut derive_tasks: Vec<DeriveTask> = Vec::new();
        // Multi-param constraint names and their witness spans, collected for
        // diagnostic emission after the immutable scan loop ends.
        let mut multi_param_errors: Vec<(String, Span)> = Vec::new();
        // Conditional witnesses whose target may be self-referential; resolved and
        // checked after the immutable scan.
        #[allow(clippy::type_complexity)]
        let mut recursive_candidates: Vec<(
            String,
            TypeId,
            Vec<BindingId>,
            Vec<Vec<BindingId>>,
            BindingId,
            Span,
        )> = Vec::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Witness {
                constraint,
                target,
                params,
                param_bounds,
                derive,
                fields,
                ..
            } = &decl.kind
            {
                let Some(cst_binding) = constraint else {
                    continue;
                };
                let Some(cst_info) = constraint_map.get(cst_binding) else {
                    continue;
                };
                if cst_info.params.len() != 1 {
                    // Multi-param constraints are not yet supported: collect for
                    // diagnostic emission below (outside the immutable-borrow loop).
                    multi_param_errors.push((cst_info.name.clone(), decl.span));
                    continue;
                }
                if !params.is_empty() {
                    recursive_candidates.push((
                        cst_info.name.clone(),
                        *target,
                        params.clone(),
                        param_bounds.clone(),
                        *cst_binding,
                        decl.span,
                    ));
                }
                if *derive {
                    derive_tasks.push(DeriveTask {
                        span: decl.span,
                        target: *target,
                        constraint: *cst_binding,
                        constraint_name: cst_info.name.clone(),
                        methods: cst_info.methods.clone(),
                        derivable: cst_info.derivable,
                        has_recipe: cst_info.has_recipe,
                        has_code_recipe: cst_info.has_code_recipe,
                        code_recipe_type: cst_info.code_recipe_type,
                        definition: cst_info.definition,
                    });
                    continue;
                }
                let fields_owned: Vec<(String, ThirExprId, Span)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), f.value, f.span))
                    .collect();
                tasks.push(WitnessTask {
                    span: decl.span,
                    target: *target,
                    constraint_param: cst_info.params[0],
                    constraint_name: cst_info.name.clone(),
                    methods: cst_info.methods.clone(),
                    method_params: cst_info.method_params.clone(),
                    fields: fields_owned,
                });
            }
        }

        // Emit UnsupportedMultiParamConstraint diagnostics (collected above).
        for (name, span) in multi_param_errors {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnsupportedMultiParamConstraint { name },
                span,
            });
        }

        // A conditional witness whose target is one of its own params loops only
        // when that param's bound requires the *same* constraint being defined
        // (`Eq @A :: <A: Eq>`): resolving `Eq A` then needs `Eq A` again. A bound
        // by a *different* constraint (`Eq @A :: <A: Ord>`) makes progress —
        // consuming an `Ord` dict to produce an `Eq` dict — and is not recursive.
        for (name, target, params, param_bounds, cst_binding, span) in recursive_candidates {
            let resolved = self.resolve_alias(target, &mut FxHashSet::default(), span);
            if let TypeKind::TypeVar(b) = self.type_arena[resolved.0 as usize].kind
                && let Some(idx) = params.iter().position(|p| *p == b)
                && param_bounds
                    .get(idx)
                    .is_some_and(|bounds| bounds.contains(&cst_binding))
            {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::RecursiveWitness { constraint: name },
                    span,
                });
            }
        }

        // Phase 2: mutable checks over owned task data.
        for task in tasks {
            // Kind-check the witness target against the constraint's target kind
            // (`Functor @Int` is rejected: `Int : Type` but `Functor` wants
            // `Type -> Type`). Skip field checks for a mis-kinded witness.
            let expected_kind = self
                .type_param_kinds
                .get(&task.constraint_param)
                .cloned()
                .unwrap_or_else(Kind::ground);
            let target_kind = self.kind_of(task.target, task.span);
            if !self.kind_compatible(&expected_kind, &target_kind, task.span) {
                let target_name = self.type_name(task.target);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::WitnessTargetKindMismatch {
                        constraint: task.constraint_name.clone(),
                        target: target_name,
                    },
                    span: task.span,
                });
                continue;
            }
            let subst: FxHashMap<BindingId, TypeId> =
                [(task.constraint_param, task.target)].into_iter().collect();

            let field_names: FxHashSet<String> =
                task.fields.iter().map(|(n, _, _)| n.clone()).collect();

            for (fname, value_expr, fspan) in &task.fields {
                if let Some((_, _, _, method_sig)) =
                    task.methods.iter().find(|(n, _, _, _)| n == fname)
                {
                    let mut field_subst = subst.clone();
                    if let Some(mps) = task.method_params.get(fname) {
                        let mspan = self.ty(*method_sig).span;
                        for &mp in mps {
                            let fresh = self.fresh_infer_var(mspan);
                            field_subst.insert(mp, fresh);
                        }
                    }
                    let expected = self.instantiate_type_vars(*method_sig, &field_subst);
                    let found = self.expr(*value_expr).ty;
                    let expected_name = self.type_name(expected);
                    let found_name = self.type_name(found);
                    if !self.type_matches(expected, found) {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::WitnessFieldTypeMismatch {
                                name: fname.clone(),
                                expected: expected_name,
                                found: found_name,
                            },
                            span: *fspan,
                        });
                    }
                } else {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::UnknownWitnessField {
                            name: fname.clone(),
                        },
                        span: *fspan,
                    });
                }
            }

            for (mname, optional, has_default, _) in &task.methods {
                // D6/4a: suppress MissingWitnessField when the method has a default body.
                if !optional && !has_default && !field_names.contains(mname) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::MissingWitnessField {
                            name: mname.clone(),
                        },
                        span: task.span,
                    });
                }
            }
        }

        let explicit_witnesses = self.collect_explicit_witness_keys();
        for task in derive_tasks {
            if !task.derivable {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::DeriveConstraintNotDerivable {
                        constraint: task.constraint_name.clone(),
                        definition: task.definition,
                    },
                    span: task.span,
                });
                continue;
            }
            if self.derive_target_is_open_row(task.target) {
                let target = self.type_name(task.target);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::DeriveOpenRowTarget {
                        constraint: task.constraint_name.clone(),
                        target,
                        definition: task.definition,
                    },
                    span: task.span,
                });
                continue;
            }
            let has_eq_method = task
                .methods
                .iter()
                .any(|(name, _, _, _)| derive_method_is_eq(name));
            let mut unsupported = false;
            if task.constraint_name == "FromData" {
                // `FromData` is the first typed structural-code recipe. Its
                // target-shape validation and component expansion happen at the
                // staging boundary, not through equality-family method names.
                if !self.supports_from_data_target(task.target, &mut FxHashSet::default()) {
                    let found = self.type_name(task.target);
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::DeriveRecipeTypeMismatch {
                            constraint: task.constraint_name.clone(),
                            method: "fromData".to_string(),
                            expected: "Bool, Int, Float, Text, atom, List, Optional, closed non-recursive record, or closed non-recursive union".to_string(),
                            found,
                            definition: task.definition,
                        },
                        span: task.span,
                    });
                    unsupported = true;
                }
            } else if !task.has_recipe {
                for (name, optional, has_default, _) in &task.methods {
                    if *optional || *has_default {
                        continue;
                    }
                    // A method is structurally derivable only if it is equality-family
                    // AND a positive `eq`/`==` recipe exists to build on (a lone
                    // `neq`/`!=` cannot be derived: there is nothing to negate).
                    if !derive_method_is_equality(name) || !has_eq_method {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::DeriveUnsupportedMethod {
                                constraint: task.constraint_name.clone(),
                                method: name.clone(),
                                definition: task.definition,
                            },
                            span: task.span,
                        });
                        unsupported = true;
                    }
                }
            } else if !task.has_code_recipe
                && !task.methods.iter().any(|(name, _, _, _)| {
                    matches!(
                        derive_recipe_method_kind(name),
                        Some(DeriveRecipeMethodKind::Show | DeriveRecipeMethodKind::Ord)
                    )
                })
            {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::DeriveRecipeTypeMismatch {
                        constraint: task.constraint_name.clone(),
                        method: "<recipe>".to_string(),
                        expected: "supported Show/Ord-style method".to_string(),
                        found: "custom recipe".to_string(),
                        definition: task.definition,
                    },
                    span: task.span,
                });
                unsupported = true;
            }
            if unsupported {
                continue;
            }
            if let Some(recipe_ty) = task.code_recipe_type {
                let TypeKind::Record(fields, RowTail::Closed) = self.ty(recipe_ty).kind.clone()
                else {
                    let found = self.type_name(recipe_ty);
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::DeriveRecipeTypeMismatch {
                            constraint: task.constraint_name.clone(),
                            method: "<recipe>".to_string(),
                            expected: "closed witness record".to_string(),
                            found,
                            definition: task.definition,
                        },
                        span: task.span,
                    });
                    continue;
                };
                let subst: FxHashMap<BindingId, TypeId> = constraint_map
                    .get(&task.constraint)
                    .and_then(|info| info.params.first().copied())
                    .map(|param| [(param, task.target)].into_iter().collect())
                    .unwrap_or_default();
                for field in &fields {
                    let Some((_, _, _, method_sig)) = task
                        .methods
                        .iter()
                        .find(|(name, _, _, _)| name == &field.name)
                    else {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::UnknownWitnessField {
                                name: field.name.clone(),
                            },
                            span: task.span,
                        });
                        continue;
                    };
                    let expected = self.instantiate_type_vars(*method_sig, &subst);
                    if !self.type_matches(expected, field.ty) {
                        let expected_name = self.type_name(expected);
                        let found_name = self.type_name(field.ty);
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::DeriveRecipeTypeMismatch {
                                constraint: task.constraint_name.clone(),
                                method: field.name.clone(),
                                expected: expected_name,
                                found: found_name,
                                definition: task.definition,
                            },
                            span: task.span,
                        });
                    }
                }
                for (method, optional, has_default, _) in &task.methods {
                    if !optional
                        && !has_default
                        && !fields.iter().any(|field| &field.name == method)
                    {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::MissingWitnessField {
                                name: method.clone(),
                            },
                            span: task.span,
                        });
                    }
                }
            }
            if task.has_code_recipe || task.constraint_name == "FromData" {
                continue;
            }
            for component in self.derive_components(task.target) {
                if !self.derive_component_has_witness(
                    task.constraint,
                    component,
                    &task.methods,
                    &explicit_witnesses,
                ) {
                    let component_name = self.type_name(component);
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::DeriveComponentMissingWitness {
                            constraint: task.constraint_name.clone(),
                            component: component_name,
                            definition: task.definition,
                        },
                        span: task.span,
                    });
                }
            }
        }
    }

    /// Whether a derive target's top-level shape is an *open* record or union
    /// row (a `...` or `...Rest` tail). Structural derives (`eq`/`show`/`compare`
    /// and the reflection/`FromData` recipes) enumerate a target's members; an
    /// open row hides members, so a witness built over the visible members is
    /// unsound (`eq p p` can read `false`, `compare` can crash). Reflection and
    /// `FromData` already refuse open rows at their own boundaries; this closes
    /// the top-level equality/show/ord derive path to match.
    fn derive_target_is_open_row(&mut self, ty: TypeId) -> bool {
        let span = self.type_arena[ty.0 as usize].span;
        let resolved = self.resolve_alias(ty, &mut FxHashSet::default(), span);
        match self.type_arena[resolved.0 as usize].kind.clone() {
            TypeKind::Record(_, tail) | TypeKind::Union(_, tail) => {
                matches!(tail, RowTail::Open | RowTail::Param(_))
            }
            _ => false,
        }
    }

    fn supports_from_data_target(&self, ty: TypeId, seen: &mut FxHashSet<BindingId>) -> bool {
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Text
            | TypeKind::Atom(_) => true,
            TypeKind::List(inner) | TypeKind::Optional(inner) => {
                self.supports_from_data_target(inner, seen)
            }
            TypeKind::Record(fields, RowTail::Closed) => fields
                .iter()
                .all(|field| self.supports_from_data_target(field.ty, seen)),
            TypeKind::Union(variants, RowTail::Closed) => variants.iter().all(|variant| {
                variant
                    .payload
                    .is_none_or(|payload| self.supports_from_data_target(payload, seen))
            }),
            TypeKind::Alias(binding) => {
                if !seen.insert(binding) {
                    return false;
                }
                let result = self
                    .aliases
                    .get(&binding)
                    .copied()
                    .is_some_and(|body| self.supports_from_data_target(body, seen));
                seen.remove(&binding);
                result
            }
            TypeKind::AliasApply { binding, .. } => {
                if !seen.insert(binding) {
                    return false;
                }
                let result = self
                    .aliases
                    .get(&binding)
                    .copied()
                    .is_some_and(|body| self.supports_from_data_target(body, seen));
                seen.remove(&binding);
                result
            }
            TypeKind::TypeVar(_) => true,
            _ => false,
        }
    }

    pub(in crate::lower) fn collect_explicit_witness_keys(
        &mut self,
    ) -> FxHashSet<(BindingId, String)> {
        let witnesses: Vec<(BindingId, TypeId)> = self
            .decl_arena
            .iter()
            .filter_map(|(_, decl)| {
                if let ThirDeclKind::Witness {
                    constraint: Some(constraint),
                    target,
                    ..
                } = &decl.kind
                {
                    Some((*constraint, *target))
                } else {
                    None
                }
            })
            .collect();

        witnesses
            .into_iter()
            .map(|(constraint, target)| (constraint, self.witness_target_key(target)))
            .collect()
    }

    pub(in crate::lower) fn derive_components(&mut self, target: TypeId) -> Vec<TypeId> {
        let span = self.type_arena[target.0 as usize].span;
        let target = self.resolve_alias(target, &mut FxHashSet::default(), span);
        match self.type_arena[target.0 as usize].kind.clone() {
            TypeKind::Record(fields, _) => fields.into_iter().map(|field| field.ty).collect(),
            TypeKind::Tuple(items) => items
                .into_iter()
                .map(|item| match item {
                    TypeTupleItem::Named { ty, .. } | TypeTupleItem::Positional(ty) => ty,
                })
                .collect(),
            TypeKind::Union(variants, _) => {
                let mut components = Vec::new();
                for variant in variants {
                    if let Some(payload) = variant.payload {
                        let payload_span = self.type_arena[payload.0 as usize].span;
                        let payload =
                            self.resolve_alias(payload, &mut FxHashSet::default(), payload_span);
                        match self.type_arena[payload.0 as usize].kind.clone() {
                            TypeKind::Record(fields, _) => {
                                components.extend(fields.into_iter().map(|field| field.ty));
                            }
                            _ => components.push(payload),
                        }
                    }
                }
                components
            }
            _ => Vec::new(),
        }
    }

    pub(in crate::lower) fn derive_component_has_witness(
        &mut self,
        constraint: BindingId,
        component: TypeId,
        methods: &[(String, bool, bool, TypeId)],
        witness_keys: &FxHashSet<(BindingId, String)>,
    ) -> bool {
        if self.derive_can_use_builtin_leaf(component, methods) {
            return true;
        }

        let key = self.witness_target_key(component);
        witness_keys.contains(&(constraint, key))
    }

    pub(in crate::lower) fn derive_can_use_builtin_leaf(
        &mut self,
        ty: TypeId,
        methods: &[(String, bool, bool, TypeId)],
    ) -> bool {
        let supports_eq = methods
            .iter()
            .any(|(name, _, _, _)| derive_method_is_equality(name));
        let supports_show = methods.iter().any(|(name, _, _, _)| {
            matches!(
                derive_recipe_method_kind(name),
                Some(DeriveRecipeMethodKind::Show)
            )
        });
        let supports_ord = methods.iter().any(|(name, _, _, _)| {
            matches!(
                derive_recipe_method_kind(name),
                Some(DeriveRecipeMethodKind::Ord)
            )
        });
        if !(supports_eq || supports_show || supports_ord) {
            return false;
        }

        // Eq and Ord lower leaves to builtin comparison ops (`Eq`/`Lt`/`Gt`), so
        // a primitive component needs no witness. Show has no primitive
        // rendering builtin (`Int -> Text` etc. do not exist): the spec requires
        // every Show component to delegate through `witness Show @Component`, so a
        // Show leaf is never builtin-eligible and a missing component witness is
        // reported as `DeriveComponentMissingWitness`.
        let span = self.type_arena[ty.0 as usize].span;
        let ty = self.resolve_alias(ty, &mut FxHashSet::default(), span);
        match self.type_arena[ty.0 as usize].kind {
            TypeKind::Int | TypeKind::Float | TypeKind::Posit(_) | TypeKind::Text => {
                supports_eq || supports_ord
            }
            TypeKind::Bool | TypeKind::True | TypeKind::False | TypeKind::Atom(_) => supports_eq,
            _ => false,
        }
    }

    /// Enforce coherence: at most one witness per `(Constraint, Type)` pair.
    ///
    /// For each non-`derive` or `derive` witness whose `constraint` binding is
    /// resolved, compute a structural key `(constraint_binding, target_key)`.
    /// If a prior witness already claimed that key, emit `ConflictingWitness` at
    /// the later witness's span. Witnesses with `constraint == None` (unresolved
    /// constraint name) are skipped — that error is reported elsewhere.
    ///
    /// Must run after `check_witnesses` and before `zonk_type_arena()`.
    pub(in crate::lower) fn check_witness_coherence(&mut self) {
        // Phase 1: immutable scan — collect (constraint, target, params, span).
        let mut entries: Vec<(BindingId, TypeId, Vec<BindingId>, Span)> = Vec::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Witness {
                constraint: Some(cst),
                target,
                params,
                ..
            } = &decl.kind
            {
                entries.push((*cst, *target, params.clone(), decl.span));
            }
        }

        // Phase 2: mutable — compute param-normalized keys, detect duplicates so
        // two conditional witnesses that overlap (e.g. two `Eq @(List A)`) are
        // flagged as ambiguous.
        let mut seen: FxHashMap<(BindingId, String), ()> = FxHashMap::default();
        for (cst, target, params, span) in entries {
            let norm: FxHashMap<BindingId, usize> =
                params.iter().enumerate().map(|(i, &p)| (p, i)).collect();
            let target_key = self.witness_target_key_with(target, &norm);
            let key = (cst, target_key);
            if let std::collections::hash_map::Entry::Vacant(entry) = seen.entry(key) {
                entry.insert(());
            } else {
                let constraint_name = self.hir.bindings[cst.0 as usize].name.clone();
                let target_name = self.type_name(target);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ConflictingWitness {
                        constraint: constraint_name,
                        target: target_name,
                    },
                    span,
                });
            }
        }
    }
}

fn derive_method_is_equality(method_name: &str) -> bool {
    matches!(method_name, "eq" | "==" | "neq" | "!=")
}

fn derive_method_is_eq(method_name: &str) -> bool {
    matches!(method_name, "eq" | "==")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::lower) enum DeriveRecipeMethodKind {
    Show,
    Ord,
}

pub(in crate::lower) fn derive_recipe_method_kind(
    method_name: &str,
) -> Option<DeriveRecipeMethodKind> {
    match method_name {
        "show" => Some(DeriveRecipeMethodKind::Show),
        "compare" => Some(DeriveRecipeMethodKind::Ord),
        _ => None,
    }
}
