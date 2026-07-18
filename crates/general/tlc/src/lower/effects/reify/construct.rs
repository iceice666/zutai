// ── Construction: Computation type, drivers, and the reify transform ───────────

use super::*;

impl<'m> Reifier<'m> {
    /// Build the recursive `Computation` type alias and the per-target context.
    pub(super) fn build_ctx(
        &mut self,
        ops_clauses: &[TlcHandleClause],
        op_meta: FxHashMap<String, (TlcTypeId, TlcTypeId)>,
        fn_set: FxHashSet<BindingId>,
        handle_ops: FxHashSet<String>,
        carrier_ty: TlcTypeId,
    ) {
        let comp_binding = self.fresh_binding();
        let comp_ref_ty = self.alloc_type(TlcType::TyVar(
            TlcTypeVar::Named(comp_binding.0),
            Kind::ground(),
        ));
        let pure_payload_ty = self.alloc_type(TlcType::Record(Row::from_record_fields([(
            "value".to_string(),
            carrier_ty,
            false,
        )])));

        let mut ops_map: FxHashMap<String, OpInfo> = FxHashMap::default();
        let mut variant_fields: Vec<(String, TlcTypeId)> =
            vec![(PURE_TAG.to_string(), pure_payload_ty)];
        for clause in ops_clauses {
            let (arg_ty, resume_ty) = op_meta[&clause.op];
            let resume_fn = self.fun_ty(resume_ty, comp_ref_ty);
            let payload_ty = self.alloc_type(TlcType::Record(Row::from_record_fields([
                ("payload".to_string(), arg_ty, false),
                ("resume".to_string(), resume_fn, false),
            ])));
            variant_fields.push((op_tag(&clause.op), payload_ty));
            ops_map.insert(
                clause.op.clone(),
                OpInfo {
                    arg_ty,
                    resume_ty,
                    payload_ty,
                    handler_body: clause.body,
                },
            );
        }
        let variant_ty = self.alloc_type(TlcType::VariantT(Row::from_fields(variant_fields)));
        let alias = self.module.decl_arena.alloc(TlcDecl::TypeAlias {
            binding: comp_binding,
            params: vec![],
            body: variant_ty,
        });
        self.module.decls.push(alias);

        // Effectful-codata (V3-G4): build a scope-local `Cell'` per effectful cell,
        // rewriting each effectful field to `Computation`-data and the recursive
        // `tail` to `Unit -> Cell'`.
        self.build_cell_primes(comp_ref_ty);

        let cont_ty = self.fun_ty(carrier_ty, comp_ref_ty);
        let bind_inner = self.fun_ty(cont_ty, comp_ref_ty);
        let bind_ty = self.fun_ty(comp_ref_ty, bind_inner);
        let bind_binding = self.fresh_binding();

        self.ctx = Some(ReifyCtx {
            comp_ref_ty,
            carrier_ty,
            pure_payload_ty,
            ops: ops_map,
            fn_set,
            handle_ops,
            bind_binding,
            bind_ty,
            cont_ty,
            fn_new_ty: FxHashMap::default(),
        });
    }

    /// The body type of the `TypeAlias` whose binding id is `cell_id`.
    pub(super) fn alias_body(&self, cell_id: u32) -> Option<TlcTypeId> {
        self.module
            .decl_arena
            .iter()
            .find_map(|(_, decl)| match decl {
                TlcDecl::TypeAlias { binding, body, .. } if binding.0 == cell_id => Some(*body),
                _ => None,
            })
    }

