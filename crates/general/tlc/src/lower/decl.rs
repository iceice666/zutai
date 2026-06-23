use zutai_hir::BindingId;
use zutai_thir::{ThirClause, ThirConstraintMethod, ThirDeclId, ThirDeclKind};

use crate::ir::{Row, TlcDecl, TlcDeclId, TlcExpr, TlcExprId};

use super::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_decl(&mut self, id: ThirDeclId) -> TlcDeclId {
        let decl = &self.thir.decl_arena[id];
        let binding = decl.binding;
        let tlc_decl = match decl.kind.clone() {
            ThirDeclKind::TypeAlias { params, ty } => {
                use crate::ir::TlcType;
                let mut body = self.lower_type(ty);
                for &p in params.iter().rev() {
                    let tyvar = self.named_tyvar(p);
                    body =
                        self.alloc_type(TlcType::TyLamK(tyvar, self.kind_for_type_param(p), body));
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
                    let row_params = self.sig_row_param_bindings(sig);
                    for (type_param_binding, _) in ep.iter().rev() {
                        let tyvar = self.named_tyvar(*type_param_binding);
                        // A `<Rest>` parameter used as a row tail quantifies with
                        // `Kind::Row`; an ordinary type parameter uses its solved THIR kind.
                        let kind = if row_params.contains(&type_param_binding.0) {
                            Kind::Row(Box::new(self.kind_for_type_param(*type_param_binding)))
                        } else {
                            self.kind_for_type_param(*type_param_binding)
                        };
                        let span = self.spans.get(&current_body).copied().unwrap_or_default();
                        current_ty =
                            self.alloc_type(TlcType::ForAll(tyvar, kind.clone(), current_ty));
                        current_body = self.alloc_expr(
                            TlcExpr::TyLam(tyvar, kind, current_body),
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
            ThirDeclKind::Witness {
                constraint,
                target,
                params,
                param_bounds,
                fields,
                derive,
            } => {
                use crate::ir::TlcType;

                // Register a dict param for each bound on each witness param, so
                // constraint-method calls inside the field bodies dispatch through
                // the witness's own dictionaries (e.g. `eq` on element type `A`).
                // `binders` records the abstraction order (outer → inner): a
                // `TyLam` per param followed by a `Lam` per bound, matching the
                // `TyApp`/`App` order in `resolve_conditional_witness`.
                enum Binder {
                    Ty(BindingId, crate::ir::TlcTypeVar),
                    Dict(BindingId, crate::ir::TlcTypeId),
                }
                let mut binders: Vec<Binder> = Vec::new();
                let mut registered: Vec<(u32, u32)> = Vec::new();
                for (param, bounds) in params.iter().zip(param_bounds.iter()) {
                    let tyvar = self.named_tyvar(*param);
                    binders.push(Binder::Ty(*param, tyvar));
                    for &cst_b in bounds.iter() {
                        let dict_param = self.fresh_synth_binding();
                        let dict_ty = self.alloc_type(TlcType::Record(Row::REmpty));
                        self.active_dict_params
                            .insert((cst_b.0, param.0), dict_param);
                        self.active_dict_types.insert(dict_param, dict_ty);
                        registered.push((cst_b.0, param.0));
                        binders.push(Binder::Dict(dict_param, dict_ty));
                    }
                }

                // Concrete witness: expose the dict record being built to its own
                // field/default bodies under the constraint's type param, so a
                // sibling-method reference (e.g. a default `neq` whose body calls
                // `eq`) dispatches through this very dict. The witness value is
                // bound at `binding` in the top-level letrec, so `Var(binding)`
                // resolves to the finished record by call time.
                let self_dict: Option<(u32, u32)> = if params.is_empty() {
                    constraint
                        .and_then(|c| self.constraint_target_param(c).map(|p| (c, p)))
                        .map(|(c, p)| {
                            let dict_ty = self.alloc_type(TlcType::Record(Row::REmpty));
                            self.active_dict_params.insert((c.0, p.0), binding);
                            self.active_dict_types.insert(binding, dict_ty);
                            (c.0, p.0)
                        })
                } else {
                    None
                };

                let tlc_fields: Vec<(String, TlcExprId)> = if derive {
                    constraint
                        .map(|constraint| self.synthesize_derive_fields(constraint, target))
                        .unwrap_or_default()
                } else {
                    let span = zutai_syntax::Span::default();
                    let mut out: Vec<(String, TlcExprId)> = Vec::with_capacity(fields.len());
                    for f in fields {
                        self.defining_op_witness = Some((binding, f.name.clone()));
                        let mut value_expr = self.lower_expr(f.value);
                        self.defining_op_witness = None;
                        // A polymorphic method's body is wrapped in `TyLam` per
                        // method param (outer = first declared) so the dict field
                        // has a `ForAll` type the call site instantiates via `TyApp`.
                        if let Some(c) = constraint {
                            // Wrap one TyLam per method param that actually
                            // appears in the signature — the same set the call
                            // site applies via TyApp (expr.rs `index_of` filter),
                            // keeping ForAll-binder and TyApp counts in lockstep
                            // even for a method that declares an unused param.
                            let sig_vars = self
                                .method_sig_for(c, &f.name)
                                .map(|s| self.collect_thir_type_vars(s))
                                .unwrap_or_default();
                            let mparams: Vec<BindingId> = self
                                .method_params_for(c, &f.name)
                                .into_iter()
                                .filter(|p| sig_vars.contains(p))
                                .collect();
                            for &mp in mparams.iter().rev() {
                                let tyvar = self.named_tyvar(mp);
                                let body_ty = self.expr_types[&value_expr];
                                let kind = self.kind_for_type_param(mp);
                                let forall_ty =
                                    self.alloc_type(TlcType::ForAll(tyvar, kind.clone(), body_ty));
                                value_expr = self.alloc_expr(
                                    TlcExpr::TyLam(tyvar, kind, value_expr),
                                    forall_ty,
                                    span,
                                );
                            }
                        }
                        out.push((f.name.clone(), value_expr));
                    }
                    if let Some(c) = constraint {
                        for method in self.default_methods_for(c) {
                            if out.iter().any(|(name, _)| name == &method.name) {
                                continue;
                            }
                            let Some(default) = method.default.as_ref() else {
                                continue;
                            };
                            self.defining_op_witness = Some((binding, method.name.clone()));
                            let mut value_expr = self.lower_function_clauses(method.sig, default);
                            self.defining_op_witness = None;
                            let sig_vars = self.collect_thir_type_vars(method.sig);
                            let mparams: Vec<BindingId> = method
                                .params
                                .iter()
                                .copied()
                                .filter(|p| sig_vars.contains(p))
                                .collect();
                            for &mp in mparams.iter().rev() {
                                let tyvar = self.named_tyvar(mp);
                                let body_ty = self.expr_types[&value_expr];
                                let kind = self.kind_for_type_param(mp);
                                let forall_ty =
                                    self.alloc_type(TlcType::ForAll(tyvar, kind.clone(), body_ty));
                                value_expr = self.alloc_expr(
                                    TlcExpr::TyLam(tyvar, kind, value_expr),
                                    forall_ty,
                                    span,
                                );
                            }
                            out.push((method.name, value_expr));
                        }
                    }
                    out
                };

                if let Some(key) = self_dict {
                    self.active_dict_params.remove(&key);
                }
                for key in registered {
                    self.active_dict_params.remove(&key);
                }

                let dict_ty = self.alloc_type(TlcType::Record(Row::REmpty));
                let span = zutai_syntax::Span::default();
                let mut body = self.alloc_expr(TlcExpr::Record(tlc_fields), dict_ty, span);
                let mut body_ty = dict_ty;
                for binder in binders.into_iter().rev() {
                    match binder {
                        Binder::Dict(dict_param, d_ty) => {
                            body_ty = self.alloc_type(TlcType::Fun(d_ty, body_ty, Row::REmpty));
                            body = self.alloc_expr(
                                TlcExpr::Lam(dict_param, d_ty, body),
                                body_ty,
                                span,
                            );
                        }
                        Binder::Ty(param, tyvar) => {
                            let kind = self.kind_for_type_param(param);
                            body_ty =
                                self.alloc_type(TlcType::ForAll(tyvar, kind.clone(), body_ty));
                            body =
                                self.alloc_expr(TlcExpr::TyLam(tyvar, kind, body), body_ty, span);
                        }
                    }
                }
                TlcDecl::Value {
                    binding,
                    ty: body_ty,
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
        use crate::ir::{TlcExpr, TlcType};

        let mut current_body = inner_body;
        let mut current_ty = inner_ty;

        for &v in scheme_vars.iter().rev() {
            let tyvar = self.inferred_tyvar(v);
            let kind = self.kind_for_infer_var(v);
            current_ty = self.alloc_type(TlcType::ForAll(tyvar, kind.clone(), current_ty));
            let span = self.spans.get(&inner_body).copied().unwrap_or_default();
            current_body =
                self.alloc_expr(TlcExpr::TyLam(tyvar, kind, current_body), current_ty, span);
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

        // Each curried lambda layer needs its own progressively-peeled function
        // type: the outermost lambda is `sig_tlc` (`A -> B -> R`), the next its
        // result (`B -> R`), and so on. Reusing `sig_tlc` for every layer hands an
        // inner lambda a param type from the wrong position, which the Dataflow
        // structural validator rejects whenever two parameters have distinct types.
        let mut layer_tys = Vec::with_capacity(arity);
        let mut cur = sig_tlc;
        for _ in 0..arity {
            layer_tys.push(cur);
            cur = match self.type_arena[cur].clone() {
                TlcType::Fun(_, result, _) => result,
                _ => cur,
            };
        }

        arg_bindings
            .iter()
            .rev()
            .enumerate()
            .fold(case_expr, |inner, (i, &arg)| {
                let rev_i = arity - 1 - i;
                let pat_ty = self.thir.pat_arena[clauses[0].patterns[rev_i]].ty;
                let arg_tlc_ty = self.lower_type(pat_ty);
                self.alloc_expr(TlcExpr::Lam(arg, arg_tlc_ty, inner), layer_tys[rev_i], span)
            })
    }
    /// The method-level type params of constraint method `name`, by scanning the
    /// THIR constraint decl. Empty if the method has none or is not found.
    fn method_params_for(&self, constraint: BindingId, name: &str) -> Vec<BindingId> {
        self.thir
            .decls
            .iter()
            .find_map(|&decl_id| {
                let decl = &self.thir.decl_arena[decl_id];
                if decl.binding == constraint
                    && let ThirDeclKind::Constraint { methods, .. } = &decl.kind
                {
                    return methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(|m| m.params.clone());
                }
                None
            })
            .unwrap_or_default()
    }

    fn default_methods_for(&self, constraint: BindingId) -> Vec<ThirConstraintMethod> {
        self.thir
            .decls
            .iter()
            .find_map(|&decl_id| {
                let decl = &self.thir.decl_arena[decl_id];
                if decl.binding == constraint
                    && let ThirDeclKind::Constraint { methods, .. } = &decl.kind
                {
                    return Some(
                        methods
                            .iter()
                            .filter(|method| method.default.is_some())
                            .cloned()
                            .collect(),
                    );
                }
                None
            })
            .unwrap_or_default()
    }

    /// The constraint's target type parameter (`@A`) — its first declared param.
    /// Used to register a concrete witness's own dict under that param so default
    /// and sibling method bodies dispatch through it.
    fn constraint_target_param(&self, constraint: BindingId) -> Option<BindingId> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint { params, .. } = &decl.kind
            {
                return params.first().copied();
            }
            None
        })
    }
}
