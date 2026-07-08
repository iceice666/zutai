use super::*;

impl Lowerer {
    pub(super) fn define_top_decl(&mut self, decl: &ast::Decl) -> BindingId {
        match decl {
            ast::Decl::Inferred { .. } | ast::Decl::Typed { .. } => {
                self.define_current(decl.name().to_string(), BindingKind::TopValue, decl.span())
            }
            ast::Decl::Destructure { fields, span, .. } => {
                // Allocate a synthetic (unscoped, so never a duplicate) receiver
                // binding for the record value, plus one in-scope value binding per
                // destructured field name. Pass 2 fills them via `lower_destructure_decl`.
                let receiver = self.alloc_binding_unscoped(
                    "$destructure".to_string(),
                    BindingKind::TopValue,
                    *span,
                );
                let field_bindings = fields
                    .iter()
                    .map(|f| {
                        let binding =
                            self.define_current(f.name.clone(), BindingKind::TopValue, f.span);
                        (binding, f.name.clone())
                    })
                    .collect();
                self.destructure_fields.insert(receiver, field_bindings);
                receiver
            }
            ast::Decl::Use { items, span } => {
                // Allocate a synthetic (unscoped, so never a duplicate) group
                // binding, plus one in-scope value binding per import alias.
                let group =
                    self.alloc_binding_unscoped("$use".to_string(), BindingKind::TopValue, *span);
                let imports = items
                    .iter()
                    .map(|item| {
                        let binding = self.define_current(
                            item.alias.clone(),
                            BindingKind::TopValue,
                            item.span,
                        );
                        (
                            binding,
                            super::types::clone_import_source(&item.source),
                            item.span,
                        )
                    })
                    .collect();
                self.use_imports.insert(group, imports);
                group
            }
            ast::Decl::TypeAlias { .. } => {
                self.define_current(decl.name().to_string(), BindingKind::TopType, decl.span())
            }
            ast::Decl::Function { .. } | ast::Decl::NoSigFn { .. } => self.define_current(
                decl.name().to_string(),
                BindingKind::TopFunction,
                decl.span(),
            ),
            ast::Decl::Constraint { name, methods, .. } => {
                let constraint_binding =
                    self.define_current(name.clone(), BindingKind::TopConstraint, decl.span());
                // D1/D3: Allocate a BindingId for each *named* method now (Pass 1) so
                // method names are resolvable by any body lowered in Pass 2, regardless of
                // source order.  Operator methods get `None` (deferred to a later increment).
                let method_bindings: Vec<Option<BindingId>> = methods
                    .iter()
                    .map(|m| match &m.name {
                        ast::MethodName::Ident(method_name) => {
                            let id = self.define_current(
                                method_name.clone(),
                                BindingKind::ConstraintMethod,
                                m.span,
                            );
                            Some(id)
                        }
                        // D6 (4b): allocate an unscoped binding for operator methods so
                        // ThirConstraintMethod.binding is Some for operators too.
                        // Unscoped (not define_current) because operators are never
                        // referenced as bare idents and two constraints could share the
                        // same symbol name without a DuplicateBinding conflict.
                        ast::MethodName::Operator(op) => {
                            let id = self.alloc_binding_unscoped(
                                op.clone(),
                                BindingKind::ConstraintMethod,
                                m.span,
                            );
                            Some(id)
                        }
                    })
                    .collect();
                self.constraint_method_bindings
                    .insert(constraint_binding, method_bindings);
                constraint_binding
            }
            ast::Decl::Witness { constraint, .. } => {
                // D3: unscoped so duplicate witnesses don't raise DuplicateBinding
                self.alloc_binding_unscoped(
                    constraint.clone(),
                    BindingKind::TopWitness,
                    decl.span(),
                )
            }
        }
    }

