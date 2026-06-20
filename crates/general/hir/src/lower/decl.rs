use super::*;

impl Lowerer {
    pub(super) fn define_top_decl(&mut self, decl: &ast::Decl) -> BindingId {
        match decl {
            ast::Decl::Inferred { .. } | ast::Decl::Typed { .. } => {
                self.define_current(decl.name().to_string(), BindingKind::TopValue, decl.span())
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
            ast::Decl::TypeAlias {
                params, ty, span, ..
            } => {
                self.push_scope();
                let hir_params = self.lower_type_params(params);
                let ty = self.lower_type(ty);
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
                span,
                ..
            } => {
                self.push_scope();
                let hir_params = self.lower_hir_type_params(params);
                let hir_target = self.lower_type(target);
                let mut seen_methods: HashMap<String, Span> = HashMap::new();
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
                self.pop_scope();
                (
                    HirDeclKind::Constraint {
                        params: hir_params,
                        target: hir_target,
                        methods: hir_methods,
                        derivable: *derivable,
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
                        let mut seen: HashMap<String, Span> = HashMap::new();
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

    pub(super) fn lower_type_params(&mut self, params: &[ast::TypeParam]) -> Vec<HirTypeParam> {
        // First pass: allocate all BindingIds so that forward-references within
        // the param list are handled correctly (mirrors lower_hir_type_params).
        let bindings: Vec<BindingId> = params
            .iter()
            .map(|param| {
                self.define_current(param.name.clone(), BindingKind::TypeParam, param.span)
            })
            .collect();
        // Second pass: resolve bounds, storing them (was D1 resolve-but-don't-store).
        bindings
            .into_iter()
            .zip(params)
            .map(|(binding, param)| {
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
            })
            .collect()
    }

    /// Lower type params for constraint/witness decls: creates `HirTypeParam` with
    /// resolved bounds and lowered kind annotations.
    pub(super) fn lower_hir_type_params(&mut self, params: &[ast::TypeParam]) -> Vec<HirTypeParam> {
        params
            .iter()
            .map(|param| {
                let binding =
                    self.define_current(param.name.clone(), BindingKind::TypeParam, param.span);
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
            })
            .collect()
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
