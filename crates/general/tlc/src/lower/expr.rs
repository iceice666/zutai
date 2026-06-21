use zutai_hir::BindingId;
use zutai_syntax::ast::BinOp;
use zutai_thir::{ThirClause, ThirExprId, ThirExprKind, ThirPatId, ThirPatKind, TypeId};

use crate::ir::{
    BuiltinOp, Literal, PrimTy, TlcAlt, TlcExpr, TlcExprId, TlcHandleClause, TlcPat, TlcPatItem,
    TlcTupleItem, TlcType, TlcTypeId,
};

use super::Lowerer;

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
            ThirExprKind::Lambda { params, body } => self.lower_lambda(params, body, tlc_ty, span),
            ThirExprKind::Apply {
                func,
                arg,
                instantiation,
            } => {
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

                let func_tlc = self.lower_expr(func);
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
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_tlc = self.lower_expr(payload);
                self.alloc_expr(TlcExpr::Variant(tag, payload_tlc), tlc_ty, span)
            }
        }
    }

    fn lower_binding_ref(
        &mut self,
        binding: BindingId,
        tlc_ty: TlcTypeId,
        ref_thir_ty: TypeId,
        span: zutai_syntax::Span,
    ) -> TlcExprId {
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
        span: zutai_syntax::Span,
    ) -> TlcExprId {
        let body_expr = self.lower_expr(body);
        params.iter().rev().fold(body_expr, |inner, &pat_id| {
            let pat = &self.thir.pat_arena[pat_id];
            let (param_binding, param_ty) = match pat.kind {
                ThirPatKind::Bind(b) => (b, self.lower_type(pat.ty)),
                _ => {
                    let fresh = self.fresh_synth_binding();
                    let ty = self.lower_type(pat.ty);
                    (fresh, ty)
                }
            };
            self.alloc_expr(TlcExpr::Lam(param_binding, param_ty, inner), outer_ty, span)
        })
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
