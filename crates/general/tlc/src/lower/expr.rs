use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;
use zutai_syntax::{Span, ast::BinOp};
use zutai_thir::{
    ThirClause, ThirExprId, ThirExprKind, ThirPatId, ThirPatKind, TypeId, TypeKind, TypeRecordField,
};

use crate::ir::{
    BuiltinOp, Literal, PrimTy, TlcAlt, TlcExpr, TlcExprId, TlcHandleClause, TlcPat, TlcPatItem,
    TlcTupleItem, TlcType, TlcTypeId,
};

use super::Lowerer;
type ForallLambdaDict = (BindingId, BindingId, TlcTypeId);
type ForallLambdaLayer = Vec<(BindingId, Vec<ForallLambdaDict>)>;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_expr(&mut self, id: ThirExprId) -> TlcExprId {
        let expr = &self.thir.expr_arena[id];
        let span = expr.span;
        let thir_ty = expr.ty;
        let tlc_ty = self.lower_type(thir_ty);

        match expr.kind.clone() {
            ThirExprKind::Error => self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span),
            ThirExprKind::True => self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), tlc_ty, span),
            ThirExprKind::False => {
                self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), tlc_ty, span)
            }
            ThirExprKind::Integer(n) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Int(n)), tlc_ty, span)
            }
            ThirExprKind::Float(f) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Float(f)), tlc_ty, span)
            }
            ThirExprKind::Posit(literal) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Posit(literal)), tlc_ty, span)
            }
            ThirExprKind::String(s) => self.alloc_expr(TlcExpr::Lit(Literal::Str(s)), tlc_ty, span),
            ThirExprKind::Atom(s) => self.alloc_expr(TlcExpr::Lit(Literal::Atom(s)), tlc_ty, span),
            ThirExprKind::BindingRef(binding) => {
                self.lower_binding_ref(binding, tlc_ty, thir_ty, span)
            }
            ThirExprKind::Record(fields) => {
                let tlc_fields: Vec<(String, TlcExprId)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_expr(f.value)))
                    .collect();
                self.alloc_expr(TlcExpr::Record(tlc_fields), tlc_ty, span)
            }
            ThirExprKind::RecordUpdate { receiver, fields } => {
                let receiver = self.lower_expr(receiver);
                let tlc_fields: Vec<(String, TlcExprId)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_expr(f.value)))
                    .collect();
                self.alloc_expr(
                    TlcExpr::RecordUpdate {
                        receiver,
                        fields: tlc_fields,
                    },
                    tlc_ty,
                    span,
                )
            }
            ThirExprKind::Tuple(items) => {
                let tlc_items: Vec<TlcTupleItem> = items
                    .iter()
                    .map(|item| match item {
                        zutai_thir::ThirTupleItem::Named { name, value, .. } => {
                            TlcTupleItem::Named {
                                name: name.clone(),
                                value: self.lower_expr(*value),
                            }
                        }
                        zutai_thir::ThirTupleItem::Positional(v) => {
                            TlcTupleItem::Positional(self.lower_expr(*v))
                        }
                    })
                    .collect();
                self.alloc_expr(TlcExpr::Tuple(tlc_items), tlc_ty, span)
            }
            ThirExprKind::List(items) => {
                let tlc_items: Vec<TlcExprId> = items.iter().map(|&e| self.lower_expr(e)).collect();
                self.alloc_expr(TlcExpr::List(tlc_items), tlc_ty, span)
            }
            ThirExprKind::Access { receiver, field } => {
                let recv = self.lower_expr(receiver);
                self.alloc_expr(TlcExpr::GetField(recv, field), tlc_ty, span)
            }
            ThirExprKind::OptionalAccess { receiver, field } => {
                let recv = self.lower_expr(receiver);
                self.alloc_expr(TlcExpr::GetField(recv, field), tlc_ty, span)
            }
            ThirExprKind::Binary { op, lhs, rhs } => {
                let lhs_tlc = self.lower_expr(lhs);
                let rhs_tlc = self.lower_expr(rhs);
                let lhs_ty = self.thir.expr_arena[lhs].ty;

                if op == BinOp::Ne {
                    if let Some(expr) = self
                        .lower_operator_method_call("!=", lhs_ty, lhs_tlc, rhs_tlc, tlc_ty, span)
                    {
                        return expr;
                    }
                    if let Some(eq_expr) = self
                        .lower_operator_method_call("==", lhs_ty, lhs_tlc, rhs_tlc, tlc_ty, span)
                    {
                        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
                        let true_lit =
                            self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), bool_ty, span);
                        return self.alloc_expr(
                            TlcExpr::Builtin(BuiltinOp::Ne, eq_expr, true_lit),
                            tlc_ty,
                            span,
                        );
                    }
                } else if let Some(op_name) = binop_operator_method_name(op)
                    && let Some(expr) = self
                        .lower_operator_method_call(op_name, lhs_ty, lhs_tlc, rhs_tlc, tlc_ty, span)
                {
                    return expr;
                }

                let builtin = binop_to_builtin(op);
                self.alloc_expr(TlcExpr::Builtin(builtin, lhs_tlc, rhs_tlc), tlc_ty, span)
            }
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let scrutinee = self.lower_expr(cond);
                let then_expr = self.lower_expr(then_branch);
                let else_expr = self.lower_expr(else_branch);
                let alts = vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: then_expr,
                    },
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(false)),
                        guard: None,
                        body: else_expr,
                    },
                ];
                self.alloc_expr(TlcExpr::Case(scrutinee, alts), tlc_ty, span)
            }
            ThirExprKind::Block { bindings, result } => {
                let tail = self.lower_expr(result);
                bindings.iter().rev().fold(tail, |body, local| {
                    let value = self.lower_expr(local.value);
                    let ty = self.lower_type(local.ty);
                    self.alloc_expr(
                        TlcExpr::Let {
                            binding: local.binding,
                            ty,
                            value,
                            body,
                        },
                        tlc_ty,
                        local.span,
                    )
                })
            }
            ThirExprKind::Match { scrutinee, arms } => {
                let scrut = self.lower_expr(scrutinee);
                let alts: Vec<TlcAlt> = arms
                    .iter()
                    .map(|arm| self.lower_clause_as_alt(arm))
                    .collect();
                self.alloc_expr(TlcExpr::Case(scrut, alts), tlc_ty, span)
            }
            ThirExprKind::Lambda { params, body } => {
                self.lower_lambda(params, body, tlc_ty, thir_ty, span)
            }
            ThirExprKind::Apply {
                func,
                arg,
                instantiation,
                forall_instantiation,
            } => {
                if let Some(expr) = self.lower_overlay_full_apply(func, arg, thir_ty, span) {
                    return expr;
                }

                // Extract func binding info without holding a borrow while calling &mut self.
                let func_binding_info = {
                    let fe = &self.thir.expr_arena[func];
                    if let ThirExprKind::BindingRef(b) = fe.kind {
                        Some((b, fe.ty, fe.span))
                    } else {
                        None
                    }
                };

                if let Some((binding, func_thir_ty, func_span)) = func_binding_info {
                    if self.is_builtin_print_binding(binding) {
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(
                            TlcExpr::Perform {
                                op: "io.print".to_string(),
                                arg: arg_tlc,
                            },
                            tlc_ty,
                            span,
                        );
                    }

                    // Constraint method call: dispatch via GetField on the active dict param.
                    if let Some(info) = self.constraint_methods.get(&binding).cloned()
                        && !instantiation.is_empty()
                    {
                        // Recover the method sig's exact type-var order (deduped,
                        // sorted by binding id) — this reproduces THIR's
                        // `collect_type_vars` and is positionally aligned with
                        // `instantiation`, even when the method omits some declared
                        // param. Fall back to constraint-param + method-params if the
                        // sig is unavailable.
                        let vars: Vec<BindingId> = self
                            .method_sig_for(info.constraint, &info.name)
                            .map(|sig| self.collect_thir_type_vars(sig))
                            .filter(|v| !v.is_empty())
                            .unwrap_or_else(|| {
                                let mut v: Vec<BindingId> =
                                    Vec::with_capacity(1 + info.method_params.len());
                                v.push(info.constraint_param);
                                v.extend(info.method_params.iter().copied());
                                v.sort_by_key(|b| b.0);
                                v.dedup();
                                v
                            });
                        let index_of = |b: BindingId| vars.iter().position(|v| *v == b);

                        // The constraint param's instantiation selects the dict.
                        let dict_inst = index_of(info.constraint_param)
                            .and_then(|i| instantiation.get(i).copied())
                            .unwrap_or(instantiation[0]);
                        let dict_expr = self.get_dict_expr(info.constraint, dict_inst, func_span);
                        let method_ty = self.lower_type(func_thir_ty);
                        let method_name = info.name.clone();
                        let mut acc = self.alloc_expr(
                            TlcExpr::GetField(dict_expr, method_name.clone()),
                            method_ty,
                            span,
                        );
                        self.register_dict_field_slot(acc, info.constraint, &method_name);
                        // Each method-level type param becomes a `TyApp`, in
                        // declaration order, so the dict's `TyLam`-wrapped method is
                        // instantiated at the call site's inferred type arguments.
                        for &mp in &info.method_params {
                            if let Some(i) = index_of(mp)
                                && let Some(&inst_ty) = instantiation.get(i)
                            {
                                let ty_arg = self.lower_type(inst_ty);
                                acc = self.alloc_expr(TlcExpr::TyApp(acc, ty_arg), method_ty, span);
                            }
                        }
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(TlcExpr::App(acc, arg_tlc), tlc_ty, span);
                    }

                    // Explicit-params function call: inject TyApp + dict App before value arg.
                    if let Some(explicit_params) = self.fn_explicit_params.get(&binding).cloned()
                        && !instantiation.is_empty()
                    {
                        let fn_var_ty = self.lower_type(func_thir_ty);
                        let mut cur = self.alloc_expr(TlcExpr::Var(binding), fn_var_ty, func_span);
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
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(TlcExpr::App(cur, arg_tlc), tlc_ty, span);
                    }
                }

                let mut func_tlc = self.lower_expr(func);
                if !forall_instantiation.is_empty() {
                    let func_thir_ty_id = self.thir.expr_arena[func].ty;
                    if let TypeKind::ForAll {
                        params,
                        param_bounds,
                        ..
                    } = self.thir.type_arena[func_thir_ty_id.0 as usize]
                        .kind
                        .clone()
                    {
                        for (i, (&_param, &inst_ty)) in
                            params.iter().zip(forall_instantiation.iter()).enumerate()
                        {
                            let ty_arg = self.lower_type(inst_ty);
                            let cur_ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                            func_tlc =
                                self.alloc_expr(TlcExpr::TyApp(func_tlc, ty_arg), cur_ty, span);
                            for &bound in &param_bounds[i] {
                                let dict = self.get_dict_expr(bound, inst_ty, span);
                                let after_dict_ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                                func_tlc = self.alloc_expr(
                                    TlcExpr::App(func_tlc, dict),
                                    after_dict_ty,
                                    span,
                                );
                            }
                        }
                    }
                }
                let arg_tlc = self.lower_expr(arg);
                self.alloc_expr(TlcExpr::App(func_tlc, arg_tlc), tlc_ty, span)
            }
            ThirExprKind::Perform { op, arg } => {
                let arg = self.lower_expr(arg);
                self.alloc_expr(TlcExpr::Perform { op, arg }, tlc_ty, span)
            }
            ThirExprKind::Resume { value } => {
                let value = self.lower_expr(value);
                self.alloc_expr(TlcExpr::Resume { value }, tlc_ty, span)
            }
            ThirExprKind::Handle { expr, value, ops } => {
                let expr = self.lower_expr(expr);
                let value = value.map(|value| self.lower_expr(value));
                let ops = ops
                    .into_iter()
                    .map(|clause| TlcHandleClause {
                        op: clause.op,
                        body: self.lower_expr(clause.body),
                    })
                    .collect();
                self.alloc_expr(TlcExpr::Handle { expr, value, ops }, tlc_ty, span)
            }
            ThirExprKind::Sequence(items) => {
                let items = items
                    .into_iter()
                    .map(|item| self.lower_expr(item))
                    .collect();
                self.alloc_expr(TlcExpr::Sequence(items), tlc_ty, span)
            }
            ThirExprKind::Import(source) => self.alloc_expr(TlcExpr::Import(source), tlc_ty, span),
            ThirExprKind::TypeValue(_) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span)
            }
            ThirExprKind::WitnessReflect { constraint, target } => {
                let dict = constraint
                    .map(|constraint| self.get_dict_expr(constraint, target, span))
                    .unwrap_or_else(|| {
                        self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span)
                    });
                self.expr_types.insert(dict, tlc_ty);
                dict
            }
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_tlc = self.lower_expr(payload);
                self.alloc_expr(TlcExpr::Variant(tag, payload_tlc), tlc_ty, span)
            }
        }
    }

    fn lower_overlay_full_apply(
        &mut self,
        func: ThirExprId,
        base: ThirExprId,
        target: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        let (patch, deep) = self.overlay_full_apply_parts(func)?;
        let base = self.lower_expr(base);
        let patch_fields = self.record_literal_fields(patch, &mut FxHashSet::default())?;
        self.lower_overlay_record(
            base,
            patch_fields,
            target,
            &FxHashMap::default(),
            deep,
            span,
        )
    }

    fn overlay_full_apply_parts(&self, func: ThirExprId) -> Option<(ThirExprId, bool)> {
        let ThirExprKind::Apply {
            func: builtin,
            arg: patch,
            ..
        } = &self.thir.expr_arena[func].kind
        else {
            return None;
        };
        let ThirExprKind::BindingRef(binding) = &self.thir.expr_arena[*builtin].kind else {
            return None;
        };
        match self.builtin_overlay_name(*binding)? {
            "overlay" => Some((*patch, false)),
            "overlayDeep" => Some((*patch, true)),
            _ => None,
        }
    }

    fn builtin_overlay_name(&self, binding: BindingId) -> Option<&str> {
        let name = self.thir.binding_names.get(binding.0 as usize)?.as_str();
        if name != "overlay" && name != "overlayDeep" {
            return None;
        }
        let first = self
            .thir
            .binding_names
            .iter()
            .position(|candidate| candidate == name)?;
        (first == binding.0 as usize).then_some(name)
    }

    fn record_literal_fields(
        &self,
        expr: ThirExprId,
        seen: &mut FxHashSet<BindingId>,
    ) -> Option<Vec<(String, ThirExprId)>> {
        match &self.thir.expr_arena[expr].kind {
            ThirExprKind::Record(fields) => Some(
                fields
                    .iter()
                    .map(|field| (field.name.clone(), field.value))
                    .collect(),
            ),
            ThirExprKind::BindingRef(binding) => {
                if !seen.insert(*binding) {
                    return None;
                }
                let value = self.value_decl_expr(*binding)?;
                let fields = self.record_literal_fields(value, seen);
                seen.remove(binding);
                fields
            }
            _ => None,
        }
    }

    fn value_decl_expr(&self, binding: BindingId) -> Option<ThirExprId> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding != binding {
                return None;
            }
            match &decl.kind {
                zutai_thir::ThirDeclKind::Value { value, .. } => Some(*value),
                _ => None,
            }
        })
    }

    fn lower_overlay_record(
        &mut self,
        base: TlcExprId,
        patch_fields: Vec<(String, ThirExprId)>,
        target: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
        deep: bool,
        span: Span,
    ) -> Option<TlcExprId> {
        let (target_fields, _tail, env) = self.record_shape_with_subst(target, subst)?;
        let result_ty = self.lower_type_with_subst(target, subst);
        let mut updates = Vec::with_capacity(patch_fields.len());
        for (name, patch_value) in patch_fields {
            let field = target_fields.iter().find(|field| field.name == name)?;
            let value = self.lower_overlay_field(base, patch_value, field, &env, deep, span)?;
            updates.push((name, value));
        }
        if updates.is_empty() {
            return Some(base);
        }
        Some(self.alloc_expr(
            TlcExpr::RecordUpdate {
                receiver: base,
                fields: updates,
            },
            result_ty,
            span,
        ))
    }

    fn lower_overlay_field(
        &mut self,
        base: TlcExprId,
        patch_value: ThirExprId,
        field: &TypeRecordField,
        subst: &FxHashMap<BindingId, TypeId>,
        deep: bool,
        span: Span,
    ) -> Option<TlcExprId> {
        if deep && self.record_shape_with_subst(field.ty, subst).is_some() {
            if field.optional {
                return None;
            }
            let field_ty = self.lower_type_with_subst(field.ty, subst);
            let base_field =
                self.alloc_expr(TlcExpr::GetField(base, field.name.clone()), field_ty, span);
            let patch_fields =
                self.record_literal_fields(patch_value, &mut FxHashSet::default())?;
            return self.lower_overlay_record(
                base_field,
                patch_fields,
                field.ty,
                subst,
                true,
                span,
            );
        }
        Some(self.lower_expr(patch_value))
    }

    fn lower_binding_ref(
        &mut self,
        binding: BindingId,
        tlc_ty: TlcTypeId,
        ref_thir_ty: TypeId,
        span: zutai_syntax::Span,
    ) -> TlcExprId {
        if self.is_builtin_print_binding(binding)
            && let TlcType::Fun(arg_ty, result_ty, _) = self.type_arena[tlc_ty].clone()
        {
            let arg_binding = self.fresh_synth_binding();
            let arg_var = self.alloc_expr(TlcExpr::Var(arg_binding), arg_ty, span);
            let perform = self.alloc_expr(
                TlcExpr::Perform {
                    op: "io.print".to_string(),
                    arg: arg_var,
                },
                result_ty,
                span,
            );
            return self.alloc_expr(TlcExpr::Lam(arg_binding, arg_ty, perform), tlc_ty, span);
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

    fn is_builtin_print_binding(&self, binding: BindingId) -> bool {
        self.thir
            .binding_names
            .get(binding.0 as usize)
            .is_some_and(|name| name == "print")
            && self
                .thir
                .binding_kinds
                .get(binding.0 as usize)
                .is_some_and(|kind| *kind == zutai_hir::BindingKind::BuiltinValue)
    }

    fn lower_clause_as_alt(&mut self, clause: &ThirClause) -> TlcAlt {
        let pat = if clause.patterns.is_empty() {
            TlcPat::Wildcard
        } else {
            self.lower_pat(clause.patterns[0])
        };
        let guard = clause.guard.map(|g| self.lower_expr(g));
        let body = self.lower_expr(clause.body);
        TlcAlt { pat, guard, body }
    }

    pub(super) fn lower_pat(&mut self, id: ThirPatId) -> TlcPat {
        match self.thir.pat_arena[id].kind.clone() {
            ThirPatKind::Error | ThirPatKind::Wildcard => TlcPat::Wildcard,
            ThirPatKind::Bind(b) => TlcPat::Bind(b),
            ThirPatKind::True => TlcPat::Lit(Literal::Bool(true)),
            ThirPatKind::False => TlcPat::Lit(Literal::Bool(false)),
            ThirPatKind::Integer(n) => TlcPat::Lit(Literal::Int(n)),
            ThirPatKind::Float(f) => TlcPat::Lit(Literal::Float(f)),
            ThirPatKind::Posit(literal) => TlcPat::Lit(Literal::Posit(literal)),
            ThirPatKind::String(s) => TlcPat::Lit(Literal::Str(s)),
            ThirPatKind::Atom(s) => TlcPat::Atom(s),
            ThirPatKind::Tuple(items) => {
                let tlc_items: Vec<TlcPatItem> = items
                    .iter()
                    .map(|item| match item {
                        zutai_thir::ThirTuplePatItem::Named { name, pattern, .. } => {
                            TlcPatItem::Named {
                                name: name.clone(),
                                pat: self.lower_pat(*pattern),
                            }
                        }
                        zutai_thir::ThirTuplePatItem::Positional(p) => {
                            TlcPatItem::Positional(self.lower_pat(*p))
                        }
                    })
                    .collect();
                TlcPat::Tuple(tlc_items)
            }
            ThirPatKind::Record(fields) => {
                let tlc_fields: Vec<(String, TlcPat)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_pat(f.pattern)))
                    .collect();
                TlcPat::Record(tlc_fields)
            }
            ThirPatKind::TaggedValue { tag, payload } => {
                let inner = if payload.is_empty() {
                    // Bare atom arm: `#dev` — no payload to bind.
                    Box::new(TlcPat::Wildcard)
                } else {
                    // Tagged-payload arm: `(#circle, radius: r)` — match as record.
                    let fields: Vec<(String, TlcPat)> = payload
                        .iter()
                        .map(|f| (f.name.clone(), self.lower_pat(f.pattern)))
                        .collect();
                    Box::new(TlcPat::Record(fields))
                };
                TlcPat::Variant(tag, inner)
            }
        }
    }

    fn binary_method_type(
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

    fn lower_operator_method_call(
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

    pub(super) fn lower_lambda(
        &mut self,
        params: Vec<ThirPatId>,
        body: ThirExprId,
        outer_ty: TlcTypeId,
        thir_ty: TypeId,
        span: zutai_syntax::Span,
    ) -> TlcExprId {
        let forall_layers = self.prepare_forall_lambda_layers(thir_ty);
        let body_expr = self.lower_expr(body);
        for layer in &forall_layers {
            for &(param, ref dicts) in layer {
                for &(bound, _, _) in dicts {
                    self.active_dict_params.remove(&(bound.0, param.0));
                }
            }
        }

        // Peel one function arrow per parameter so each curried lambda layer is
        // typed `param -> rest` instead of sharing the full `outer_ty`. Sharing it
        // gives inner lambdas a param type from the wrong position and fails the
        // Dataflow structural validator when parameters have distinct types.
        let arity = params.len();
        let mut layer_tys = Vec::with_capacity(arity);
        let mut cur = outer_ty;
        for _ in 0..arity {
            layer_tys.push(cur);
            cur = match self.type_arena[cur].clone() {
                TlcType::Fun(_, result, _) => result,
                _ => cur,
            };
        }

        let mut expr = body_expr;
        for (i, &pat_id) in params.iter().enumerate().rev() {
            let pat = &self.thir.pat_arena[pat_id];
            let (param_binding, param_ty) = match pat.kind {
                ThirPatKind::Bind(b) => (b, self.lower_type(pat.ty)),
                _ => {
                    let fresh = self.fresh_synth_binding();
                    let ty = self.lower_type(pat.ty);
                    (fresh, ty)
                }
            };
            expr = self.alloc_expr(
                TlcExpr::Lam(param_binding, param_ty, expr),
                layer_tys[i],
                span,
            );
        }

        for layer in forall_layers.iter().rev() {
            for &(param, ref dicts) in layer.iter().rev() {
                for &(_, dict_param, dict_ty) in dicts.iter().rev() {
                    expr = self.alloc_expr(TlcExpr::Lam(dict_param, dict_ty, expr), outer_ty, span);
                }
                let tyvar = self.named_tyvar(param);
                let kind = self.kind_for_type_param(param);
                expr = self.alloc_expr(TlcExpr::TyLam(tyvar, kind, expr), outer_ty, span);
            }
        }
        expr
    }

    fn prepare_forall_lambda_layers(&mut self, thir_ty: TypeId) -> Vec<ForallLambdaLayer> {
        let mut layers = Vec::new();
        let mut current = thir_ty;
        loop {
            match self.thir.type_arena[current.0 as usize].kind.clone() {
                TypeKind::ForAll {
                    params,
                    param_bounds,
                    body,
                } => {
                    let mut layer = Vec::with_capacity(params.len());
                    for (param, bounds) in params.into_iter().zip(param_bounds) {
                        let mut dicts = Vec::with_capacity(bounds.len());
                        for bound in bounds {
                            let dict_param = self.fresh_synth_binding();
                            let dict_ty = self.alloc_type(TlcType::Record(crate::ir::Row::REmpty));
                            self.active_dict_params
                                .insert((bound.0, param.0), dict_param);
                            self.active_dict_types.insert(dict_param, dict_ty);
                            dicts.push((bound, dict_param, dict_ty));
                        }
                        layer.push((param, dicts));
                    }
                    layers.push(layer);
                    current = body;
                }
                _ => return layers,
            }
        }
    }
}

fn binop_operator_method_name(op: BinOp) -> Option<&'static str> {
    match op {
        BinOp::Eq => Some("=="),
        BinOp::Ne => Some("!="),
        BinOp::Lt => Some("<"),
        BinOp::Le => Some("<="),
        BinOp::Gt => Some(">"),
        BinOp::Ge => Some(">="),
        _ => None,
    }
}

fn binop_to_builtin(op: BinOp) -> BuiltinOp {
    match op {
        BinOp::Add => BuiltinOp::Add,
        BinOp::Sub => BuiltinOp::Sub,
        BinOp::Mul => BuiltinOp::Mul,
        BinOp::Div => BuiltinOp::Div,
        BinOp::Eq => BuiltinOp::Eq,
        BinOp::Ne => BuiltinOp::Ne,
        BinOp::Lt => BuiltinOp::Lt,
        BinOp::Le => BuiltinOp::Le,
        BinOp::Gt => BuiltinOp::Gt,
        BinOp::Ge => BuiltinOp::Ge,
        BinOp::And => BuiltinOp::And,
        BinOp::Or => BuiltinOp::Or,
        BinOp::Coalesce => BuiltinOp::Coalesce,
    }
}
