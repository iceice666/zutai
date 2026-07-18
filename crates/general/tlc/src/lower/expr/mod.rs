use rustc_hash::FxHashMap;

use zutai_hir::BindingId;
use zutai_syntax::{Span, ast::BinOp};
use zutai_thir::{
    ThirClause, ThirDeclKind, ThirExprId, ThirExprKind, ThirPatId, ThirPatKind, TypeId, TypeKind,
};

use crate::ir::{
    BuiltinOp, Literal, PrimTy, TlcAlt, TlcExpr, TlcExprId, TlcHandleClause, TlcPat, TlcPatItem,
    TlcTupleField, TlcTupleItem, TlcType, TlcTypeId,
};

use crate::lower::Lowerer;
type ForallLambdaDict = (BindingId, BindingId, TlcTypeId);
type ForallLambdaLayer = Vec<(BindingId, Vec<ForallLambdaDict>)>;
const CODE_EXPANSION_FUEL: u16 = 256;

#[derive(Clone)]
struct CodeExpansion {
    value: ThirExprId,
    frames: Vec<FxHashMap<BindingId, ThirExprId>>,
}

#[derive(Clone)]
struct CodeClosure {
    params: Vec<ThirPatId>,
    body: ThirExprId,
    frames: Vec<FxHashMap<BindingId, ThirExprId>>,
}

enum CompileTimeValue {
    Code(CodeExpansion),
    Closure(CodeClosure),
    Expr(CodeExpansion),
}

#[derive(Clone, Copy)]
enum WrapperKind {
    Optional,
    Maybe,
}

impl WrapperKind {
    fn absent_tag(self) -> &'static str {
        match self {
            WrapperKind::Optional => "none",
            WrapperKind::Maybe => "absent",
        }
    }

    fn present_tag(self) -> &'static str {
        match self {
            WrapperKind::Optional => "some",
            WrapperKind::Maybe => "present",
        }
    }
}

