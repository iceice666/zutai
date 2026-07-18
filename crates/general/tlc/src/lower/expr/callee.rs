use rustc_hash::FxHashMap;

use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{TypeId, TypeKind};

use crate::ir::{Literal, TlcExpr, TlcExprId, TlcType, TlcTypeId, UnresolvedDispatch};

use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    /// Build the instantiated callee expression for a reference to `binding` at
    /// `instantiation` — the type-application / dictionary-passing prefix an
    /// `Apply` injects before the value argument. Returns `None` when `binding`
    /// needs no such dispatch (the caller then uses the plain `Var`, possibly with
    /// InferVar poly-scheme `TyApp`s). Shared by the `Apply` callee path and a
    /// standalone `BindingRef` (a polymorphic value used outside callee position).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_instantiated_callee(
        &mut self,
        binding: BindingId,
        callee_thir_ty: TypeId,
        callee_span: Span,
        instantiation: &[TypeId],
        arg_thir_ty: Option<TypeId>,
        tlc_ty: TlcTypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        if instantiation.is_empty() {
            return None;
        }

        if let Some(info) = self.constraint_methods.get(&binding).cloned() {
            // Recover the method sig's exact type-var order (deduped, sorted by
            // binding id) — reproduces THIR's `collect_type_vars`, positionally
            // aligned with `instantiation` even when the method omits a declared
            // param. Fall back to constraint-param + method-params if unavailable.
            let vars: Vec<BindingId> = self
                .method_sig_for(info.constraint, &info.name)
                .map(|sig| self.collect_thir_type_vars(sig))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    let mut v: Vec<BindingId> = Vec::with_capacity(1 + info.method_params.len());
                    v.push(info.constraint_param);
                    v.extend(info.method_params.iter().copied());
                    v.sort_by_key(|b| b.0);
                    v.dedup();
                    v
                });
            let index_of = |b: BindingId| vars.iter().position(|v| *v == b);

            let higher_kinded_dispatch = matches!(
                self.thir.type_param_kinds.get(&info.constraint_param),
                Some(zutai_thir::Kind::Arrow(_, _))
            );
            let dict_inst = index_of(info.constraint_param)
                .and_then(|i| instantiation.get(i).copied())
                .unwrap_or(instantiation[0]);
            let dispatch = if higher_kinded_dispatch {
                self.constraint_dispatch_target(&info, instantiation, arg_thir_ty)
            } else {
                None
            };
            let dispatch_key = dispatch.as_ref().map(|(key, _)| key.clone());
            let extern_inst = dispatch
                .as_ref()
                .map(|(_, target)| *target)
                .unwrap_or(dict_inst);
            let dict_expr = if higher_kinded_dispatch {
                let constraint = self
                    .thir
                    .binding_names
                    .get(info.constraint.0 as usize)
                    .map(String::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.try_extern_dict_by_name(&constraint, extern_inst, callee_span)
                    .unwrap_or_else(|| self.get_dict_expr(info.constraint, dict_inst, callee_span))
            } else {
                self.get_dict_expr(info.constraint, dict_inst, callee_span)
            };
            let method_ty = self.lower_type(callee_thir_ty);
            let method_name = info.name.clone();
            let mut acc = self.alloc_expr(
                TlcExpr::GetField(dict_expr, method_name.clone()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(acc, info.constraint, &method_name);
            let dispatch_key = dispatch_key
                .or_else(|| {
                    self.structural_witness_key(
                        if higher_kinded_dispatch {
                            extern_inst
                        } else {
                            dict_inst
                        },
                        &mut rustc_hash::FxHashSet::default(),
                    )
                })
                .unwrap_or_default();
            // S1 gate: if the dict fell back to `Lit(Nothing)` (no local witness,
            // no derivable instance, not an abstract dict param) yet the operand
            // is a concrete, keyable type, the witness may still be provided by an
            // import — which this pure, import-agnostic lowering cannot see.
            // Record the dispatch so the semantic layer can check it against the
            // merged witness registry and refuse a genuinely-unwitnessed call.
            if matches!(self.expr_arena[dict_expr], TlcExpr::Lit(Literal::Nothing))
                && !dispatch_key.is_empty()
                && !dispatch_key.contains('?')
                && !dispatch_key.contains('@')
            {
                let constraint = self
                    .thir
                    .binding_names
                    .get(info.constraint.0 as usize)
                    .cloned()
                    .unwrap_or_default();
                let target_display = self.thir_type_display(extern_inst);
                self.unresolved_dispatches.push(UnresolvedDispatch {
                    constraint,
                    target_key: dispatch_key.clone(),
                    target_display,
                    span,
                });
            }
            self.dict_dispatch_keys.insert(acc, dispatch_key);
            // Each method-level type param becomes a `TyApp`, in declaration order.
            for &mp in &info.method_params {
                if let Some(i) = index_of(mp)
                    && let Some(&inst_ty) = instantiation.get(i)
                {
                    let ty_arg = self.lower_type(inst_ty);
                    acc = self.alloc_expr(TlcExpr::TyApp(acc, ty_arg), method_ty, span);
                }
            }
            return Some(acc);
        }

        // Explicit-params function: inject TyApp + dict App over the plain Var.
        if let Some(explicit_params) = self.fn_explicit_params.get(&binding).cloned() {
            let fn_var_ty = self.lower_type(callee_thir_ty);
            let mut cur = self.alloc_expr(TlcExpr::Var(binding), fn_var_ty, callee_span);
            for (i, (_, constraint_bindings)) in explicit_params.iter().enumerate() {
                if i < instantiation.len() {
                    let inst_ty_id = instantiation[i];
                    let ty_arg = self.lower_type(inst_ty_id);
                    cur = self.alloc_expr(TlcExpr::TyApp(cur, ty_arg), tlc_ty, span);
                    for &cst_b in constraint_bindings.iter() {
                        let dict = self.get_dict_expr(cst_b, inst_ty_id, span);
                        cur = self.alloc_expr(TlcExpr::App(cur, dict), tlc_ty, span);
                    }
                }
            }
            return Some(cur);
        }

        None
    }
    pub(super) fn constraint_dispatch_target(
        &self,
        info: &crate::lower::witness::ConstraintMethodInfo,
        instantiation: &[TypeId],
        arg_thir_ty: Option<TypeId>,
    ) -> Option<(String, TypeId)> {
        if !matches!(
            self.thir.type_param_kinds.get(&info.constraint_param),
            Some(zutai_thir::Kind::Arrow(_, _))
        ) {
            return None;
        }
        arg_thir_ty?;
        let sig = self.method_sig_for(info.constraint, &info.name)?;
        let vars = self.collect_thir_type_vars(sig);
        let constructor_ty = vars
            .iter()
            .position(|binding| *binding == info.constraint_param)
            .and_then(|index| instantiation.get(index))
            .copied()?;
        let mut subst = FxHashMap::default();
        for (binding, ty) in vars.iter().copied().zip(instantiation.iter().copied()) {
            if binding != info.constraint_param {
                subst.insert(binding, ty);
            }
        }
        let applied_type_arg =
            self.method_constructor_element_arg(sig, info.constraint_param, 0)?;
        let concrete_type_arg = match self.thir.type_arena[applied_type_arg.0 as usize].kind {
            TypeKind::TypeVar(binding) => subst.get(&binding).copied().unwrap_or(applied_type_arg),
            _ => applied_type_arg,
        };
        let mut seen = rustc_hash::FxHashSet::default();
        match self.thir.type_arena[constructor_ty.0 as usize].kind {
            TypeKind::Apply { .. } => {
                let (head, mut args) = self.thir_app_spine(constructor_ty);
                let binding = match self.thir.type_arena[head.0 as usize].kind {
                    TypeKind::Alias(binding) | TypeKind::Con(binding) => binding,
                    _ => return None,
                };
                let name = self.thir.binding_names.get(binding.0 as usize)?;
                let saturated = self
                    .thir
                    .type_arena
                    .iter()
                    .enumerate()
                    .find_map(|(index, ty)| match ty.kind {
                        TypeKind::Apply { func, arg }
                            if self.thir_types_equal(func, constructor_ty)
                                && self.thir_types_equal(arg, concrete_type_arg) =>
                        {
                            Some(TypeId(index as u32))
                        }
                        _ => None,
                    })
                    .unwrap_or(constructor_ty);
                args.push(concrete_type_arg);
                let mut key = name.clone();
                for arg in args {
                    let argument_key = self.structural_witness_key_env(arg, &subst, &mut seen)?;
                    key.push('[');
                    key.push_str(&argument_key);
                    key.push(']');
                }
                Some((key, saturated))
            }
            TypeKind::Con(binding) | TypeKind::Alias(binding) => {
                let name = self.thir.binding_names.get(binding.0 as usize)?;
                let argument_key = self.structural_witness_key(concrete_type_arg, &mut seen)?;
                Some((format!("{name}[{argument_key}]"), constructor_ty))
            }
            _ => None,
        }
    }

    pub(super) fn method_constructor_element_arg(
        &self,
        ty: TypeId,
        constructor_param: BindingId,
        depth: u32,
    ) -> Option<TypeId> {
        if depth > 64 {
            return None;
        }
        if let Some(element) = self.constructor_application_arg(ty, constructor_param) {
            return Some(element);
        }
        let recur =
            |nested| self.method_constructor_element_arg(nested, constructor_param, depth + 1);
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => recur(from).or_else(|| recur(to)),
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Code(inner) => recur(inner),
            TypeKind::Patch { target, .. } | TypeKind::Effect { base: target, .. } => recur(target),
            TypeKind::Record(fields, _) => fields.into_iter().find_map(|field| recur(field.ty)),
            TypeKind::Union(variants, _) => variants
                .into_iter()
                .filter_map(|variant| variant.payload)
                .find_map(recur),
            TypeKind::Tuple(items) => items.into_iter().find_map(|item| match item {
                zutai_thir::TypeTupleItem::Named { ty, .. }
                | zutai_thir::TypeTupleItem::Positional(ty) => recur(ty),
            }),
            TypeKind::AliasApply { args, .. } => args.into_iter().find_map(recur),
            TypeKind::ForAll { body, .. } => recur(body),
            _ => None,
        }
    }

    pub(super) fn constructor_application_arg(
        &self,
        ty: TypeId,
        constructor_param: BindingId,
    ) -> Option<TypeId> {
        let (head, args) = self.thir_app_spine(ty);
        matches!(
            self.thir.type_arena[head.0 as usize].kind,
            TypeKind::TypeVar(binding) if binding == constructor_param
        )
        .then(|| args.last().copied())
        .flatten()
    }

    pub(super) fn lower_binding_ref(
        &mut self,
        binding: BindingId,
        instantiation: &[TypeId],
        tlc_ty: TlcTypeId,
        ref_thir_ty: TypeId,
        span: zutai_syntax::Span,
    ) -> TlcExprId {
        if let Some(op) = self.builtin_effect_op(binding)
            && let TlcType::Fun(arg_ty, result_ty, _) = self.type_arena[tlc_ty].clone()
        {
            let arg_binding = self.fresh_synth_binding();
            let arg_var = self.alloc_expr(TlcExpr::Var(arg_binding), arg_ty, span);
            let perform = self.alloc_expr(
                TlcExpr::Perform {
                    op: op.to_string(),
                    arg: arg_var,
                },
                result_ty,
                span,
            );
            return self.alloc_expr(TlcExpr::Lam(arg_binding, arg_ty, perform), tlc_ty, span);
        }

        // A polymorphic value used outside callee position carries a recorded
        // instantiation (THIR `lower_binding_ref` freshened its `<A>` TypeVars):
        // emit the same TyApp + dict-App prefix the Apply path would.
        if let Some(callee) = self.lower_instantiated_callee(
            binding,
            ref_thir_ty,
            span,
            instantiation,
            None,
            tlc_ty,
            span,
        ) {
            return callee;
        }

        let var_expr = self.alloc_expr(TlcExpr::Var(binding), tlc_ty, span);
        let scheme = self.thir.poly_schemes.get(&binding).cloned();
        let Some(vars) = scheme else {
            return var_expr;
        };
        if vars.is_empty() {
            return var_expr;
        }
        let Some(&decl_thir_ty) = self.decl_thir_types.get(&binding) else {
            return var_expr;
        };
        let instantiation = self.extract_instantiation(&vars, decl_thir_ty, ref_thir_ty);
        instantiation
            .into_iter()
            .fold(var_expr, |expr, (_, ty_arg)| {
                self.alloc_expr(TlcExpr::TyApp(expr, ty_arg), tlc_ty, span)
            })
    }

    pub(super) fn binary_method_type(
        &mut self,
        operand_ty: TypeId,
        result_ty: TlcTypeId,
    ) -> (TlcTypeId, TlcTypeId) {
        use crate::ir::Row;

        let operand_tlc_ty = self.lower_type(operand_ty);
        let after_first = self.alloc_type(TlcType::Fun(operand_tlc_ty, result_ty, Row::REmpty));
        let full = self.alloc_type(TlcType::Fun(operand_tlc_ty, after_first, Row::REmpty));
        (full, after_first)
    }

    pub(super) fn lower_operator_method_call(
        &mut self,
        op_name: &str,
        operand_ty: TypeId,
        lhs_tlc: TlcExprId,
        rhs_tlc: TlcExprId,
        result_ty: TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let guard = self.defining_op_witness.clone();
        for info in self.operator_methods.clone() {
            if info.name != op_name || !info.method_params.is_empty() {
                continue;
            }

            // Self-recursion guard: if we are lowering the body of this very
            // operator method and the call would dispatch back to it, use the
            // builtin instead. This makes `(==) = \a b. a == b` mean "delegate to
            // the primitive" rather than loop forever.
            if let Some((guard_binding, guard_op)) = &guard
                && guard_op == op_name
                && self.concrete_witness_binding(info.constraint, operand_ty)
                    == Some(*guard_binding)
            {
                continue;
            }

            let Some(dict) = self.try_get_dict_expr(info.constraint, operand_ty, span) else {
                continue;
            };
            let (method_ty, after_first_ty) = self.binary_method_type(operand_ty, result_ty);
            let method_name = info.name.clone();
            let method = self.alloc_expr(
                TlcExpr::GetField(dict, method_name.clone()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(method, info.constraint, &method_name);
            let first = self.alloc_expr(TlcExpr::App(method, lhs_tlc), after_first_ty, span);
            return Some(self.alloc_expr(TlcExpr::App(first, rhs_tlc), result_ty, span));
        }

        None
    }
}
