use super::*;

struct OverlayApply {
    builtin: BindingId,
    source: HirExprId,
    patch: HirExprId,
    deep: bool,
}

impl<'hir> Lowerer<'hir> {
    /// Peel one layer of ForAll quantifiers from `ty`, substituting each bound
    /// parameter with a fresh InferVar. Returns the inner type and the fresh
    /// infer-variable TypeIds in parameter order.
    pub(super) fn peel_forall(&mut self, ty: TypeId, span: Span) -> (TypeId, Vec<TypeId>) {
        let resolved = self.resolve_alias(ty, &mut FxHashSet::default(), span);
        let resolved = self.resolve(resolved);
        match self.type_arena[resolved.0 as usize].kind.clone() {
            TypeKind::ForAll { params, body, .. } => {
                let mut subst = FxHashMap::default();
                let mut infer_ids = Vec::with_capacity(params.len());
                for param in &params {
                    let fresh = self.fresh_infer_var(span);
                    subst.insert(*param, fresh);
                    infer_ids.push(fresh);
                }
                let instantiated = self.instantiate_type_vars(body, &subst);
                (instantiated, infer_ids)
            }
            _ => (ty, Vec::new()),
        }
    }

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
        if let HirExprKind::Lambda { params, .. } = &self.hir_expr(func).kind
            && params.len() == 1
        {
            return self.lower_single_arg_lambda_apply_expr(id, func, arg, span);
        }
        let func = self.infer_expr(func);
        // A rank-1 polymorphic `BindingRef` callee has already freshened its `<A>`
        // TypeVars and recorded the instantiation (`lower_binding_ref`), leaving
        // only InferVars in its type — so the local `collect_type_vars` below would
        // find nothing. Forward the ref's recorded instantiation onto the `Apply`
        // node so callee dispatch is unchanged. (Higher-rank bindings record an
        // empty instantiation and keep the existing peel/collect path.)
        let binding_ref_inst = match &self.expr(func).kind {
            ThirExprKind::BindingRef { instantiation, .. } if !instantiation.is_empty() => {
                Some(instantiation.clone())
            }
            _ => None,
        };
        let func_ty = self.expr(func).ty;
        let (func_ty, forall_instantiation) = self.peel_forall(func_ty, span);
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
                    forall_instantiation: Vec::new(),
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
            let mut subst = FxHashMap::default();
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
            let mut row_subst = FxHashMap::default();
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
        let effect_ty = self.resolve_alias(to, &mut FxHashSet::default(), span);
        let result_ty = match self.type_arena[effect_ty.0 as usize].kind.clone() {
            TypeKind::Effect { base, row } => {
                self.discharge_row(&row, span);
                base
            }
            _ => result_ty,
        };
        let instantiation = binding_ref_inst.unwrap_or(instantiation);
        self.alloc_expr(ThirExpr {
            source: id,
            ty: result_ty,
            kind: ThirExprKind::Apply {
                func,
                arg,
                instantiation,
                forall_instantiation,
            },
            span,
        })
    }

    fn lower_single_arg_lambda_apply_expr(
        &mut self,
        id: HirExprId,
        func: HirExprId,
        arg: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let arg = self.infer_expr(arg);
        let from = self.expr(arg).ty;
        let to = self.fresh_infer_var(span);
        let func_ty = self.alloc_type(Type {
            kind: TypeKind::Function { from, to },
            span: self.hir_expr(func).span,
        });
        let func = self.check_expr(func, func_ty);
        let result_ty = self.resolve(to);
        let effect_ty = self.resolve_alias(result_ty, &mut FxHashSet::default(), span);
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
                instantiation: Vec::new(),
                forall_instantiation: Vec::new(),
            },
            span,
        })
    }

    fn field_section_binding(&self, params: &[zutai_hir::HirPatId]) -> Option<BindingId> {
        let [param] = params else {
            return None;
        };
        let zutai_hir::HirPatKind::Bind(binding) = self.hir_pat(*param).kind else {
            return None;
        };
        (self.hir.bindings[binding.0 as usize].name == super::super::FIELD_SECTION_PARAM)
            .then_some(binding)
    }

    pub(super) fn access_roots_in_active_field_section(&self, id: HirExprId) -> bool {
        match &self.hir_expr(id).kind {
            HirExprKind::BindingRef(binding) => self.field_section_params.contains(binding),
            HirExprKind::Access { receiver, .. } | HirExprKind::OptAccess { receiver, .. } => {
                self.access_roots_in_active_field_section(*receiver)
            }
            _ => false,
        }
    }

    fn expr_roots_in_binding(&self, id: HirExprId, binding: BindingId) -> bool {
        match &self.hir_expr(id).kind {
            HirExprKind::BindingRef(candidate) => *candidate == binding,
            HirExprKind::Access { receiver, .. } | HirExprKind::OptAccess { receiver, .. } => {
                self.expr_roots_in_binding(*receiver, binding)
            }
            _ => false,
        }
    }

    fn overlay_full_apply(&self, func: HirExprId) -> Option<OverlayApply> {
        let HirExprKind::Apply {
            func: builtin,
            arg: patch,
        } = self.hir_expr(func).kind.clone()
        else {
            return None;
        };
        let (binding, source, name) = self.overlay_builtin_or_stdlib_alias(builtin)?;
        match name {
            "overlay" => Some(OverlayApply {
                builtin: binding,
                source,
                patch,
                deep: false,
            }),
            "overlayDeep" => Some(OverlayApply {
                builtin: binding,
                source,
                patch,
                deep: true,
            }),
            _ => None,
        }
    }

    fn overlay_builtin_or_stdlib_alias(
        &self,
        expr: HirExprId,
    ) -> Option<(BindingId, HirExprId, &'static str)> {
        if let HirExprKind::BindingRef(binding) = self.hir_expr(expr).kind.clone() {
            let binding_info = &self.hir.bindings[binding.0 as usize];
            if binding_info.kind == BindingKind::BuiltinValue {
                return match binding_info.name.as_str() {
                    "overlay" => Some((binding, expr, "overlay")),
                    "overlayDeep" => Some((binding, expr, "overlayDeep")),
                    _ => None,
                };
            }
        }
        let field = self.stdlib_config_overlay_field(expr, &mut FxHashSet::default())?;
        let binding = self.builtin_binding(field)?;
        Some((binding, expr, field))
    }

    fn stdlib_config_overlay_field(
        &self,
        expr: HirExprId,
        seen: &mut FxHashSet<BindingId>,
    ) -> Option<&'static str> {
        match &self.hir_expr(expr).kind {
            HirExprKind::Access { receiver, field }
                if matches!(field.as_str(), "overlay" | "overlayDeep") =>
            {
                if self.expr_is_stdlib_import(*receiver, "config", seen) {
                    Some(if field == "overlay" {
                        "overlay"
                    } else {
                        "overlayDeep"
                    })
                } else {
                    None
                }
            }
            HirExprKind::BindingRef(binding) => {
                if !seen.insert(*binding) {
                    return None;
                }
                let value = self.value_decl_expr(*binding)?;
                self.stdlib_config_overlay_field(value, seen)
            }
            _ => None,
        }
    }

    fn expr_is_stdlib_import(
        &self,
        expr: HirExprId,
        module: &str,
        seen: &mut FxHashSet<BindingId>,
    ) -> bool {
        match &self.hir_expr(expr).kind {
            HirExprKind::Import(zutai_hir::HirImportSource::Path(parts)) => {
                matches!(parts.as_slice(), [root, name] if root == "stdlib" && name == module)
            }
            HirExprKind::BindingRef(binding) => {
                if !seen.insert(*binding) {
                    return false;
                }
                self.value_decl_expr(*binding)
                    .is_some_and(|value| self.expr_is_stdlib_import(value, module, seen))
            }
            _ => false,
        }
    }

    fn value_decl_expr(&self, binding: BindingId) -> Option<HirExprId> {
        self.hir.decls.iter().find_map(|decl_id| {
            let decl = self.hir_decl(*decl_id);
            if decl.binding != binding {
                return None;
            }
            let HirDeclKind::Value { value, .. } = decl.kind else {
                return None;
            };
            Some(value)
        })
    }

    fn builtin_binding(&self, name: &str) -> Option<BindingId> {
        self.hir
            .bindings
            .iter()
            .position(|binding| binding.kind == BindingKind::BuiltinValue && binding.name == name)
            .map(|index| BindingId(index as u32))
    }

    fn lower_overlay_apply_expr(
        &mut self,
        id: HirExprId,
        inner_source: HirExprId,
        base: HirExprId,
        span: Span,
        overlay: OverlayApply,
    ) -> ThirExprId {
        let OverlayApply {
            builtin,
            source,
            patch,
            deep,
        } = overlay;
        let builtin_ref = self.lower_binding_ref(source, builtin, self.hir_expr(source).span, true);
        let base_expr = self.infer_expr(base);
        let target = self.expr(base_expr).ty;
        let patch_ty = self.patch_type(target, deep, span);
        let patch_expr = self.check_expr(patch, patch_ty);
        let inner_ty = self.alloc_type(Type {
            kind: TypeKind::Function {
                from: target,
                to: target,
            },
            span,
        });
        let inner = self.alloc_expr(ThirExpr {
            source: inner_source,
            ty: inner_ty,
            kind: ThirExprKind::Apply {
                func: builtin_ref,
                arg: patch_expr,
                instantiation: Vec::new(),
                forall_instantiation: Vec::new(),
            },
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty: target,
            kind: ThirExprKind::Apply {
                func: inner,
                arg: base_expr,
                instantiation: Vec::new(),
                forall_instantiation: Vec::new(),
            },
            span,
        })
    }

    pub(super) fn lower_binding_ref(
        &mut self,
        id: HirExprId,
        binding: BindingId,
        span: Span,
        instantiate: bool,
    ) -> ThirExprId {
        let binding_info = &self.hir.bindings[binding.0 as usize];
        // Local bindings (a parameter / let-bound name) carry type variables that
        // are *inherited* from an enclosing function's scheme — rigid in this
        // context and not the binding's own quantifier. Only a top-level binding
        // owns its `<A>` scheme, so only those may be freshened per use.
        let is_local = matches!(binding_info.kind, BindingKind::Param | BindingKind::Local);
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
                // InferVar-based poly scheme (no-signature inference).
                let ty = match self.poly_schemes.get(&binding).cloned() {
                    Some(scheme) => {
                        let subst: FxHashMap<u32, TypeId> = scheme
                            .into_iter()
                            .map(|v| (v, self.fresh_infer_var(span)))
                            .collect();
                        self.instantiate_infer_vars(ty, &subst)
                    }
                    None => ty,
                };
                // A rank-1 explicit `<A>` annotation lowers to free `TypeVar`s.
                // When this binding is *applied*, `lower_apply_expr` freshens them
                // per call site; but a polymorphic *value* used directly (passed as
                // an argument, returned, or bound) is never in callee position, so
                // freshen them here and record the instantiation so TLC can
                // monomorphize and thread witness dicts. Higher-rank bindings (an
                // outer `ForAll`) are left untouched — their quantifiers stay on the
                // existing apply-site / fall-through path; freshening here would
                // wrongly capture `ForAll`-bound vars as if they were free.
                let resolved = self.resolve_alias(ty, &mut FxHashSet::default(), span);
                let is_forall = matches!(self.ty(resolved).kind, TypeKind::ForAll { .. });
                let type_vars = if instantiate && !is_forall && !is_local {
                    self.collect_type_vars(ty)
                } else {
                    Vec::new()
                };
                let (ty, instantiation) = if type_vars.is_empty() {
                    (ty, Vec::new())
                } else {
                    let mut subst: FxHashMap<BindingId, TypeId> = FxHashMap::default();
                    let mut inst = Vec::with_capacity(type_vars.len());
                    for var in &type_vars {
                        let fresh = self.fresh_infer_var(span);
                        subst.insert(*var, fresh);
                        inst.push(fresh);
                    }
                    (self.instantiate_type_vars(ty, &subst), inst)
                };
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::BindingRef {
                        binding,
                        instantiation,
                        forall_instantiation: Vec::new(),
                    },
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
        let expected_body = self.strip_forall_expected(expected, span);
        let (param_types, return_type) = self.function_parts(expected_body, span);
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
        let field_section_binding = self
            .field_section_binding(params)
            .filter(|binding| self.expr_roots_in_binding(body, *binding));
        let inserted_field_section_binding =
            field_section_binding.is_some_and(|binding| self.field_section_params.insert(binding));
        let body = self.check_expr(body, body_ty);
        if inserted_field_section_binding && let Some(binding) = field_section_binding {
            self.field_section_params.remove(&binding);
        }
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

    fn strip_forall_expected(&mut self, expected: TypeId, span: Span) -> TypeId {
        let mut current = self.resolve_alias(expected, &mut FxHashSet::default(), span);
        loop {
            let resolved = self.resolve(current);
            match self.type_arena[resolved.0 as usize].kind.clone() {
                TypeKind::ForAll { body, .. } => current = body,
                _ => return current,
            }
        }
    }
}
