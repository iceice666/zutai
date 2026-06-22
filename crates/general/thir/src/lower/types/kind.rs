use super::*;

impl<'hir> Lowerer<'hir> {
    /// Interpret a kind annotation type-expression: `Type -> Type` lowers to an
    /// arrow kind over fresh universe metas; a `Type` leaf is `Type α`.
    pub(in crate::lower) fn hir_kind_of(&mut self, hir_ty: HirTypeId) -> Kind {
        let ty = self.hir_type(hir_ty);
        match &ty.kind {
            HirTypeKind::Arrow { from, to } => Kind::Arrow(
                Box::new(self.hir_kind_of(*from)),
                Box::new(self.hir_kind_of(*to)),
            ),
            HirTypeKind::BindingRef(binding)
                if self.hir.bindings[binding.0 as usize].name == "Type" =>
            {
                Kind::Type(self.fresh_level_meta())
            }
            _ => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::InvalidTypeExpression {
                        reason: "kind annotations must be Type or arrows over Type",
                    },
                    span: ty.span,
                });
                Kind::ground()
            }
        }
    }

    /// Compute the kind of a type. Concrete, saturated forms return
    /// `Kind::Type(level)`; constructors return arrows over universe-carrying
    /// kinds. Cumulativity is checked by `kind_compatible`, not exact equality.
    pub(in crate::lower) fn kind_of(&mut self, ty: TypeId, span: Span) -> Kind {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) => self
                .type_param_kinds
                .get(&b)
                .cloned()
                .unwrap_or_else(Kind::ground),
            TypeKind::Con(_) => {
                let level = self.fresh_level_meta();
                Kind::Arrow(
                    Box::new(Kind::Type(level.clone())),
                    Box::new(Kind::Type(level)),
                )
            }
            TypeKind::Alias(b) => self.alias_constructor_kind(b, span),
            TypeKind::Apply { func, arg } => {
                let head_kind = self.kind_of(func, span);
                match head_kind {
                    Kind::Arrow(param, result) => {
                        let arg_kind = self.kind_of(arg, span);
                        if self.kind_compatible(&param, &arg_kind, span) {
                            *result
                        } else {
                            Kind::ground()
                        }
                    }
                    Kind::Type(_) => Kind::ground(),
                    Kind::Row(_) => Kind::ground(),
                }
            }
            _ => Kind::Type(self.type_universe(ty, span)),
        }
    }

    pub(in crate::lower) fn alias_constructor_kind(
        &mut self,
        binding: BindingId,
        span: Span,
    ) -> Kind {
        let params = self.alias_params.get(&binding).cloned().unwrap_or_default();
        if params.is_empty() {
            if let Some(body) = self.aliases.get(&binding).copied() {
                return Kind::Type(self.type_universe(body, span));
            }
            return Kind::ground();
        }

        let mut subst = FxHashMap::default();
        let mut param_kinds = Vec::with_capacity(params.len());
        for param in &params {
            let level = self.fresh_level_meta();
            let kind = Kind::Type(level.clone());
            param_kinds.push(kind);
            let ty = self.alloc_type(Type {
                kind: TypeKind::TypeVar(*param),
                span,
            });
            self.type_universe_cache.insert(ty, level);
            subst.insert(*param, ty);
        }

        let result = if let Some(body) = self.aliases.get(&binding).copied() {
            let body = self.instantiate_type_vars(body, &subst);
            Kind::Type(self.type_universe(body, span))
        } else {
            Kind::ground()
        };

        param_kinds.into_iter().rev().fold(result, |acc, param| {
            Kind::Arrow(Box::new(param), Box::new(acc))
        })
    }

    /// Verify a type used in a value position is fully applied. Any
    /// `Kind::Type(_)` is concrete; arrows remain under-applied constructors.
    pub(in crate::lower) fn require_ground_type(&mut self, ty: TypeId, span: Span) {
        let r = self.resolve(ty);
        match self.type_arena[r.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                self.require_ground_type(from, span);
                self.require_ground_type(to, span);
            }
            TypeKind::Effect { base, row } => {
                self.require_ground_type(base, span);
                for op in row.ops {
                    self.require_ground_type(op.param, span);
                    self.require_ground_type(op.result, span);
                }
            }
            TypeKind::List(e)
            | TypeKind::Optional(e)
            | TypeKind::Maybe(e)
            | TypeKind::Patch { target: e, .. } => self.require_ground_type(e, span),
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.require_ground_type(f.ty, span);
                }
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(p) = v.payload {
                        self.require_ground_type(p, span);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let t = match item {
                        TypeTupleItem::Named { ty, .. } => ty,
                        TypeTupleItem::Positional(ty) => ty,
                    };
                    self.require_ground_type(t, span);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in args {
                    self.require_ground_type(a, span);
                }
            }
            TypeKind::ForAll { body, .. } => self.require_ground_type(body, span),
            TypeKind::Apply { .. } => {
                if self.kind_of(r, span).is_concrete_type() {
                    let (_, args) = self.app_spine(r);
                    for a in args {
                        self.require_ground_type(a, span);
                    }
                } else {
                    self.report_underapplied(r, span);
                }
            }
            TypeKind::Con(binding) => {
                let name = self.hir.bindings[binding.0 as usize].name.clone();
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: 1,
                        found: 0,
                    },
                    span,
                });
            }
            TypeKind::Alias(_) if !self.kind_of(r, span).is_concrete_type() => {
                self.report_underapplied(r, span);
            }
            _ => {}
        }
    }

    /// Emit a `TypeConstructorArityMismatch` for an under-applied constructor
    /// spine (`Pair Text` → expected 2, found 1).
    pub(in crate::lower) fn report_underapplied(&mut self, ty: TypeId, span: Span) {
        let (head, args) = self.app_spine(ty);
        let head = self.resolve(head);
        match self.type_arena[head.0 as usize].kind.clone() {
            TypeKind::Alias(b) => {
                let name = self.hir.bindings[b.0 as usize].name.clone();
                let expected = self
                    .alias_params
                    .get(&b)
                    .map(|p| p.len())
                    .unwrap_or(args.len());
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected,
                        found: args.len(),
                    },
                    span,
                });
            }
            TypeKind::Con(b) => {
                let name = self.hir.bindings[b.0 as usize].name.clone();
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: 1,
                        found: args.len(),
                    },
                    span,
                });
            }
            _ => {
                self.invalid_type(
                    "higher-kinded type used where a concrete type is required",
                    span,
                );
            }
        }
    }
}