    /// `(name, ty, optional)` of each field of a record row.
    pub(super) fn record_fields_of(&self, row: &Row) -> Vec<(String, TlcTypeId, bool)> {
        let mut out = Vec::new();
        let mut r = row;
        while let Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } = r
        {
            out.push((label.clone(), *ty, *optional));
            r = tail;
        }
        out
    }

    /// Whether `ty` is a demand thunk `_ -> Cell` for the given effectful cell.
    pub(super) fn is_demand_thunk_of_cell(&self, ty: TlcTypeId, cell_id: u32) -> bool {
        matches!(&self.module.type_arena[ty], TlcType::Fun(_, b, _)
            if self.cell_identity(*b) == Some(cell_id))
    }

    /// Build a scope-local `Cell'` alias per effectful cell: effectful fields →
    /// `Computation`-data, recursive `tail` → `Unit -> Cell'`. The fresh alias
    /// reference is registered in `cell_prime` *before* its body is built so the
    /// recursive `tail` back-edge ties the knot.
    pub(super) fn build_cell_primes(&mut self, comp_ref: TlcTypeId) {
        let cells: Vec<(u32, FxHashSet<(String, String)>)> = self
            .eff_fields
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let mut new_binding: FxHashMap<u32, BindingId> = FxHashMap::default();
        for (cell_id, _) in &cells {
            let b = self.fresh_binding();
            let r = self.alloc_type(TlcType::TyVar(TlcTypeVar::Named(b.0), Kind::ground()));
            new_binding.insert(*cell_id, b);
            self.cell_prime.insert(*cell_id, r);
        }
        for (cell_id, eff_set) in &cells {
            let Some(body_ty) = self.alias_body(*cell_id) else {
                continue;
            };
            let TlcType::VariantT(row) = self.module.type_arena[body_ty].clone() else {
                continue;
            };
            let cell_prime_ref = self.cell_prime[cell_id];
            let arms: Vec<(String, TlcTypeId)> =
                row.fields().map(|(t, ty)| (t.to_string(), ty)).collect();
            let mut new_arms: Vec<(String, TlcTypeId)> = Vec::new();
            for (tag, payload_ty) in arms {
                let new_payload = match self.module.type_arena[payload_ty].clone() {
                    TlcType::Record(prow) => {
                        let fields = self.record_fields_of(&prow);
                        let new_fields: Vec<(String, TlcTypeId, bool)> = fields
                            .into_iter()
                            .map(|(name, fty, opt)| {
                                let nty = if eff_set.contains(&(tag.clone(), name.clone())) {
                                    comp_ref
                                } else if self.is_demand_thunk_of_cell(fty, *cell_id) {
                                    match self.module.type_arena[fty].clone() {
                                        TlcType::Fun(a, _, _) => self.fun_ty(a, cell_prime_ref),
                                        _ => fty,
                                    }
                                } else {
                                    fty
                                };
                                (name, nty, opt)
                            })
                            .collect();
                        self.alloc_type(TlcType::Record(Row::from_record_fields(new_fields)))
                    }
                    _ => payload_ty,
                };
                new_arms.push((tag, new_payload));
            }
            let new_variant = self.alloc_type(TlcType::VariantT(Row::from_fields(new_arms)));
            let alias = self.module.decl_arena.alloc(TlcDecl::TypeAlias {
                binding: new_binding[cell_id],
                params: vec![],
                body: new_variant,
            });
            self.module.decls.push(alias);
        }
    }

    /// Whether `ty` is an arrow whose curried spine carries only handled effects
    /// (so the function it types is reified to `Computation` form).
    pub(super) fn arrow_is_handled(&self, ty: TlcTypeId) -> bool {
        if !self.fun_spine_has_effect(ty) {
            return false;
        }
        let mut ops = FxHashSet::default();
        self.effect_ops_of(ty, &mut ops);
        ops.is_subset(&self.ctx().handle_ops)
    }

    /// Rewrite a type to its `Computation`-returning monadic form: any curried
    /// arrow whose spine carries a handled effect has that effectful result
    /// replaced by the (single, closed) `Computation` type and its rows cleared.
    /// Recurses into composite types (tuples, records, lists, unions) so an
    /// effectful arrow nested in, e.g., a tupled multi-parameter scrutinee is
    /// rewritten too. Returns `ty` unchanged when nothing needs rewriting.
    pub(super) fn monadic_ty(&mut self, ty: TlcTypeId) -> TlcTypeId {
        let comp = self.ctx().comp_ref_ty;
        // An effectful codata type `Cell` (alias ref or resolved body) → `Cell'`.
        if let Some(cell_id) = self.cell_identity(ty)
            && let Some(&cprime) = self.cell_prime.get(&cell_id)
        {
            return cprime;
        }
        match self.module.type_arena[ty].clone() {
            TlcType::Fun(a, b, row) => {
                let a2 = self.monadic_ty(a);
                if self.arrow_is_handled(ty) {
                    if !matches!(row, Row::REmpty) {
                        // This arrow carries the handled effect; result → Computation.
                        self.fun_ty(a2, comp)
                    } else {
                        let b2 = self.monadic_ty(b);
                        self.fun_ty(a2, b2)
                    }
                } else {
                    let b2 = self.monadic_ty(b);
                    if a2 == a && b2 == b {
                        ty
                    } else {
                        self.alloc_type(TlcType::Fun(a2, b2, row))
                    }
                }
            }
            TlcType::Tuple(fields) => {
                let mut changed = false;
                let new_fields: Vec<TlcTupleField> = fields
                    .into_iter()
                    .map(|f| match f {
                        TlcTupleField::Named { name, ty } => {
                            let ty2 = self.monadic_ty(ty);
                            changed |= ty2 != ty;
                            TlcTupleField::Named { name, ty: ty2 }
                        }
                        TlcTupleField::Positional(ty) => {
                            let ty2 = self.monadic_ty(ty);
                            changed |= ty2 != ty;
                            TlcTupleField::Positional(ty2)
                        }
                    })
                    .collect();
                if changed {
                    self.alloc_type(TlcType::Tuple(new_fields))
                } else {
                    ty
                }
            }
            TlcType::List(inner) => {
                let i2 = self.monadic_ty(inner);
                if i2 == inner {
                    ty
                } else {
                    self.alloc_type(TlcType::List(i2))
                }
            }
            TlcType::Optional(inner) => {
                let i2 = self.monadic_ty(inner);
                if i2 == inner {
                    ty
                } else {
                    self.alloc_type(TlcType::Optional(i2))
                }
            }
            TlcType::Maybe(inner) => {
                let i2 = self.monadic_ty(inner);
                if i2 == inner {
                    ty
                } else {
                    self.alloc_type(TlcType::Maybe(i2))
                }
            }
            TlcType::Record(row) => {
                let (row2, changed) = self.monadic_row(&row);
                if changed {
                    self.alloc_type(TlcType::Record(row2))
                } else {
                    ty
                }
            }
            TlcType::VariantT(row) => {
                let (row2, changed) = self.monadic_row(&row);
                if changed {
                    self.alloc_type(TlcType::VariantT(row2))
                } else {
                    ty
                }
            }
            _ => ty,
        }
    }

    /// Rewrite each field type of a row via `monadic_ty`; report whether changed.
    pub(super) fn monadic_row(&mut self, row: &Row) -> (Row, bool) {
        match row {
            Row::REmpty | Row::RVar(_) => (row.clone(), false),
            Row::RExtend {
                label,
                ty,
                optional,
                tail,
            } => {
                let ty2 = self.monadic_ty(*ty);
                let (tail2, tail_changed) = self.monadic_row(tail);
                let changed = ty2 != *ty || tail_changed;
                (
                    Row::RExtend {
                        label: label.clone(),
                        ty: ty2,
                        optional: *optional,
                        tail: Box::new(tail2),
                    },
                    changed,
                )
            }
        }
    }

    /// Whether `node` references an effectful callee to reify: a `Var` that is
    /// either a top-level reified function (`fn_set`) or a value whose recorded
    /// type is a function arrow carrying only handled effects (a higher-order
    /// effectful parameter, e.g. `f` in `apply f = f 1`).
    pub(super) fn node_is_eff_fn_ref(
        &self,
        node: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        let TlcExpr::Var(b) = &self.module.expr_arena[node] else {
            return false;
        };
        if fn_set.contains(b) {
            return true;
        }
        let Some(&ty) = self.module.expr_types.get(&node) else {
            return false;
        };
        if !self.fun_spine_has_effect(ty) {
            return false;
        }
        let mut ops = FxHashSet::default();
        self.effect_ops_of(ty, &mut ops);
        ops.is_subset(handle_ops)
    }

    /// Whether `node` is a record-field projection `box.f` yielding an effectful
    /// function value whose operations are all handled — the projection analogue of
    /// `node_is_eff_fn_ref` (which only recognizes `Var` heads). The underlying
    /// field value is reified to `Computation` form via the wrapper-binding rewrite,
    /// so the call site treats `box.f` as an opaque monadic callee.
    pub(super) fn node_is_eff_field_ref(
        &self,
        node: TlcExprId,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        if !matches!(self.module.expr_arena[node], TlcExpr::GetField(..)) {
            return false;
        }
        let Some(&ty) = self.module.expr_types.get(&node) else {
            return false;
        };
        if !self.fun_spine_has_effect(ty) {
            return false;
        }
        let mut ops = FxHashSet::default();
        self.effect_ops_of(ty, &mut ops);
        ops.is_subset(handle_ops)
    }

    /// `#__zt_pure { value = v }`.
    pub(super) fn make_pure(&mut self, v: TlcExprId) -> TlcExprId {
        let pure_payload_ty = self.ctx().pure_payload_ty;
        let comp = self.ctx().comp_ref_ty;
        let rec = self.mk(
            TlcExpr::Record(vec![("value".to_string(), v)]),
            pure_payload_ty,
        );
        self.mk(TlcExpr::Variant(PURE_TAG.to_string(), rec), comp)
    }

    /// Transform a computation expression to a `Computation` value, threading the
    /// pure continuation `k`. `val_ty` is the pure type of the value this
    /// computation yields (recorded `expr_ty` is unreliable — a multi-clause
    /// function body lowers to a `Case` typed as the whole *function* type).
    /// `bind m (\jp. k jp)` — compose a `Computation` value `m` with continuation
    /// `k`, both at the scope carrier.
    pub(super) fn bind_m(&mut self, m: TlcExprId, k: ReifyK) -> TlcExprId {
        let comp = self.ctx().comp_ref_ty;
        let carrier = self.ctx().carrier_ty;
        let cont_ty = self.ctx().cont_ty;
        let bind_binding = self.ctx().bind_binding;
        let bind_ty = self.ctx().bind_ty;
        let jp = self.fresh_binding();
        let jp_var = self.var(jp, carrier);
        let join_body = k(self, jp_var);
        let join_lam = self.mk(TlcExpr::Lam(jp, carrier, join_body), cont_ty);
        let bind_var = self.var(bind_binding, bind_ty);
        let bind_inner_ty = self.fun_ty(cont_ty, comp);
        let app1 = self.mk(TlcExpr::App(bind_var, m), bind_inner_ty);
        self.mk(TlcExpr::App(app1, join_lam), comp)
    }

    /// For a `Case` arm pattern over an effectful cell, add each effectful-field
    /// binder to `comp_binders` and return them (to remove after the arm). The
    /// pattern is `Variant(tag, Record[(field, Bind(b))…])`.
    pub(super) fn mark_arm_comp_binders(&mut self, pat: &TlcPat, cell_id: u32) -> Vec<BindingId> {
        let TlcPat::Variant(tag, inner) = pat else {
            return Vec::new();
        };
        let TlcPat::Record(field_pats) = inner.as_ref() else {
            return Vec::new();
        };
        let eff = self.eff_fields.get(&cell_id).cloned().unwrap_or_default();
        let mut marked = Vec::new();
        for (field, fpat) in field_pats {
            if let TlcPat::Bind(b) = fpat
                && eff.contains(&(tag.clone(), field.clone()))
            {
                self.comp_binders.insert(*b);
                marked.push(*b);
            }
        }
        marked
    }

    /// Whether `id` is a use of a binder bound to a `Computation` value (a
    /// head-field binder of an effectful cell), already in `bind`-able form.
    pub(super) fn node_is_comp_value(&self, id: TlcExprId) -> bool {
        matches!(&self.module.expr_arena[id], TlcExpr::Var(b) if self.comp_binders.contains(b))
    }

    /// Whether the subtree (not descending into lambdas) uses a `Computation`-valued
    /// binder.
    pub(super) fn subtree_has_comp_binder(&self, id: TlcExprId) -> bool {
        if self.comp_binders.is_empty() {
            return false;
        }
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Var(b) if self.comp_binders.contains(b) => return true,
                TlcExpr::Lam(..) => continue,
                _ => {}
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        false
    }

    /// If `ty` is a demand thunk `_ -> Cell` for some effectful cell, that cell's id.
    pub(super) fn demand_thunk_cell(&self, ty: TlcTypeId) -> Option<u32> {
        if let TlcType::Fun(_, b, _) = &self.module.type_arena[ty]
            && let Some(id) = self.cell_identity(*b)
            && self.eff_fields.contains_key(&id)
        {
            return Some(id);
        }
        None
    }

    /// Reify a demand-thunk value `\_. <cell>` into `\_. <Cell'-form cell>`.
    pub(super) fn reify_thunk(&mut self, thunk_id: TlcExprId) -> TlcExprId {
        let TlcExpr::Lam(p, ty, body) = self.module.expr_arena[thunk_id].clone() else {
            return thunk_id;
        };
        let new_body = self.reify_cell_body(body);
        let p_ty = self.monadic_ty(ty);
        let body_ty = self.expr_ty(new_body);
        let lam_ty = self.fun_ty(p_ty, body_ty);
        self.mk(TlcExpr::Lam(p, p_ty, new_body), lam_ty)
    }

    /// Reify a cell-constructing expression `#cons { head = perform …; tail = … }`
    /// into `Cell'` form: an effectful field's `perform` becomes `Computation`-data,
    /// a recursive `tail` thunk is reified, pure fields pass through.
    pub(super) fn reify_cell_body(&mut self, id: TlcExprId) -> TlcExprId {
        let TlcExpr::Variant(tag, payload) = self.module.expr_arena[id].clone() else {
            return id;
        };
        let Some(cell_id) = self.cell_identity(self.expr_ty(id)) else {
            return id;
        };
        let Some(&cprime) = self.cell_prime.get(&cell_id) else {
            return id;
        };
        let TlcExpr::Record(fields) = self.module.expr_arena[payload].clone() else {
            // Payload-less arm (e.g. `#nil`): just retype to `Cell'`.
            return self.mk(TlcExpr::Variant(tag, payload), cprime);
        };
        let eff = self.eff_fields[&cell_id].clone();
        let mut new_fields: Vec<(String, TlcExprId)> = Vec::new();
        let mut field_tys: Vec<(String, TlcTypeId, bool)> = Vec::new();
        for (name, val) in fields {
            let nv = if eff.contains(&(tag.clone(), name.clone())) {
                let vty = self.expr_ty(val);
                self.reify(val, vty, Box::new(|this, v| this.make_pure(v)))
            } else if self.demand_thunk_cell(self.expr_ty(val)).is_some() {
                self.reify_thunk(val)
            } else {
                val
            };
            field_tys.push((name.clone(), self.expr_ty(nv), false));
            new_fields.push((name, nv));
        }
        let payload_ty = self.alloc_type(TlcType::Record(Row::from_record_fields(field_tys)));
        let new_payload = self.mk(TlcExpr::Record(new_fields), payload_ty);
        self.mk(TlcExpr::Variant(tag, new_payload), cprime)
    }

    /// If `arg` is a demand-thunk value for an effectful cell, reify its body;
    /// otherwise leave it.
    pub(super) fn maybe_reify_thunk_arg(&mut self, arg: TlcExprId) -> TlcExprId {
        if self.demand_thunk_cell(self.expr_ty(arg)).is_some() {
            self.reify_thunk(arg)
        } else {
            arg
        }
    }

    /// If `arg` is an eta-expanded partial-application lambda synthesized by
    /// `normalize_undersaturated_eff_args`, reify its (saturated effectful) body to
    /// `Computation` form and rebuild the lambda with monadic parameter/result
    /// types — `reify` itself never descends into ordinary lambda bodies. Mirrors
    /// the reified-function-body commit loop.
    pub(super) fn maybe_reify_eta_fn_arg(&mut self, arg: TlcExprId) -> TlcExprId {
        if !self.eta_fn_args.contains(&arg) {
            return arg;
        }
        let (params, core) = self.peel_lams(arg);
        let core_val_ty = self.expr_ty(core);
        let mut acc = self.reify(core, core_val_ty, Box::new(|this, v| this.make_pure(v)));
        let mut acc_ty = self.ctx().comp_ref_ty;
        for (param, pty) in params.iter().rev() {
            let pty2 = self.monadic_ty(*pty);
            let lam_ty = self.fun_ty(pty2, acc_ty);
            acc = self.mk(TlcExpr::Lam(*param, pty2, acc), lam_ty);
            acc_ty = lam_ty;
        }
        acc
    }

    pub(super) fn reify(&mut self, id: TlcExprId, val_ty: TlcTypeId, k: ReifyK) -> TlcExprId {
        if !self.is_effectful(id) {
            return k(self, id);
        }
        // A `Computation`-valued binder (an effectful cell's head field) is already
        // in monadic form; `bind` it.
        if self.node_is_comp_value(id) {
            return self.bind_m(id, k);
        }
        match self.module.expr_arena[id].clone() {
            TlcExpr::Perform { op, arg } => {
                let (resume_ty, payload_ty) = {
                    let oi = &self.ctx().ops[&op];
                    (oi.resume_ty, oi.payload_ty)
                };
                let comp = self.ctx().comp_ref_ty;
                let r = self.fresh_binding();
                let r_var = self.var(r, resume_ty);
                let resume_body = k(self, r_var);
                let resume_fn_ty = self.fun_ty(resume_ty, comp);
                let resume_lam = self.mk(TlcExpr::Lam(r, resume_ty, resume_body), resume_fn_ty);
                let rec = self.mk(
                    TlcExpr::Record(vec![
                        ("payload".to_string(), arg),
                        ("resume".to_string(), resume_lam),
                    ]),
                    payload_ty,
                );
                self.mk(TlcExpr::Variant(op_tag(&op), rec), comp)
            }
            TlcExpr::Sequence(items) => self.reify_sequence(items, val_ty, k),
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let comp = self.ctx().comp_ref_ty;
                if self.is_effectful(value) {
                    self.reify(
                        value,
                        ty,
                        Box::new(move |this, vv| {
                            let body_c = this.reify(body, val_ty, k);
                            this.mk(
                                TlcExpr::Let {
                                    binding,
                                    ty,
                                    value: vv,
                                    body: body_c,
                                },
                                comp,
                            )
                        }),
                    )
                } else {
                    // A pure value may be an effectful-function *value* (e.g. a
                    // local bound to a reified callee); rewrite the binder type to
                    // its monadic form so later calls to it see `… -> Computation`.
                    let ty2 = self.monadic_ty(ty);
                    let body_c = self.reify(body, val_ty, k);
                    self.mk(
                        TlcExpr::Let {
                            binding,
                            ty: ty2,
                            value,
                            body: body_c,
                        },
                        comp,
                    )
                }
            }
            TlcExpr::Case(scrut, alts) => {
                let comp = self.ctx().comp_ref_ty;
                let case_val_ty = val_ty;
                let jp = self.fresh_binding();
                let jp_var = self.var(jp, case_val_ty);
                let join_body = k(self, jp_var);
                let join_ty = self.fun_ty(case_val_ty, comp);
                let join_lam = self.mk(TlcExpr::Lam(jp, case_val_ty, join_body), join_ty);
                let join_binding = self.fresh_binding();
                // If matching an effectful cell, each arm's head-field binders hold
                // `Computation` values (V3-G4); mark them while reifying that arm.
                let scrut_cell = self
                    .cell_identity(self.expr_ty(scrut))
                    .filter(|id| self.eff_fields.contains_key(id));
                let new_alts: Vec<TlcAlt> = alts
                    .into_iter()
                    .map(|alt| {
                        let marked = match scrut_cell {
                            Some(cell_id) => self.mark_arm_comp_binders(&alt.pat, cell_id),
                            None => Vec::new(),
                        };
                        let body = self.reify(
                            alt.body,
                            val_ty,
                            Box::new(move |this, av| {
                                let jv = this.var(join_binding, join_ty);
                                this.mk(TlcExpr::App(jv, av), comp)
                            }),
                        );
                        for b in marked {
                            self.comp_binders.remove(&b);
                        }
                        TlcAlt {
                            pat: alt.pat,
                            guard: alt.guard,
                            body,
                        }
                    })
                    .collect();
                let new_case = self.mk(TlcExpr::Case(scrut, new_alts), comp);
                self.mk(
                    TlcExpr::Let {
                        binding: join_binding,
                        ty: join_ty,
                        value: join_lam,
                        body: new_case,
                    },
                    comp,
                )
            }
            TlcExpr::Builtin(op, l, r) => {
                // Left-to-right: reify the left operand, then the right, then apply
                // the builtin in the continuation. Pure operands pass straight
                // through `reify`; an effectful operand (e.g. `n + f (n - 1)`)
                // composes through `bind`.
                let lty = self.expr_ty(l);
                let rty = self.expr_ty(r);
                let bty = self.expr_ty(id);
                self.reify(
                    l,
                    lty,
                    Box::new(move |this, lv| {
                        this.reify(
                            r,
                            rty,
                            Box::new(move |this, rv| {
                                let b = this.mk(TlcExpr::Builtin(op, lv, rv), bty);
                                k(this, b)
                            }),
                        )
                    }),
                )
            }
            TlcExpr::App(..) => {
                let fn_set = self.ctx().fn_set.clone();
                let handle_ops = self.ctx().handle_ops.clone();
                let (f_opt, head_node, args) = self
                    .effectful_call(id, &fn_set, &handle_ops)
                    .expect("validated reifiable");
                // Top-level reified fns have a precomputed monadic type; a
                // higher-order effectful parameter or a record-field projection
                // (`box.f`) derives its monadic type from the head node's recorded
                // (effectful-arrow) type.
                let new_f_ty = match f_opt.and_then(|b| self.ctx().fn_new_ty.get(&b).copied()) {
                    Some(ty) => ty,
                    None => {
                        let head_ty = self.expr_ty(head_node);
                        self.monadic_ty(head_ty)
                    }
                };
                // A `Var` head (reified fn or higher-order param) rebuilds as a typed
                // var; a `GetField` head (`box.f`) rebuilds the projection retyped to
                // its monadic form.
                let mut cur = match f_opt {
                    Some(b) => self.var(b, new_f_ty),
                    None => {
                        let node = self.module.expr_arena[head_node].clone();
                        self.mk(node, new_f_ty)
                    }
                };
                let mut cur_ty = new_f_ty;
                for arg in args {
                    // A demand-thunk argument carrying an effectful generator (e.g.
                    // `stream {…}` passed to a consumer) is reified into `Cell'` form.
                    let arg = self.maybe_reify_thunk_arg(arg);
                    // An eta-expanded partial-application lambda argument has its
                    // saturated effectful body reified to monadic form here.
                    let arg = self.maybe_reify_eta_fn_arg(arg);
                    let ret_ty = match &self.module.type_arena[cur_ty] {
                        TlcType::Fun(_, ret, _) => *ret,
                        _ => self.ctx().comp_ref_ty,
                    };
                    cur = self.mk(TlcExpr::App(cur, arg), ret_ty);
                    cur_ty = ret_ty;
                }
                let m = cur;
                self.bind_m(m, k)
            }
            _ => unreachable!("reifiable validated the computation shape"),
        }
    }

    pub(super) fn reify_sequence(
        &mut self,
        items: Vec<TlcExprId>,
        val_ty: TlcTypeId,
        k: ReifyK,
    ) -> TlcExprId {
        let mut iter = items.into_iter();
        let Some(first) = iter.next() else {
            let carrier = self.ctx().carrier_ty;
            let nothing = self.mk(TlcExpr::Lit(Literal::Nothing), carrier);
            return k(self, nothing);
        };
        let rest: Vec<_> = iter.collect();
        if rest.is_empty() {
            return self.reify(first, val_ty, k);
        }
        // A non-last item's value is discarded; reify it with its own recorded
        // type and ignore the result.
        let first_ty = self.expr_ty(first);
        self.reify(
            first,
            first_ty,
            Box::new(move |this, _| this.reify_sequence(rest, val_ty, k)),
        )
    }

    /// Generate the recursive `bind : Computation -> (R -> Computation) -> Computation`.
    pub(super) fn emit_bind_decl(&mut self) {
        let comp = self.ctx().comp_ref_ty;
        let carrier = self.ctx().carrier_ty;
        let cont_ty = self.ctx().cont_ty;
        let bind_binding = self.ctx().bind_binding;
        let bind_ty = self.ctx().bind_ty;

        let m = self.fresh_binding();
        let kb = self.fresh_binding();

        let mut arms: Vec<TlcAlt> = Vec::new();
        // pure arm: #__zt_pure { value = v } => k v
        let v = self.fresh_binding();
        let v_var = self.var(v, carrier);
        let kb_var = self.var(kb, cont_ty);
        let pure_body = self.mk(TlcExpr::App(kb_var, v_var), comp);
        arms.push(TlcAlt {
            pat: pure_pat(v),
            guard: None,
            body: pure_body,
        });

        let op_list: Vec<(String, TlcTypeId, TlcTypeId, TlcTypeId)> = self
            .ctx()
            .ops
            .iter()
            .map(|(name, oi)| (name.clone(), oi.arg_ty, oi.resume_ty, oi.payload_ty))
            .collect();
        for (op, arg_ty, resume_ty, payload_ty) in op_list {
            let p = self.fresh_binding();
            let r = self.fresh_binding();
            let resume_fn_ty = self.fun_ty(resume_ty, comp);
            // resume' = \x. bind (r x) k
            let x = self.fresh_binding();
            let x_var = self.var(x, resume_ty);
            let r_var = self.var(r, resume_fn_ty);
            let rx = self.mk(TlcExpr::App(r_var, x_var), comp);
            let bind_var = self.var(bind_binding, bind_ty);
            let bind_inner_ty = self.fun_ty(cont_ty, comp);
            let b1 = self.mk(TlcExpr::App(bind_var, rx), bind_inner_ty);
            let kb_var2 = self.var(kb, cont_ty);
            let b2 = self.mk(TlcExpr::App(b1, kb_var2), comp);
            let new_resume = self.mk(TlcExpr::Lam(x, resume_ty, b2), resume_fn_ty);
            let p_var = self.var(p, arg_ty);
            let rec = self.mk(
                TlcExpr::Record(vec![
                    ("payload".to_string(), p_var),
                    ("resume".to_string(), new_resume),
                ]),
                payload_ty,
            );
            let op_node = self.mk(TlcExpr::Variant(op_tag(&op), rec), comp);
            arms.push(TlcAlt {
                pat: op_pat(&op, p, r),
                guard: None,
                body: op_node,
            });
        }

        let m_var = self.var(m, comp);
        let case = self.mk(TlcExpr::Case(m_var, arms), comp);
        let inner_lam_ty = self.fun_ty(cont_ty, comp);
        let inner_lam = self.mk(TlcExpr::Lam(kb, cont_ty, case), inner_lam_ty);
        let outer_lam = self.mk(TlcExpr::Lam(m, comp, inner_lam), bind_ty);
        let decl = self.module.decl_arena.alloc(TlcDecl::Value {
            binding: bind_binding,
            ty: bind_ty,
            body: outer_lam,
        });
        self.module.decls.push(decl);
    }

    /// Generate the recursive `run : Computation -> HandleResult` driver and
    /// return its binding.
    pub(super) fn emit_run_decl(
        &mut self,
        value_clause: Option<TlcExprId>,
        ops_clauses: &[TlcHandleClause],
        handle_result_ty: TlcTypeId,
    ) -> BindingId {
        let comp = self.ctx().comp_ref_ty;
        let carrier = self.ctx().carrier_ty;
        let run_binding = self.fresh_binding();
        let run_ty = self.fun_ty(comp, handle_result_ty);
        let m = self.fresh_binding();

        let mut arms: Vec<TlcAlt> = Vec::new();
        // pure arm: value clause applied, or identity.
        let v = self.fresh_binding();
        let pure_body = if let Some(vc) = value_clause {
            let v_var = self.var(v, carrier);
            self.mk(TlcExpr::App(vc, v_var), handle_result_ty)
        } else {
            self.var(v, carrier)
        };
        arms.push(TlcAlt {
            pat: pure_pat(v),
            guard: None,
            body: pure_body,
        });

        let op_list: Vec<(String, TlcTypeId, TlcTypeId, TlcExprId)> = ops_clauses
            .iter()
            .map(|c| {
                let oi = &self.ctx().ops[&c.op];
                (c.op.clone(), oi.arg_ty, oi.resume_ty, oi.handler_body)
            })
            .collect();
        for (op, arg_ty, resume_ty, handler_body) in op_list {
            let p = self.fresh_binding();
            let r = self.fresh_binding();
            let resume_fn_ty = self.fun_ty(resume_ty, comp);
            let handler_rw =
                self.rewrite_resume(handler_body, run_binding, run_ty, r, resume_fn_ty);
            let p_var = self.var(p, arg_ty);
            let body = self.mk(TlcExpr::App(handler_rw, p_var), handle_result_ty);
            arms.push(TlcAlt {
                pat: op_pat(&op, p, r),
                guard: None,
                body,
            });
        }

        let m_var = self.var(m, comp);
        let case = self.mk(TlcExpr::Case(m_var, arms), handle_result_ty);
        let lam = self.mk(TlcExpr::Lam(m, comp, case), run_ty);
        let decl = self.module.decl_arena.alloc(TlcDecl::Value {
            binding: run_binding,
            ty: run_ty,
            body: lam,
        });
        self.module.decls.push(decl);
        run_binding
    }

    /// Rewrite a handler clause body, replacing `resume X` with `run (r X)`.
    /// Subtrees with no `resume` are shared unchanged.
    pub(super) fn rewrite_resume(
        &mut self,
        id: TlcExprId,
        run_binding: BindingId,
        run_ty: TlcTypeId,
        r_binding: BindingId,
        resume_fn_ty: TlcTypeId,
    ) -> TlcExprId {
        if self.no_residual_control(id) {
            return id;
        }
        let ty = self.expr_ty(id);
        let rec = |this: &mut Self, child: TlcExprId| {
            this.rewrite_resume(child, run_binding, run_ty, r_binding, resume_fn_ty)
        };
        match self.module.expr_arena[id].clone() {
            TlcExpr::Resume { value } => {
                let comp = self.ctx().comp_ref_ty;
                let r_var = self.var(r_binding, resume_fn_ty);
                let rx = self.mk(TlcExpr::App(r_var, value), comp);
                let run_var = self.var(run_binding, run_ty);
                self.mk(TlcExpr::App(run_var, rx), ty)
            }
            TlcExpr::Lam(b, lty, body) => {
                let body = rec(self, body);
                self.mk(TlcExpr::Lam(b, lty, body), ty)
            }
            TlcExpr::App(f, a) => {
                let f = rec(self, f);
                let a = rec(self, a);
                self.mk(TlcExpr::App(f, a), ty)
            }
            TlcExpr::Let {
                binding,
                ty: lty,
                value,
                body,
            } => {
                let value = rec(self, value);
                let body = rec(self, body);
                self.mk(
                    TlcExpr::Let {
                        binding,
                        ty: lty,
                        value,
                        body,
                    },
                    ty,
                )
            }
            TlcExpr::Case(scrut, alts) => {
                let scrut = rec(self, scrut);
                let alts = alts
                    .into_iter()
                    .map(|alt| {
                        let guard = alt.guard.map(|g| rec(self, g));
                        let body = rec(self, alt.body);
                        TlcAlt {
                            pat: alt.pat,
                            guard,
                            body,
                        }
                    })
                    .collect();
                self.mk(TlcExpr::Case(scrut, alts), ty)
            }
            TlcExpr::Builtin(op, l, r) => {
                let l = rec(self, l);
                let r = rec(self, r);
                self.mk(TlcExpr::Builtin(op, l, r), ty)
            }
            TlcExpr::Sequence(items) => {
                let items = items.into_iter().map(|i| rec(self, i)).collect();
                self.mk(TlcExpr::Sequence(items), ty)
            }
            TlcExpr::Variant(tag, payload) => {
                let payload = rec(self, payload);
                self.mk(TlcExpr::Variant(tag, payload), ty)
            }
            TlcExpr::Record(fields) => {
                let fields = fields.into_iter().map(|(n, e)| (n, rec(self, e))).collect();
                self.mk(TlcExpr::Record(fields), ty)
            }
            TlcExpr::GetField(base, field) => {
                let base = rec(self, base);
                self.mk(TlcExpr::GetField(base, field), ty)
            }
            TlcExpr::TyApp(e, t) => {
                let e = rec(self, e);
                self.mk(TlcExpr::TyApp(e, t), ty)
            }
            TlcExpr::TyLam(v, kind, body) => {
                let body = rec(self, body);
                self.mk(TlcExpr::TyLam(v, kind, body), ty)
            }
            // No other node kind can contain a `resume` in a validated handler body.
            _ => id,
        }
    }
}