    pub(super) fn alloc_binding_unscoped(
        &mut self,
        name: String,
        kind: BindingKind,
        span: Span,
    ) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding { name, kind, span });
        id
    }

    pub(super) fn lower_decl(&mut self, decl: &ast::Decl, binding: BindingId) -> HirDeclId {
        let (kind, span) = match decl {
            ast::Decl::Inferred { value, span, .. } => (
                HirDeclKind::Value {
                    annotation: None,
                    value: self.lower_expr(value),
                },
                *span,
            ),
            ast::Decl::Typed {
                ty, value, span, ..
            } => (
                HirDeclKind::Value {
                    annotation: Some(self.lower_type(ty)),
                    value: self.lower_expr(value),
                },
                *span,
            ),
            ast::Decl::Destructure { .. } => {
                unreachable!("destructure decls are expanded in lower_file, not lower_decl")
            }
            ast::Decl::Use { .. } => {
                unreachable!("use decls are expanded in lower_file, not lower_decl")
            }
            ast::Decl::TypeAlias {
                params, ty, span, ..
            } => {
                self.push_scope();
                let hir_params = self.lower_type_params(params);
                let ty = self.lower_type(ty);
                self.report_unused_level_params(&hir_params);
                self.pop_scope();
                (
                    HirDeclKind::TypeAlias {
                        params: hir_params.into_iter().map(|p| p.binding).collect(),
                        ty,
                    },
                    *span,
                )
            }
            ast::Decl::Function {
                params,
                sig,
                clauses,
                span,
                ..
            } => {
                self.push_scope();
                let params = self.lower_type_params(params);
                let sig = self.lower_type(sig);
                let clauses = clauses
                    .iter()
                    .map(|clause| self.lower_clause(clause))
                    .collect();
                self.report_unused_level_params(&params);
                self.pop_scope();
                (
                    HirDeclKind::Function {
                        params,
                        sig: Some(sig),
                        clauses,
                    },
                    *span,
                )
            }
            ast::Decl::NoSigFn {
                patterns,
                body,
                span,
                ..
            } => {
                self.push_scope();
                let patterns = patterns.iter().map(|pat| self.lower_pattern(pat)).collect();
                let body = self.lower_expr(body);
                self.pop_scope();
                (
                    HirDeclKind::Function {
                        params: Vec::new(),
                        sig: None,
                        clauses: vec![HirClause {
                            patterns,
                            guard: None,
                            body,
                            span: *span,
                        }],
                    },
                    *span,
                )
            }
            ast::Decl::Constraint {
                params,
                target,
                methods,
                derivable,
                recipe,
                span,
                ..
            } => {
                self.push_scope();
                let hir_params = self.lower_hir_type_params(params);
                let hir_target = self.lower_type(target);
                let mut seen_methods: FxHashMap<String, Span> = FxHashMap::default();
                let mut hir_methods = Vec::new();
                for (idx, method) in methods.iter().enumerate() {
                    let key = method.name.as_str().to_string();
                    if let Some(&first_span) = seen_methods.get(&key) {
                        self.diagnostics.push(HirDiagnostic {
                            kind: HirDiagnosticKind::DuplicateConstraintMethod {
                                name: key.clone(),
                                first_span,
                            },
                            span: method.span,
                        });
                    } else {
                        seen_methods.insert(key.clone(), method.span);
                    }
                    self.push_scope();
                    let method_params = self.lower_hir_type_params(&method.params);
                    let sig = self.lower_type(&method.sig);
                    let default = method
                        .default
                        .iter()
                        .map(|c| self.lower_clause(c))
                        .collect();
                    self.pop_scope();
                    let (is_operator, name_str) = match &method.name {
                        ast::MethodName::Ident(s) => (false, s.clone()),
                        ast::MethodName::Operator(s) => (true, s.clone()),
                    };
                    // D3: retrieve the pre-allocated binding from the threaded map.
                    let method_binding = self
                        .constraint_method_bindings
                        .get(&binding)
                        .and_then(|v| v.get(idx))
                        .copied()
                        .flatten();
                    hir_methods.push(HirConstraintMethod {
                        name: name_str,
                        is_operator,
                        optional: method.optional,
                        params: method_params,
                        sig,
                        default,
                        span: method.span,
                        binding: method_binding,
                    });
                }
                let recipe = recipe.as_ref().map(|recipe| {
                    self.push_scope();
                    let params = self.lower_hir_type_params(&recipe.params);
                    let body = self.lower_expr(&recipe.body);
                    self.pop_scope();
                    HirDeriveRecipe {
                        params,
                        body,
                        span: recipe.span,
                    }
                });
                self.pop_scope();
                (
                    HirDeclKind::Constraint {
                        params: hir_params,
                        target: hir_target,
                        methods: hir_methods,
                        derivable: *derivable,
                        recipe,
                    },
                    *span,
                )
            }
            ast::Decl::Witness {
                constraint,
                target,
                params,
                body,
                span,
            } => {
                let constraint_binding = match self.resolve(constraint) {
                    Some(bid) => Some(bid),
                    None => {
                        self.diagnostics.push(HirDiagnostic {
                            kind: HirDiagnosticKind::UnknownConstraint {
                                name: constraint.clone(),
                            },
                            span: *span,
                        });
                        None
                    }
                };
                self.push_scope();
                let hir_params = self.lower_hir_type_params(params);
                let hir_target = self.lower_type(target);
                let (hir_fields, derive) = match body {
                    ast::WitnessBody::Derive => (Vec::new(), true),
                    ast::WitnessBody::Fields(fields) => {
                        let mut seen: FxHashMap<String, Span> = FxHashMap::default();
                        let mut hir_fields = Vec::new();
                        for field in fields {
                            let key = field.name.as_str().to_string();
                            if let Some(&first_span) = seen.get(&key) {
                                self.diagnostics.push(HirDiagnostic {
                                    kind: HirDiagnosticKind::DuplicateWitnessField {
                                        name: key.clone(),
                                        first_span,
                                    },
                                    span: field.span,
                                });
                            } else {
                                seen.insert(key.clone(), field.span);
                            }
                            let (is_operator, name_str) = match &field.name {
                                ast::MethodName::Ident(s) => (false, s.clone()),
                                ast::MethodName::Operator(s) => (true, s.clone()),
                            };
                            let value = self.lower_expr(&field.value);
                            hir_fields.push(HirWitnessField {
                                name: name_str,
                                is_operator,
                                value,
                                span: field.span,
                            });
                        }
                        (hir_fields, false)
                    }
                };
                self.pop_scope();
                (
                    HirDeclKind::Witness {
                        constraint: constraint_binding,
                        target: hir_target,
                        params: hir_params,
                        fields: hir_fields,
                        derive,
                    },
                    *span,
                )
            }
        };
        self.alloc_decl(HirDecl {
            binding,
            kind,
            span,
        })
    }

    /// Expand a destructuring binding into a synthetic `receiver ::= value` decl
    /// plus one `field ::= receiver.field` value decl per name. The receiver is
    /// bound once so member accesses do not re-evaluate the (possibly effectful or
    /// import-backed) record value.
    pub(super) fn lower_destructure_decl(
        &mut self,
        receiver: BindingId,
        value: &ast::Expr,
        out: &mut Vec<HirDeclId>,
    ) {
        let value_expr = self.lower_expr(value);
        let receiver_span = self.bindings[receiver.0 as usize].span;
        let receiver_decl = self.alloc_decl(HirDecl {
            binding: receiver,
            kind: HirDeclKind::Value {
                annotation: None,
                value: value_expr,
            },
            span: receiver_span,
        });
        out.push(receiver_decl);

        let fields = self
            .destructure_fields
            .remove(&receiver)
            .unwrap_or_default();
        for (field_binding, field_name) in fields {
            let span = self.bindings[field_binding.0 as usize].span;
            let receiver_ref = self.alloc_expr(HirExpr {
                kind: HirExprKind::BindingRef(receiver),
                span,
            });
            let access = self.alloc_expr(HirExpr {
                kind: HirExprKind::Access {
                    receiver: receiver_ref,
                    field: field_name,
                },
                span,
            });
            let decl = self.alloc_decl(HirDecl {
                binding: field_binding,
                kind: HirDeclKind::Value {
                    annotation: None,
                    value: access,
                },
                span,
            });
            out.push(decl);
        }
    }

    /// Expand `use base { item as alias; }` into ordinary import value decls:
    /// `alias ::= import base.item;`. The syntax is top-level sugar only; later
    /// compiler stages see plain imports.
    pub(super) fn lower_use_decl(&mut self, group: BindingId, out: &mut Vec<HirDeclId>) {
        let imports = self.use_imports.remove(&group).unwrap_or_default();
        for (binding, source, span) in imports {
            let value = self.alloc_expr(HirExpr {
                kind: HirExprKind::Import(source),
                span,
            });
            let decl = self.alloc_decl(HirDecl {
                binding,
                kind: HirDeclKind::Value {
                    annotation: None,
                    value,
                },
                span,
            });
            out.push(decl);
        }
    }

    pub(super) fn lower_type_params(&mut self, params: &[ast::TypeParam]) -> Vec<HirTypeParam> {
        // First pass: allocate all BindingIds so that forward-references within
        // the param list are handled correctly (mirrors lower_hir_type_params).
        // Level binders (`$l`) get a `LevelParam` binding instead of `TypeParam`.
        let bindings: Vec<BindingId> = params
            .iter()
            .map(|param| {
                let kind = if param.is_level {
                    BindingKind::LevelParam
                } else {
                    BindingKind::TypeParam
                };
                self.define_current(param.name.clone(), kind, param.span)
            })
            .collect();
        // Second pass: resolve bounds, storing them (was D1 resolve-but-don't-store).
        bindings
            .into_iter()
            .zip(params)
            .map(|(binding, param)| self.finish_type_param(binding, param))
            .collect()
    }

    /// Lower type params for constraint/witness decls: creates `HirTypeParam` with
    /// resolved bounds and lowered kind annotations.
    pub(super) fn lower_hir_type_params(&mut self, params: &[ast::TypeParam]) -> Vec<HirTypeParam> {
        params
            .iter()
            .map(|param| {
                let kind = if param.is_level {
                    BindingKind::LevelParam
                } else {
                    BindingKind::TypeParam
                };
                let binding = self.define_current(param.name.clone(), kind, param.span);
                self.finish_type_param(binding, param)
            })
            .collect()
    }

    /// Resolve a single type/level param's bounds and kind annotation. Level
    /// binders (`$l`) carry neither.
    fn finish_type_param(&mut self, binding: BindingId, param: &ast::TypeParam) -> HirTypeParam {
        if param.is_level {
            return HirTypeParam {
                binding,
                bounds: vec![],
                kind: None,
                span: param.span,
            };
        }
        let bounds: Vec<BindingId> = param
            .bounds
            .iter()
            .filter_map(|bound| match self.resolve(&bound.name) {
                Some(bid) => Some(bid),
                None => {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::UnknownIdentifier {
                            name: bound.name.clone(),
                        },
                        span: bound.span,
                    });
                    None
                }
            })
            .collect();
        let kind = param.kind.as_ref().map(|k| self.lower_type(k));
        HirTypeParam {
            binding,
            bounds,
            kind,
            span: param.span,
        }
    }

    /// Report any declared level binder (`<$l>`) that was never referenced by a
    /// `$…` level use. Call after the signature/body of a scope is lowered.
    pub(super) fn report_unused_level_params(&mut self, params: &[HirTypeParam]) {
        for param in params {
            let binding = &self.bindings[param.binding.0 as usize];
            if binding.kind == BindingKind::LevelParam
                && !self.used_level_params.contains(&param.binding)
            {
                let name = binding.name.clone();
                self.diagnostics.push(HirDiagnostic {
                    kind: HirDiagnosticKind::UnusedLevelParam { name },
                    span: param.span,
                });
            }
        }
    }

    pub(super) fn lower_clause(&mut self, clause: &ast::FuncClause) -> HirClause {
        self.push_scope();
        let patterns = clause
            .patterns
            .iter()
            .map(|pat| self.lower_pattern(pat))
            .collect();
        let guard = clause.guard.as_ref().map(|guard| self.lower_expr(guard));
        let body = self.lower_expr(&clause.body);
        self.pop_scope();
        HirClause {
            patterns,
            guard,
            body,
            span: clause.span,
        }
    }
}
