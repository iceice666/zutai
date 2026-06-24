use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn new(
        hir: &'hir HirFile,
        imports: FxHashMap<ImportKey, ImportedType>,
    ) -> Self {
        let mut lowerer = Self {
            hir,
            imports,
            decl_arena: Arena::new(),
            expr_arena: Arena::new(),
            pat_arena: Arena::new(),
            type_arena: Vec::new(),
            aliases: FxHashMap::default(),
            value_types: FxHashMap::default(),
            diagnostics: Vec::new(),
            error_type: TypeId(0),
            type_type: TypeId(0),
            next_infer_var: 0,
            infer_subst: FxHashMap::default(),
            next_row_var: 0,
            row_subst: FxHashMap::default(),
            effect_ambient: EffectRow::closed_empty(),
            handled_stack: Vec::new(),
            resume_stack: Vec::new(),
            poly_schemes: FxHashMap::default(),
            type_param_kinds: FxHashMap::default(),
            next_level_meta: 0,
            level_lower_bounds: FxHashMap::default(),
            level_equalities: FxHashMap::default(),
            level_param_metas: FxHashMap::default(),
            type_universe_cache: FxHashMap::default(),
            alias_universe_in_progress: FxHashSet::default(),
            type_match_in_progress: FxHashSet::default(),
            alias_recursive_cache: FxHashMap::default(),
            alias_params: FxHashMap::default(),
            type_param_scope: FxHashSet::default(),
            type_eval_fuel: 10_000,
            binding_import_key: FxHashMap::default(),
            import_type_denotations: FxHashMap::default(),
        };
        lowerer.error_type = lowerer.alloc_type(Type {
            kind: TypeKind::Error,
            span: Span::default(),
        });
        // Convenience placeholder `Type` TypeId for sites that only need "this is
        // the Type sort" (builtins, imports, display). The level here is a ground
        // placeholder; type-valued expressions allocate their own leveled `Type`
        // (see `TypeForm` lowering in `expr/mod.rs`).
        lowerer.type_type = lowerer.alloc_type(Type {
            kind: TypeKind::Type(UniverseLevel::Known(0)),
            span: hir.span,
        });
        lowerer.seed_builtin_value_types();
        lowerer
    }

    pub(in crate::lower) fn lower_file(&mut self) -> LoweredThir {
        self.collect_type_param_kinds();
        self.predeclare_import_decls();
        self.predeclare_decl_types();
        // D5: Two-phase lowering.  Witness field RHSs may forward-reference later
        // top-level bindings that are unannotated (not pre-declared by
        // `predeclare_decl_types`).  Lowering normal decls first populates
        // `value_types` for all of them, letting constraint/witness lowering see a
        // complete top-level environment and avoiding `ValueTypeUnavailable` errors.
        //
        // Output order is always the original `hir.decls` source order so downstream
        // positional assumptions stay intact — the partition controls *lowering*
        // order, not *output* order.
        let (cw_ids, normal_ids): (Vec<_>, Vec<_>) =
            self.hir.decls.iter().copied().partition(|&id| {
                matches!(
                    self.hir_decl(id).kind,
                    HirDeclKind::Constraint { .. } | HirDeclKind::Witness { .. }
                )
            });
        let mut id_map: FxHashMap<HirDeclId, ThirDeclId> = FxHashMap::default();
        for id in normal_ids {
            id_map.insert(id, self.lower_decl(id));
        }
        // Constraints before witnesses: a witness checks its fields against the
        // constraint's (instantiated) method signatures, so the constraint decl
        // must already be in `decl_arena`.
        let (constraint_ids, witness_ids): (Vec<_>, Vec<_>) = cw_ids
            .into_iter()
            .partition(|&id| matches!(self.hir_decl(id).kind, HirDeclKind::Constraint { .. }));
        for id in constraint_ids {
            id_map.insert(id, self.lower_decl(id));
        }
        for id in witness_ids {
            id_map.insert(id, self.lower_decl(id));
        }
        self.check_witnesses();
        self.check_witness_coherence();
        // Reassemble in source order.
        let decls: Vec<_> = self
            .hir
            .decls
            .iter()
            .copied()
            .map(|id| id_map[&id])
            .collect();
        let saved_effect_ambient = self.enter_host_effect_boundary(self.hir.span);
        let final_expr = self.infer_expr(self.hir.final_expr);
        self.exit_effectful_result(saved_effect_ambient);

        // Zonk: replace solved InferVar slots in the type arena with their
        // concrete types so downstream consumers see fully-resolved types.
        self.zonk_type_arena();

        let type_universes = self.finalized_type_universes();
        let type_param_kinds = std::mem::take(&mut self.type_param_kinds)
            .into_iter()
            .map(|(binding, kind)| (binding, self.finalized_kind(kind)))
            .collect();
        let file = ThirFile {
            decls,
            final_expr,
            decl_arena: std::mem::take(&mut self.decl_arena),
            expr_arena: std::mem::take(&mut self.expr_arena),
            pat_arena: std::mem::take(&mut self.pat_arena),
            type_arena: std::mem::take(&mut self.type_arena),
            poly_schemes: std::mem::take(&mut self.poly_schemes),
            type_universes,
            type_param_kinds,
            binding_names: self
                .hir
                .bindings
                .iter()
                .map(|binding| binding.name.clone())
                .collect(),
            binding_kinds: self
                .hir
                .bindings
                .iter()
                .map(|binding| binding.kind)
                .collect(),
        };
        let diagnostics = std::mem::take(&mut self.diagnostics);

        LoweredThir {
            file: diagnostics.is_empty().then_some(file),
            diagnostics,
            pass_reports: Vec::new(),
        }
    }
    /// Populate `type_param_kinds` from every type parameter's `<.. :: Kind>`
    /// annotation across constraint, witness, function, and constraint-method
    /// param lists. Params without an annotation default to `Kind::ground()`.
    pub(in crate::lower) fn collect_type_param_kinds(&mut self) {
        let mut pending: Vec<(BindingId, Kind)> = Vec::new();
        for &decl_id in &self.hir.decls {
            let decl = self.hir_decl(decl_id);
            match &decl.kind {
                HirDeclKind::Constraint {
                    params, methods, ..
                } => {
                    for p in params {
                        if let Some(kind_ty) = p.kind {
                            pending.push((p.binding, self.hir_kind_of(kind_ty)));
                        }
                    }
                    for m in methods {
                        for p in &m.params {
                            if let Some(kind_ty) = p.kind {
                                pending.push((p.binding, self.hir_kind_of(kind_ty)));
                            }
                        }
                    }
                }
                HirDeclKind::Witness { params, .. } | HirDeclKind::Function { params, .. } => {
                    for p in params {
                        if let Some(kind_ty) = p.kind {
                            pending.push((p.binding, self.hir_kind_of(kind_ty)));
                        }
                    }
                }
                _ => {}
            }
        }
        for (binding, kind) in pending {
            self.type_param_kinds.insert(binding, kind);
        }
    }
}
