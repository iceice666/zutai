use super::*;

impl<'hir> Lowerer<'hir> {
    /// Interpret a kind annotation type-expression: `Type -> Type` → `Arrow`,
    /// everything else (the `Type` leaf) → `Star`.
    pub(in crate::lower) fn hir_kind_of(&self, hir_ty: HirTypeId) -> Kind {
        match &self.hir_type(hir_ty).kind {
            HirTypeKind::Arrow { from, to } => Kind::Arrow(
                Box::new(self.hir_kind_of(*from)),
                Box::new(self.hir_kind_of(*to)),
            ),
            _ => Kind::Star,
        }
    }

    /// Compute the kind of a type. `TypeVar` looks up its declared kind; `Con`
    /// (builtin `List`/`Optional`) is `Type -> Type`; a bare named `Alias` is the
    /// arrow chain of its arity; `Apply` drops one arrow off its head's kind.
    /// All saturated/ground forms are `Star`.
    pub(in crate::lower) fn kind_of(&self, ty: TypeId) -> Kind {
        let ty = self.resolve(ty);
        match &self.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(b) => self.type_param_kinds.get(b).cloned().unwrap_or(Kind::Star),
            TypeKind::Con(_) => Kind::Arrow(Box::new(Kind::Star), Box::new(Kind::Star)),
            TypeKind::Alias(b) => {
                let arity = self.alias_params.get(b).map(|p| p.len()).unwrap_or(0);
                (0..arity).fold(Kind::Star, |acc, _| {
                    Kind::Arrow(Box::new(Kind::Star), Box::new(acc))
                })
            }
            TypeKind::Apply { func, .. } => match self.kind_of(*func) {
                Kind::Arrow(_, res) => *res,
                Kind::Star => Kind::Star,
            },
            _ => Kind::Star,
        }
    }

    /// Verify a type used in a value position (value annotation, function
    /// signature) is fully applied — kind `Star`. A partial application
    /// (`Pair Text`, kind `Type -> Type`) is not a value type; re-emit the
    /// `TypeConstructorArityMismatch` so v1's new partial-application support
    /// does not silently accept under-applied constructors outside witness
    /// targets. A saturated `F A` (kind `Star`) is fine; recurse its arguments.
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
            TypeKind::List(e) | TypeKind::Optional(e) => self.require_ground_type(e, span),
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
            TypeKind::Apply { .. } => {
                if self.kind_of(r) == Kind::Star {
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
            TypeKind::Alias(_) if self.kind_of(r) != Kind::Star => {
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
