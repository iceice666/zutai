use super::*;

struct OverlayApply {
    builtin: HirExprId,
    base: HirExprId,
    deep: bool,
}

impl<'hir> Lowerer<'hir> {
    pub(super) fn lower_apply_expr(
        &mut self,
        id: HirExprId,
        func: HirExprId,
        arg: HirExprId,
        span: Span,
    ) -> ThirExprId {
        if let Some(overlay) = self.overlay_full_apply(func) {
            return self.lower_overlay_apply_expr(id, func, arg, span, overlay);
        }
        let func = self.infer_expr(func);
        let func_ty = self.expr(func).ty;
        let Some((from, to)) = self.function_input_output(func_ty, span) else {
            let found = self.type_name(func_ty);
            if !matches!(
                self.type_arena[self.resolve(func_ty).0 as usize].kind,
                TypeKind::Error
            ) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedFunction { found },
                    span,
                });
            }
            let arg = self.infer_expr(arg);
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.error_type,
                kind: ThirExprKind::Apply {
                    func,
                    arg,
                    instantiation: Vec::new(),
                },
                span,
            });
        };

        // If the function signature contains TypeVars (explicit polymorphism),
        // instantiate them with fresh InferVars so each call site is independent.
        let type_vars: Vec<_> = {
            let mut v = self.collect_type_vars(from);
            let mut from_to = self.collect_type_vars(to);
            from_to.retain(|b| !v.contains(b));
            v.extend(from_to);
            v.sort_by_key(|b| b.0);
            v.dedup();
            v
        };
        let (from, to, instantiation) = if type_vars.is_empty() {
            (from, to, Vec::new())
        } else {
            let mut subst = HashMap::new();
            let mut inst = Vec::new();
            for var in &type_vars {
                let fresh = self.fresh_infer_var(span);
                subst.insert(*var, fresh);
                inst.push(fresh);
            }
            let new_from = self.instantiate_type_vars(from, &subst);
            let new_to = self.instantiate_type_vars(to, &subst);
            (new_from, new_to, inst)
        };

        // Instantiate rigid row variables (`<Rest>`) with fresh flexible row
        // variables so each call site solves the row independently. The same
        // fresh variable is shared across `from` and `to`, preserving the tail.
        let row_params: Vec<_> = {
            let mut v = self.collect_row_params(from);
            let mut from_to = self.collect_row_params(to);
            from_to.retain(|b| !v.contains(b));
            v.extend(from_to);
            v.sort_by_key(|b| b.0);
            v.dedup();
            v
        };
        let (from, to) = if row_params.is_empty() {
            (from, to)
        } else {
            let mut row_subst = HashMap::new();
            for var in &row_params {
                row_subst.insert(*var, self.fresh_row_var());
            }
            let new_from = self.instantiate_row_params(from, &row_subst);
            let new_to = self.instantiate_row_params(to, &row_subst);
            (new_from, new_to)
        };

        let arg = self.check_expr(arg, from);
        // Resolve the return type: InferVars introduced for TypeVars may now be
        // solved after checking the argument. If the fully-applied call returns
        // an effectful computation, discharge that row into the current ambient
        // or handler layer and expose the pure base type to the caller.
        let result_ty = self.resolve(to);
        let effect_ty = self.resolve_alias(to, &mut HashSet::new(), span);
        let result_ty = match self.type_arena[effect_ty.0 as usize].kind.clone() {
            TypeKind::Effect { base, row } => {
                self.discharge_row(&row, span);
                base
            }
            _ => result_ty,
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty: result_ty,
            kind: ThirExprKind::Apply {
                func,
                arg,
                instantiation,
            },
            span,
        })
    }

    fn overlay_full_apply(&self, func: HirExprId) -> Option<OverlayApply> {
        let HirExprKind::Apply {
            func: builtin,
            arg: base,
        } = self.hir_expr(func).kind.clone()
        else {
            return None;
        };
        let HirExprKind::BindingRef(binding) = self.hir_expr(builtin).kind.clone() else {
            return None;
        };
        let binding_info = &self.hir.bindings[binding.0 as usize];
        if binding_info.kind != BindingKind::BuiltinValue {
            return None;
        }
        match binding_info.name.as_str() {
            "overlay" => Some(OverlayApply {
                builtin,
                base,
                deep: false,
            }),
            "overlayDeep" => Some(OverlayApply {
                builtin,
                base,
                deep: true,
            }),
            _ => None,
        }
    }

    fn lower_overlay_apply_expr(
        &mut self,
        id: HirExprId,
        inner_source: HirExprId,
        patch: HirExprId,
        span: Span,
        overlay: OverlayApply,
    ) -> ThirExprId {
        let OverlayApply {
            builtin,
            base,
            deep,
        } = overlay;
        let HirExprKind::BindingRef(binding) = self.hir_expr(builtin).kind.clone() else {
            return self.error_expr(id, span);
        };
        let builtin_ref = self.lower_binding_ref(builtin, binding, self.hir_expr(builtin).span);
        let base_expr = self.infer_expr(base);
        let target = self.expr(base_expr).ty;
        let patch_ty = self.patch_type(target, deep, span);
        let patch_expr = self.check_expr(patch, patch_ty);
        let inner_ty = self.alloc_type(Type {
            kind: TypeKind::Function {
                from: patch_ty,
                to: target,
            },
            span,
        });
        let inner = self.alloc_expr(ThirExpr {
            source: inner_source,
            ty: inner_ty,
            kind: ThirExprKind::Apply {
                func: builtin_ref,
                arg: base_expr,
                instantiation: Vec::new(),
            },
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty: target,
            kind: ThirExprKind::Apply {
                func: inner,
                arg: patch_expr,
                instantiation: Vec::new(),
            },
            span,
        })
    }

    pub(super) fn lower_binding_ref(
        &mut self,
        id: HirExprId,
        binding: BindingId,
        span: Span,
    ) -> ThirExprId {
        let binding_info = &self.hir.bindings[binding.0 as usize];
        if matches!(
            binding_info.kind,
            BindingKind::BuiltinType | BindingKind::TopType
        ) {
            let value = if binding_info.kind == BindingKind::TopType {
                self.alias_type(binding, span)
            } else {
                self.builtin_type_by_name(&binding_info.name, span)
                    .unwrap_or(self.error_type)
            };
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.type_type,
                kind: ThirExprKind::TypeValue(value),
                span,
            });
        }

        match self.value_types.get(&binding).copied() {
            Some(ty) => {
                let ty = match self.poly_schemes.get(&binding).cloned() {
                    Some(scheme) => {
                        let subst: HashMap<u32, TypeId> = scheme
                            .into_iter()
                            .map(|v| (v, self.fresh_infer_var(span)))
                            .collect();
                        self.instantiate_infer_vars(ty, &subst)
                    }
                    None => ty,
                };
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::BindingRef(binding),
                    span,
                })
            }
            None => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ValueTypeUnavailable {
                        name: binding_info.name.clone(),
                    },
                    span,
                });
                self.error_expr(id, span)
            }
        }
    }

    /// Infer the type of a lambda when no expected type is available.
    /// Generates fresh InferVars for each parameter; they are solved by checking
    /// the body, then zonked to concrete types at the end of lowering.
    pub(super) fn infer_lambda_expr(
        &mut self,
        id: HirExprId,
        params: &[zutai_hir::HirPatId],
        body: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let param_vars: Vec<TypeId> = params.iter().map(|_| self.fresh_infer_var(span)).collect();

        let mut scoped_bindings = Vec::new();
        let lowered_params: Vec<_> = params
            .iter()
            .zip(&param_vars)
            .map(|(&pat_id, &param_ty)| self.check_pattern(pat_id, param_ty, &mut scoped_bindings))
            .collect();

        let body_thir = self.infer_expr(body);
        let body_ty = self.expr(body_thir).ty;
        self.clear_scoped_value_types(&scoped_bindings);

        // Build curried function type: p1 -> p2 -> ... -> body_ty
        let lambda_ty = param_vars.iter().rev().fold(body_ty, |to, &from| {
            let from = self.resolve(from);
            self.alloc_type(crate::ir::Type {
                kind: TypeKind::Function { from, to },
                span,
            })
        });

        self.alloc_expr(ThirExpr {
            source: id,
            ty: lambda_ty,
            kind: ThirExprKind::Lambda {
                params: lowered_params,
                body: body_thir,
            },
            span,
        })
    }

    pub(super) fn check_lambda_expr(
        &mut self,
        id: HirExprId,
        params: &[zutai_hir::HirPatId],
        body: HirExprId,
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let (param_types, return_type) = self.function_parts(expected, span);

        if param_types.is_empty() {
            let found = self.type_name(expected);
            if !matches!(self.ty(expected).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedFunction { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        }

        if params.len() != param_types.len() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::FunctionClauseArityMismatch {
                    expected: param_types.len(),
                    found: params.len(),
                },
                span,
            });
        }

        let mut scoped_bindings = Vec::new();
        let lowered_params: Vec<_> = params
            .iter()
            .enumerate()
            .map(|(i, &pat_id)| {
                let expected_ty = param_types.get(i).copied().unwrap_or(self.error_type);
                self.check_pattern(pat_id, expected_ty, &mut scoped_bindings)
            })
            .collect();

        let (body_ty, saved_effect_ambient) = self.enter_effectful_result(return_type);
        let body = self.check_expr(body, body_ty);
        self.exit_effectful_result(saved_effect_ambient);
        self.clear_scoped_value_types(&scoped_bindings);

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Lambda {
                params: lowered_params,
                body,
            },
            span,
        })
    }
}