mod callee;
mod compile_time;
mod overlay;

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
            ThirExprKind::BindingRef {
                binding,
                instantiation,
                ..
            } => {
                if self.builtin_overlay_deep(binding).is_some()
                    && matches!(
                        self.thir.type_arena[thir_ty.0 as usize].kind,
                        TypeKind::Function { .. }
                    )
                {
                    self.lower_overlay_function(binding, thir_ty, span)
                } else if let Some(replacement) = self.code_substitution(binding) {
                    self.lower_expr(replacement)
                } else {
                    self.lower_binding_ref(binding, &instantiation, tlc_ty, thir_ty, span)
                }
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
            ThirExprKind::ListAppend { left, right } => {
                let left = self.lower_expr(left);
                let right = self.lower_expr(right);
                self.alloc_expr(TlcExpr::ListAppend(left, right), tlc_ty, span)
            }
            ThirExprKind::Access { receiver, field } => {
                let recv = self.lower_expr(receiver);
                self.alloc_expr(TlcExpr::GetField(recv, field), tlc_ty, span)
            }
            ThirExprKind::OptionalAccess { receiver, field } => {
                let recv = self.lower_expr(receiver);
                self.lower_optional_access(receiver, recv, field, tlc_ty, thir_ty, span)
            }
            ThirExprKind::Binary { op, lhs, rhs } => {
                let lhs_tlc = self.lower_expr(lhs);
                let rhs_tlc = self.lower_expr(rhs);
                let lhs_ty = self.thir.expr_arena[lhs].ty;

                if matches!(op, BinOp::And | BinOp::Or) {
                    return self.lower_logical_short_circuit(op, lhs_tlc, rhs_tlc, tlc_ty, span);
                }
                if op == BinOp::Coalesce {
                    return self.lower_coalesce(lhs_tlc, rhs_tlc, tlc_ty, span);
                }

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
                if let Some(expr) = self.lower_overlay_apply(func, arg, thir_ty, span) {
                    return expr;
                }

                // Extract func binding info without holding a borrow while calling &mut self.
                let func_binding_info = {
                    let fe = &self.thir.expr_arena[func];
                    if let ThirExprKind::BindingRef { binding: b, .. } = fe.kind {
                        Some((b, fe.ty, fe.span))
                    } else {
                        None
                    }
                };

                if let Some((binding, func_thir_ty, func_span)) = func_binding_info {
                    let arg_thir_ty = Some(self.thir.expr_arena[arg].ty);
                    if let Some(op) = self.builtin_effect_op(binding) {
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(
                            TlcExpr::Perform {
                                op: op.to_string(),
                                arg: arg_tlc,
                            },
                            tlc_ty,
                            span,
                        );
                    }

                    if self.thir.binding_names[binding.0 as usize] == "decode"
                        && let Some(&target) = instantiation.first()
                        && let Some(&sig) = self.decl_thir_types.get(&binding)
                        && let TypeKind::Function { from, to } =
                            self.thir.type_arena[sig.0 as usize].kind
                        && let Some((param, _)) = self
                            .fn_explicit_params
                            .get(&binding)
                            .and_then(|params| params.first())
                            .cloned()
                    {
                        let arg_tlc = self.lower_expr(arg);
                        let subst: FxHashMap<BindingId, TypeId> =
                            [(param, target)].into_iter().collect();
                        let data_ty = self.lower_expanded_type_with_subst(from, &subst);
                        let result_shape_ty = self.lower_expanded_type_with_subst(to, &subst);
                        self.expr_types.insert(arg_tlc, data_ty);
                        let target_ty = self.lower_type(target);
                        if let Some(decoded) = self.derive_from_data_value(
                            target,
                            target_ty,
                            arg_tlc,
                            data_ty,
                            result_shape_ty,
                            span,
                        ) {
                            self.expr_types.insert(decoded, tlc_ty);
                            return decoded;
                        }
                    }

                    if let Some(info) = self
                        .constraint_methods
                        .get(&binding)
                        .filter(|info| info.name == "fromData")
                        .cloned()
                        && let Some(&target) = instantiation.first()
                        && let Some(sig) = self.method_sig_for(info.constraint, "fromData")
                        && let TypeKind::Function { from, to } =
                            self.thir.type_arena[sig.0 as usize].kind
                    {
                        let arg_tlc = self.lower_expr(arg);
                        let subst: FxHashMap<BindingId, TypeId> =
                            [(info.constraint_param, target)].into_iter().collect();
                        let data_ty = self.lower_expanded_type_with_subst(from, &subst);
                        let result_shape_ty = self.lower_expanded_type_with_subst(to, &subst);
                        self.expr_types.insert(arg_tlc, data_ty);
                        let target_ty = self.lower_type(target);
                        if let Some(decoded) = self.derive_from_data_value(
                            target,
                            target_ty,
                            arg_tlc,
                            data_ty,
                            result_shape_ty,
                            span,
                        ) {
                            self.expr_types.insert(decoded, tlc_ty);
                            return decoded;
                        }
                    }

                    // Constraint-method / explicit-params dispatch: build the
                    // instantiated callee (TyApps + dict Apps) then apply the arg.
                    if let Some(callee) = self.lower_instantiated_callee(
                        binding,
                        func_thir_ty,
                        func_span,
                        &instantiation,
                        arg_thir_ty,
                        tlc_ty,
                        span,
                    ) {
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(TlcExpr::App(callee, arg_tlc), tlc_ty, span);
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
            ThirExprKind::Handle {
                expr,
                value,
                finally,
                ops,
            } => {
                let expr = self.lower_expr(expr);
                let value = value.map(|value| self.lower_expr(value));
                let finally = finally.map(|finally| self.lower_expr(finally));
                let ops = ops
                    .into_iter()
                    .map(|clause| TlcHandleClause {
                        op: clause.op,
                        body: self.lower_expr(clause.body),
                    })
                    .collect();
                self.alloc_expr(
                    TlcExpr::Handle {
                        expr,
                        value,
                        finally,
                        ops,
                    },
                    tlc_ty,
                    span,
                )
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
            ThirExprKind::Splice(code) => match self.resolve_code_expr(code) {
                Some(expansion) => self.lower_code_expansion(expansion),
                None => self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span),
            },
            // A standalone Code value is compile-time-only. Semantic validation
            // rejects escape; keep TLC construction total for error recovery.
            ThirExprKind::Quote(_) => self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span),
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_tlc = self.lower_expr(payload);
                self.alloc_expr(TlcExpr::Variant(tag, payload_tlc), tlc_ty, span)
            }
        }
    }

    pub(super) fn lower_quoted_recipe_record(
        &mut self,
        constraint: BindingId,
        span: Span,
    ) -> Option<Vec<(String, TlcExprId)>> {
        let (body, definition) = self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint {
                    recipe: Some(recipe),
                    ..
                } = &decl.kind
            {
                Some((recipe.body, decl.span))
            } else {
                None
            }
        })?;
        // Distinguish fuel exhaustion (a source diagnostic) from an ordinary
        // non-reducible recipe (silent fall-through to structural synthesizers).
        self.recipe_fuel_exhausted.set(false);
        let reduced = self.eval_compile_time(body, Vec::new(), CODE_EXPANSION_FUEL);
        if self.recipe_fuel_exhausted.replace(false) {
            let constraint_name = self.thir.binding_names[constraint.0 as usize].clone();
            self.diagnostics.push(zutai_thir::ThirDiagnostic {
                kind: zutai_thir::ThirDiagnosticKind::DeriveRecipeFuelExhausted {
                    constraint: constraint_name,
                    definition,
                },
                span,
            });
            return None;
        }
        // A `Code`-typed recipe body promises a witness record. If reduction
        // produced one, lower it. Otherwise, distinguish two cases by the body's
        // static type: a `Code`-typed body that failed to reduce (e.g. it stalls
        // on arithmetic or a comparison the pure reducer does not evaluate) is a
        // hard error — refuse rather than fall through to a structural witness the
        // recipe never described. A non-`Code` body (the `<T> => \x. x`
        // method-name form) legitimately falls through to the structural
        // synthesizers, so it returns `None` silently.
        let record = match reduced {
            Some(CompileTimeValue::Code(expansion)) => {
                match self.thir.expr_arena[expansion.value].kind.clone() {
                    ThirExprKind::Record(fields) => Some((fields, expansion.frames)),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some((fields, frames)) = record else {
            let body_is_code = matches!(
                self.thir.type_arena[self.thir.expr_arena[body].ty.0 as usize].kind,
                TypeKind::Code(_)
            );
            if body_is_code {
                let constraint_name = self.thir.binding_names[constraint.0 as usize].clone();
                self.diagnostics.push(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::DeriveRecipeIrreducible {
                        constraint: constraint_name,
                        definition,
                    },
                    span,
                });
            }
            return None;
        };
        let saved = std::mem::replace(&mut self.code_frames, frames);
        let lowered = fields
            .into_iter()
            .map(|field| (field.name, self.lower_expr(field.value)))
            .collect();
        self.code_frames = saved;
        Some(lowered)
    }

    fn builtin_effect_op(&self, binding: BindingId) -> Option<&'static str> {
        if !self
            .thir
            .binding_kinds
            .get(binding.0 as usize)
            .is_some_and(|kind| *kind == zutai_hir::BindingKind::BuiltinValue)
        {
            return None;
        }
        match self.thir.binding_names.get(binding.0 as usize)?.as_str() {
            "print" => Some("io.print"),
            "loadZti" => Some("load.zti"),
            "loadZt" => Some("load.zt"),
            _ => None,
        }
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

    fn lower_logical_short_circuit(
        &mut self,
        op: BinOp,
        lhs: TlcExprId,
        rhs: TlcExprId,
        ty: TlcTypeId,
        span: Span,
    ) -> TlcExprId {
        let true_lit = self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), ty, span);
        let false_lit = self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), ty, span);
        let alts = match op {
            BinOp::And => vec![
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(true)),
                    guard: None,
                    body: rhs,
                },
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(false)),
                    guard: None,
                    body: false_lit,
                },
            ],
            BinOp::Or => vec![
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(true)),
                    guard: None,
                    body: true_lit,
                },
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(false)),
                    guard: None,
                    body: rhs,
                },
            ],
            _ => unreachable!("only logical operators short-circuit"),
        };
        self.alloc_expr(TlcExpr::Case(lhs, alts), ty, span)
    }

    fn lower_coalesce(
        &mut self,
        value: TlcExprId,
        fallback: TlcExprId,
        ty: TlcTypeId,
        span: Span,
    ) -> TlcExprId {
        let some_binding = self.fresh_synth_binding();
        let some_value = self.alloc_expr(TlcExpr::Var(some_binding), ty, span);
        let present_binding = self.fresh_synth_binding();
        let present_value = self.alloc_expr(TlcExpr::Var(present_binding), ty, span);
        self.alloc_expr(
            TlcExpr::Case(
                value,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom("none".to_string()),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Atom("absent".to_string()),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant("none".to_string(), Box::new(TlcPat::Wildcard)),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant("absent".to_string(), Box::new(TlcPat::Wildcard)),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "some".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "0".to_string(),
                                TlcPat::Bind(some_binding),
                            )])),
                        ),
                        guard: None,
                        body: some_value,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "present".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "0".to_string(),
                                TlcPat::Bind(present_binding),
                            )])),
                        ),
                        guard: None,
                        body: present_value,
                    },
                ],
            ),
            ty,
            span,
        )
    }

    fn lower_optional_access(
        &mut self,
        receiver: ThirExprId,
        recv: TlcExprId,
        field: String,
        ty: TlcTypeId,
        result_thir_ty: TypeId,
        span: Span,
    ) -> TlcExprId {
        let receiver_ty = self.thir.expr_arena[receiver].ty;
        let Some((wrapper, inner_thir_ty)) = self.thir_wrapper_inner(receiver_ty) else {
            return self.alloc_expr(TlcExpr::GetField(recv, field), ty, span);
        };
        let Some((_, result_inner_ty)) = self.thir_wrapper_inner(result_thir_ty) else {
            return self.alloc_expr(TlcExpr::GetField(recv, field), ty, span);
        };

        let inner_tlc_ty = self.lower_type(inner_thir_ty);
        let result_inner_tlc_ty = self.lower_type(result_inner_ty);
        let bind = self.fresh_synth_binding();
        let bound_record = self.alloc_expr(TlcExpr::Var(bind), inner_tlc_ty, span);
        let projected = self.alloc_expr(
            TlcExpr::GetField(bound_record, field),
            result_inner_tlc_ty,
            span,
        );
        let tuple_ty = self.alloc_type(TlcType::Tuple(vec![TlcTupleField::Positional(
            result_inner_tlc_ty,
        )]));
        let payload = self.alloc_expr(
            TlcExpr::Tuple(vec![TlcTupleItem::Positional(projected)]),
            tuple_ty,
            span,
        );
        let present_body = self.alloc_expr(
            TlcExpr::Variant(wrapper.present_tag().to_string(), payload),
            ty,
            span,
        );
        let absent_body = self.alloc_expr(
            TlcExpr::Lit(Literal::Atom(wrapper.absent_tag().to_string())),
            ty,
            span,
        );

        self.alloc_expr(
            TlcExpr::Case(
                recv,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom(wrapper.absent_tag().to_string()),
                        guard: None,
                        body: absent_body,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            wrapper.absent_tag().to_string(),
                            Box::new(TlcPat::Wildcard),
                        ),
                        guard: None,
                        body: absent_body,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            wrapper.present_tag().to_string(),
                            Box::new(TlcPat::Record(vec![("0".to_string(), TlcPat::Bind(bind))])),
                        ),
                        guard: None,
                        body: present_body,
                    },
                ],
            ),
            ty,
            span,
        )
    }

    fn thir_wrapper_inner(&self, ty: TypeId) -> Option<(WrapperKind, TypeId)> {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::Optional(inner) => Some((WrapperKind::Optional, inner)),
            TypeKind::Maybe(inner) => Some((WrapperKind::Maybe, inner)),
            TypeKind::Alias(binding) => self
                .type_alias_body(binding)
                .and_then(|body| self.thir_wrapper_inner(body)),
            _ => None,
        }
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
            ThirPatKind::ListNil => TlcPat::ListNil,
            ThirPatKind::ListCons { head, tail } => TlcPat::ListCons(
                Box::new(self.lower_pat(head)),
                Box::new(self.lower_pat(tail)),
            ),
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

        // Peel `outer_ty` so each abstraction layer is typed with its own slice
        // (`∀a. rest`, `dict -> rest`, or `param -> rest`) instead of sharing the
        // full `outer_ty`. Sharing it gives an inner layer a type from the wrong
        // position, which the Dataflow structural validator rejects with an ICE.
        // This mirrors the per-layer wrapping in `lower/decl.rs`.
        //
        // `outer_ty` for a polymorphic lambda is `∀a…. dict… -> value-fun`: the
        // forall/dict prefix wraps the value-function type. Record each
        // forall-layer binder's own type while advancing `cur` past the prefix,
        // then peel the value arrows from the value-function type that remains.
        let mut cur = outer_ty;
        let mut forall_layer_tys: Vec<Vec<(TlcTypeId, Vec<TlcTypeId>)>> =
            Vec::with_capacity(forall_layers.len());
        for layer in &forall_layers {
            let mut layer_tys = Vec::with_capacity(layer.len());
            for (_param, dicts) in layer {
                let tylam_ty = cur;
                cur = match self.type_arena[cur].clone() {
                    TlcType::ForAll(_, _, body) => body,
                    _ => cur,
                };
                let mut dict_tys = Vec::with_capacity(dicts.len());
                for _ in dicts {
                    dict_tys.push(cur);
                    cur = match self.type_arena[cur].clone() {
                        TlcType::Fun(_, result, _) => result,
                        _ => cur,
                    };
                }
                layer_tys.push((tylam_ty, dict_tys));
            }
            forall_layer_tys.push(layer_tys);
        }

        // `cur` is now the value-function type; peel one arrow per value parameter
        // so each curried value lambda is typed `param -> rest`.
        let arity = params.len();
        let mut value_layer_tys = Vec::with_capacity(arity);
        for _ in 0..arity {
            value_layer_tys.push(cur);
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
                value_layer_tys[i],
                span,
            );
        }

        for (layer, layer_tys) in forall_layers.iter().zip(forall_layer_tys.iter()).rev() {
            for (&(param, ref dicts), (tylam_ty, dict_tys)) in
                layer.iter().zip(layer_tys.iter()).rev()
            {
                for (&(_, dict_param, dict_ty), &lam_ty) in dicts.iter().zip(dict_tys.iter()).rev()
                {
                    expr = self.alloc_expr(TlcExpr::Lam(dict_param, dict_ty, expr), lam_ty, span);
                }
                let tyvar = self.named_tyvar(param);
                let kind = self.kind_for_type_param(param);
                expr = self.alloc_expr(TlcExpr::TyLam(tyvar, kind, expr), *tylam_ty, span);
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
        BinOp::Rem => BuiltinOp::Rem,
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
