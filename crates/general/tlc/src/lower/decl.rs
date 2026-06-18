use zutai_thir::{ThirClause, ThirDeclId, ThirDeclKind};

use crate::ir::{TlcDecl, TlcDeclId};

use super::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_decl(&mut self, id: ThirDeclId) -> TlcDeclId {
        let decl = &self.thir.decl_arena[id];
        let binding = decl.binding;
        let tlc_decl = match decl.kind.clone() {
            ThirDeclKind::TypeAlias { params, ty } => {
                use crate::ir::{Kind, TlcType};
                let mut body = self.lower_type(ty);
                for &p in params.iter().rev() {
                    let tyvar = self.named_tyvar(p);
                    body = self.alloc_type(TlcType::TyLamK(tyvar, Kind::ground(), body));
                }
                TlcDecl::TypeAlias {
                    binding,
                    params,
                    body,
                }
            }
            ThirDeclKind::Value { ty, value } => {
                let scheme = self.thir.poly_schemes.get(&binding).cloned();
                let tlc_ty = self.lower_type(ty);
                let raw_body = self.lower_expr(value);
                let (final_ty, final_body) = if let Some(vars) = scheme {
                    self.wrap_poly(vars, tlc_ty, raw_body)
                } else {
                    (tlc_ty, raw_body)
                };
                TlcDecl::Value {
                    binding,
                    ty: final_ty,
                    body: final_body,
                }
            }
            ThirDeclKind::Function { sig, clauses, .. } => {
                use crate::ir::{Kind, Row, TlcExpr, TlcType};
                let scheme = self.thir.poly_schemes.get(&binding).cloned();
                let explicit = self.fn_explicit_params.get(&binding).cloned();
                let tlc_sig = self.lower_type(sig);

                // Register dict params for bounded type params; collect (dict_binding, dict_ty).
                let mut dict_params = Vec::new();
                if let Some(ref ep) = explicit {
                    for (type_param_binding, constraint_bindings) in ep.iter() {
                        for &cst_binding in constraint_bindings.iter() {
                            let dict_param = self.fresh_synth_binding();
                            let dict_ty = self.alloc_type(TlcType::Record(Row::REmpty));
                            self.active_dict_params
                                .insert((cst_binding.0, type_param_binding.0), dict_param);
                            self.active_dict_types.insert(dict_param, dict_ty);
                            dict_params.push((dict_param, dict_ty));
                        }
                    }
                }

                let raw_body = self.lower_function_clauses(sig, &clauses);

                // Clear active dict params after lowering the body.
                if let Some(ref ep) = explicit {
                    for (type_param_binding, constraint_bindings) in ep.iter() {
                        for &cst_binding in constraint_bindings.iter() {
                            self.active_dict_params
                                .remove(&(cst_binding.0, type_param_binding.0));
                        }
                    }
                }

                // Wrap with dict Lams (reversed so first constraint's dict is outermost).
                let mut current_body = raw_body;
                let mut current_ty = tlc_sig;
                for &(dict_param, dict_ty) in dict_params.iter().rev() {
                    let span = self.spans.get(&current_body).copied().unwrap_or_default();
                    current_ty = self.alloc_type(TlcType::Fun(dict_ty, current_ty, Row::REmpty));
                    current_body = self.alloc_expr(
                        TlcExpr::Lam(dict_param, dict_ty, current_body),
                        current_ty,
                        span,
                    );
                }

                // Wrap with TyLam/ForAll for each explicit type param (reversed → first param outermost).
                if let Some(ref ep) = explicit {
                    for (type_param_binding, _) in ep.iter().rev() {
                        let tyvar = self.named_tyvar(*type_param_binding);
                        let span = self.spans.get(&current_body).copied().unwrap_or_default();
                        current_ty =
                            self.alloc_type(TlcType::ForAll(tyvar, Kind::ground(), current_ty));
                        current_body = self.alloc_expr(
                            TlcExpr::TyLam(tyvar, Kind::ground(), current_body),
                            current_ty,
                            span,
                        );
                    }
                }

                // Wrap with HM poly vars if any remain from inference.
                let (final_ty, final_body) = if let Some(vars) = scheme {
                    self.wrap_poly(vars, current_ty, current_body)
                } else {
                    (current_ty, current_body)
                };

                TlcDecl::Value {
                    binding,
                    ty: final_ty,
                    body: final_body,
                }
            }
            ThirDeclKind::Witness { fields, .. } => {
                use crate::ir::{Row, TlcExpr, TlcType};
                let tlc_fields: Vec<(String, crate::ir::TlcExprId)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_expr(f.value)))
                    .collect();
                let dict_ty = self.alloc_type(TlcType::Record(Row::REmpty));
                let span = zutai_syntax::Span::default();
                let body = self.alloc_expr(TlcExpr::Record(tlc_fields), dict_ty, span);
                TlcDecl::Value {
                    binding,
                    ty: dict_ty,
                    body,
                }
            }
            ThirDeclKind::Constraint { .. } => {
                unreachable!("constraint decls are filtered before TLC lowering")
            }
        };
        self.alloc_decl(tlc_decl)
    }

    pub(super) fn wrap_poly(
        &mut self,
        scheme_vars: Vec<u32>,
        inner_ty: crate::ir::TlcTypeId,
        inner_body: crate::ir::TlcExprId,
    ) -> (crate::ir::TlcTypeId, crate::ir::TlcExprId) {
        use crate::ir::{Kind, TlcExpr, TlcType};

        let mut current_body = inner_body;
        let mut current_ty = inner_ty;

        for &v in scheme_vars.iter().rev() {
            let tyvar = self.inferred_tyvar(v);
            current_ty = self.alloc_type(TlcType::ForAll(tyvar, Kind::ground(), current_ty));
            let span = self.spans.get(&inner_body).copied().unwrap_or_default();
            current_body = self.alloc_expr(
                TlcExpr::TyLam(tyvar, Kind::ground(), current_body),
                current_ty,
                span,
            );
        }
        (current_ty, current_body)
    }

    pub(super) fn lower_function_clauses(
        &mut self,
        sig: zutai_thir::TypeId,
        clauses: &[ThirClause],
    ) -> crate::ir::TlcExprId {
        use crate::ir::{TlcAlt, TlcExpr, TlcPatItem, TlcTupleField, TlcTupleItem, TlcType};

        if clauses.is_empty() {
            let tlc_ty = self.lower_type(sig);
            let span = zutai_syntax::Span::default();
            return self.alloc_expr(TlcExpr::Lit(crate::ir::Literal::Nothing), tlc_ty, span);
        }

        let arity = clauses[0].patterns.len();
        let sig_tlc = self.lower_type(sig);
        let span = zutai_syntax::Span::default();

        if arity == 0 {
            return self.lower_expr(clauses[0].body);
        }

        let arg_bindings: Vec<zutai_hir::BindingId> =
            (0..arity).map(|_| self.fresh_synth_binding()).collect();

        let (scrutinee, _scrutinee_ty) = if arity == 1 {
            let arg = arg_bindings[0];
            let pat_ty = self.thir.pat_arena[clauses[0].patterns[0]].ty;
            let arg_tlc_ty = self.lower_type(pat_ty);
            let var_expr = self.alloc_expr(TlcExpr::Var(arg), arg_tlc_ty, span);
            (var_expr, arg_tlc_ty)
        } else {
            let tuple_items: Vec<TlcTupleItem> = arg_bindings
                .iter()
                .enumerate()
                .map(|(i, &arg)| {
                    let pat_ty = self.thir.pat_arena[clauses[0].patterns[i]].ty;
                    let arg_ty = self.lower_type(pat_ty);
                    let var_expr = self.alloc_expr(TlcExpr::Var(arg), arg_ty, span);
                    TlcTupleItem::Positional(var_expr)
                })
                .collect();
            let tuple_fields: Vec<TlcTupleField> = arg_bindings
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let pat_ty = self.thir.pat_arena[clauses[0].patterns[i]].ty;
                    TlcTupleField::Positional(self.lower_type(pat_ty))
                })
                .collect();
            let tuple_tlc_ty = self.alloc_type(TlcType::Tuple(tuple_fields));
            let tuple_expr = self.alloc_expr(TlcExpr::Tuple(tuple_items), tuple_tlc_ty, span);
            (tuple_expr, tuple_tlc_ty)
        };

        let alts: Vec<TlcAlt> = clauses
            .iter()
            .map(|clause| {
                let pat = if arity == 1 {
                    self.lower_pat(clause.patterns[0])
                } else {
                    let items: Vec<TlcPatItem> = clause
                        .patterns
                        .iter()
                        .map(|&p| TlcPatItem::Positional(self.lower_pat(p)))
                        .collect();
                    crate::ir::TlcPat::Tuple(items)
                };
                let guard = clause.guard.map(|g| self.lower_expr(g));
                let body = self.lower_expr(clause.body);
                TlcAlt { pat, guard, body }
            })
            .collect();

        let case_expr = self.alloc_expr(TlcExpr::Case(scrutinee, alts), sig_tlc, span);

        arg_bindings
            .iter()
            .rev()
            .enumerate()
            .fold(case_expr, |inner, (i, &arg)| {
                let rev_i = arity - 1 - i;
                let pat_ty = self.thir.pat_arena[clauses[0].patterns[rev_i]].ty;
                let arg_tlc_ty = self.lower_type(pat_ty);
                self.alloc_expr(TlcExpr::Lam(arg, arg_tlc_ty, inner), sig_tlc, span)
            })
    }
}
