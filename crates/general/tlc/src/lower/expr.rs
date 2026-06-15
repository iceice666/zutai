use zutai_hir::BindingId;
use zutai_syntax::ast::BinOp;
use zutai_thir::{ThirClause, ThirExprId, ThirExprKind, ThirPatId, ThirPatKind, TypeId};

use crate::ir::{
    BuiltinOp, Literal, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcTupleItem, TlcType,
    TlcTypeId,
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
            ThirExprKind::String(s) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Str(s)), tlc_ty, span)
            }
            ThirExprKind::Atom(s) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Atom(s)), tlc_ty, span)
            }
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
                let tlc_items: Vec<TlcExprId> =
                    items.iter().map(|&e| self.lower_expr(e)).collect();
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
                self.lower_lambda(params, body, tlc_ty, span)
            }
            ThirExprKind::Apply { func, arg, .. } => {
                let func_tlc = self.lower_expr(func);
                let arg_tlc = self.lower_expr(arg);
                self.alloc_expr(TlcExpr::App(func_tlc, arg_tlc), tlc_ty, span)
            }
            ThirExprKind::Import(_) | ThirExprKind::TypeValue(_) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span)
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
        }
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
