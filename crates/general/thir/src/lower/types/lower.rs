use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn lower_type(&mut self, id: HirTypeId) -> TypeId {
        let ty = self.hir_type(id);
        match &ty.kind {
            HirTypeKind::BindingRef(binding) => self.alias_or_builtin_type(*binding, ty.span),
            HirTypeKind::Record { fields, tail } => {
                let mut thir_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|field| self.lower_type_record_field(field))
                    .collect();
                let row_tail = self.lower_record_tail(tail.as_ref(), &mut thir_fields);
                self.alloc_type(Type {
                    kind: TypeKind::Record(thir_fields, row_tail),
                    span: ty.span,
                })
            }
            HirTypeKind::Union { variants, tail } => {
                let mut thir_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v: &HirUnionVariant| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|payload| self.lower_type(payload)),
                        span: v.span,
                    })
                    .collect();
                let row_tail = self.lower_union_tail(tail.as_ref(), &mut thir_variants);
                self.alloc_type(Type {
                    kind: TypeKind::Union(thir_variants, row_tail),
                    span: ty.span,
                })
            }
            HirTypeKind::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        HirTypeTupleItem::Named { name, ty, span } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.lower_type(*ty),
                            span: *span,
                        },
                        HirTypeTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.lower_type(*ty))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(items),
                    span: ty.span,
                })
            }
            HirTypeKind::Optional(inner) => {
                let inner = self.lower_type(*inner);
                self.optional_type(inner, ty.span)
            }
            HirTypeKind::Arrow { from, to } => {
                let from = self.lower_type(*from);
                let to = self.lower_type(*to);
                self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span: ty.span,
                })
            }
            HirTypeKind::Apply { func, arg } => self.lower_type_apply(*func, *arg, ty.span),
            HirTypeKind::Effect { base, row } => {
                let base = self.lower_type(*base);
                let mut row = self.lower_effect_row(row);
                let resolved_base = self.resolve(base);
                match self.ty(resolved_base).kind.clone() {
                    TypeKind::Effect {
                        base: inner_base,
                        row: inner_row,
                    } => {
                        row.ops.extend(inner_row.ops);
                        self.alloc_type(Type {
                            kind: TypeKind::Effect {
                                base: inner_base,
                                row,
                            },
                            span: ty.span,
                        })
                    }
                    _ => self.alloc_type(Type {
                        kind: TypeKind::Effect { base, row },
                        span: ty.span,
                    }),
                }
            }
            HirTypeKind::Select { receiver, fields } => {
                let receiver_ty = self.lower_type(*receiver);
                let resolved = self.resolve_alias(receiver_ty, &mut HashSet::new(), ty.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Record(rec_fields, _) => {
                        let mut selected = Vec::with_capacity(fields.len());
                        for sf in fields {
                            match rec_fields.iter().find(|f| f.name == sf.name) {
                                Some(rf) => selected.push(rf.clone()),
                                None => self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::UnknownField {
                                        name: sf.name.clone(),
                                    },
                                    span: sf.span,
                                }),
                            }
                        }
                        self.alloc_type(Type {
                            kind: TypeKind::Record(selected, RowTail::Closed),
                            span: ty.span,
                        })
                    }
                    TypeKind::Error => self.error_type,
                    _ => self.invalid_type("type-level select requires a record type", ty.span),
                }
            }
            HirTypeKind::Atom(name) => self.alloc_type(Type {
                kind: TypeKind::Atom(name.clone()),
                span: ty.span,
            }),
            HirTypeKind::True => self.alloc_type(Type {
                kind: TypeKind::True,
                span: ty.span,
            }),
            HirTypeKind::False => self.alloc_type(Type {
                kind: TypeKind::False,
                span: ty.span,
            }),
            HirTypeKind::UnresolvedIdent(_) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::InvalidTypeExpression {
                        reason: "unresolved type identifier",
                    },
                    span: ty.span,
                });
                self.error_type
            }
            HirTypeKind::Access { receiver, field } => {
                // Resolve `moduleLib.SomeType` in annotation position.
                // Only simple `BindingRef` receivers are supported (e.g. `serverLib`);
                // chained access (`a.b.C`) is not yet implemented.
                let access_span = ty.span;
                let receiver_hir = self.hir_type(*receiver);
                let binding = match &receiver_hir.kind {
                    HirTypeKind::BindingRef(b) => *b,
                    _ => {
                        return self.invalid_type(
                            "type field access receiver must be a simple name",
                            access_span,
                        );
                    }
                };
                // Look up the record type of the receiver (e.g. the inferred
                // record type of `serverLib := import "server.zt"`).
                let receiver_ty = match self.value_types.get(&binding).copied() {
                    Some(t) => t,
                    None => {
                        return self
                            .invalid_type("type field access on unknown binding", access_span);
                    }
                };
                // Walk to the record fields of that type.
                let fields = match self.record_fields(receiver_ty, access_span) {
                    Some(f) => f,
                    None => {
                        return self
                            .invalid_type("type field access on non-record type", access_span);
                    }
                };
                let Some(record_field) = fields.iter().find(|f| f.name == *field).cloned() else {
                    return self.invalid_type("unknown type field", access_span);
                };
                // If this binding is a known import and the field carries a
                // registered type denotation, return the concrete type so that
                // annotation-position use (`x : serverLib.Server`) type-checks.
                if let Some(import_source) = self.binding_import_key.get(&binding).cloned()
                    && let Some(&denotation) = self
                        .import_type_denotations
                        .get(&(import_source, field.clone()))
                {
                    return denotation;
                }
                record_field.ty
            }
            HirTypeKind::ExprEscape(_) => {
                self.invalid_type("type expression escapes are not supported yet", ty.span)
            }
        }
    }

    pub(in crate::lower) fn lower_effect_row(&mut self, row: &HirEffectRow) -> EffectRow {
        let ops = row
            .ops
            .iter()
            .map(|op| {
                let name = op.path.join(".");
                let (param, result) = if let Some(sig) = op.signature {
                    let sig = self.lower_type(sig);
                    let resolved = self.resolve_alias(sig, &mut HashSet::new(), op.span);
                    match self.ty(resolved).kind {
                        TypeKind::Function { from, to } => (from, to),
                        TypeKind::Error => (self.error_type, self.error_type),
                        _ => {
                            self.diagnostics.push(ThirDiagnostic {
                                kind: ThirDiagnosticKind::MalformedEffectOp {
                                    op: name.clone(),
                                    reason: "effect operation signature must be a function type",
                                },
                                span: op.span,
                            });
                            (self.error_type, self.error_type)
                        }
                    }
                } else if let Some(payload) = op.payload {
                    let payload = self.lower_type(payload);
                    match name.as_str() {
                        "fail" => (payload, self.never_type(op.span)),
                        "warn" | "log" => (payload, self.unit_type(op.span)),
                        "ask" => (self.unit_type(op.span), payload),
                        _ => {
                            self.diagnostics.push(ThirDiagnostic {
                                kind: ThirDiagnosticKind::MalformedEffectOp {
                                    op: name.clone(),
                                    reason:
                                        "compact effect form is only valid for fail, warn, log, or ask",
                                },
                                span: op.span,
                            });
                            (self.error_type, self.error_type)
                        }
                    }
                } else {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::MalformedEffectOp {
                            op: name.clone(),
                            reason: "effect operation needs a payload or an explicit signature",
                        },
                        span: op.span,
                    });
                    (self.error_type, self.error_type)
                };
                EffectOp {
                    name,
                    param,
                    result,
                    span: op.span,
                }
            })
            .collect();
        EffectRow {
            ops,
            tail: RowTail::Closed,
        }
    }

    pub(in crate::lower) fn lower_type_record_field(
        &mut self,
        field: &HirTypeRecordField,
    ) -> TypeRecordField {
        TypeRecordField {
            name: field.name.clone(),
            optional: field.optional,
            ty: self.lower_type(field.ty),
            span: field.span,
        }
    }

    /// Lower a record row tail, expanding `...Shape` spreads into `fields` and
    /// returning the resulting `RowTail`. Anonymous `...` becomes `Open`; a
    /// `<Rest>` row variable becomes a rigid `Param`.
    pub(in crate::lower) fn lower_record_tail(
        &mut self,
        tail: Option<&HirRowTail>,
        fields: &mut Vec<TypeRecordField>,
    ) -> RowTail {
        let Some(tail) = tail else {
            return RowTail::Closed;
        };
        match &tail.kind {
            HirRowTailKind::Anonymous | HirRowTailKind::Unresolved(_) => RowTail::Open,
            HirRowTailKind::Var(binding) => RowTail::Param(*binding),
            HirRowTailKind::Spread(binding) => {
                let spread = self.alias_or_builtin_type(*binding, tail.span);
                let resolved = self.resolve_alias(spread, &mut HashSet::new(), tail.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Record(spread_fields, spread_tail) => {
                        for sf in spread_fields {
                            if fields.iter().any(|f| f.name == sf.name) {
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::OverlappingRowField {
                                        name: sf.name.clone(),
                                    },
                                    span: tail.span,
                                });
                            } else {
                                fields.push(sf);
                            }
                        }
                        spread_tail
                    }
                    TypeKind::Error => RowTail::Closed,
                    _ => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::InvalidTypeExpression {
                                reason: "record spread requires a record type",
                            },
                            span: tail.span,
                        });
                        RowTail::Closed
                    }
                }
            }
        }
    }

    /// Lower a union row tail, expanding `...Shape` spreads into `variants`.
    pub(in crate::lower) fn lower_union_tail(
        &mut self,
        tail: Option<&HirRowTail>,
        variants: &mut Vec<UnionVariant>,
    ) -> RowTail {
        let Some(tail) = tail else {
            return RowTail::Closed;
        };
        match &tail.kind {
            HirRowTailKind::Anonymous | HirRowTailKind::Unresolved(_) => RowTail::Open,
            HirRowTailKind::Var(binding) => RowTail::Param(*binding),
            HirRowTailKind::Spread(binding) => {
                let spread = self.alias_or_builtin_type(*binding, tail.span);
                let resolved = self.resolve_alias(spread, &mut HashSet::new(), tail.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Union(spread_variants, spread_tail) => {
                        for sv in spread_variants {
                            if variants.iter().any(|v| v.name == sv.name) {
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::OverlappingRowField {
                                        name: sv.name.clone(),
                                    },
                                    span: tail.span,
                                });
                            } else {
                                variants.push(sv);
                            }
                        }
                        spread_tail
                    }
                    TypeKind::Error => RowTail::Closed,
                    _ => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::InvalidTypeExpression {
                                reason: "union spread requires a union type",
                            },
                            span: tail.span,
                        });
                        RowTail::Closed
                    }
                }
            }
        }
    }

    pub(in crate::lower) fn lower_type_apply(
        &mut self,
        func: HirTypeId,
        arg: HirTypeId,
        span: Span,
    ) -> TypeId {
        // Walk the left-nested Apply spine to collect head + all args left-to-right.
        let mut args = vec![self.lower_type(arg)];
        let mut head = func;
        loop {
            let head_kind = self.hir_type(head).kind.clone();
            match head_kind {
                HirTypeKind::Apply { func: f, arg: a } => {
                    args.push(self.lower_type(a));
                    head = f;
                }
                _ => break,
            }
        }
        args.reverse();

        let HirTypeKind::BindingRef(binding) = self.hir_type(head).kind else {
            return self.invalid_type("only named type constructors can be applied", span);
        };

        // Built-in single-arg constructors keep existing handling.
        let name = self.hir.bindings[binding.0 as usize].name.clone();
        match name.as_str() {
            "List" if args.len() == 1 => {
                return self.alloc_type(Type {
                    kind: TypeKind::List(args[0]),
                    span,
                });
            }
            "Optional" if args.len() == 1 => return self.optional_type(args[0], span),
            _ => {}
        }

        // Named parametric alias.
        if let Some(params) = self.alias_params.get(&binding).cloned() {
            if args.len() > params.len() {
                // Over-application: more arguments than the constructor accepts.
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: params.len(),
                        found: args.len(),
                    },
                    span,
                });
                return self.error_type;
            }
            if args.len() == params.len() {
                // Saturated: keep the direct-write `AliasApply` representation
                // (canonicalization-equivalent to the Apply-spine via `app_view`).
                return self.alloc_type(Type {
                    kind: TypeKind::AliasApply { binding, args },
                    span,
                });
            }
            // Partial application (`Result E`): curried `Apply` spine over the bare
            // alias head. `resolve_alias` leaves it inert until saturated.
            let head_ty = self.alias_type(binding, span);
            return self.fold_apply(head_ty, &args, span);
        }

        // Higher-kinded type-variable application (`F A`, F a type param of kind
        // `Type -> Type`). Curried `Apply` over the var head so it composes under
        // substitution (`F := Result E` makes `F A` reduce to `Result E A`).
        if matches!(
            self.hir.bindings[binding.0 as usize].kind,
            BindingKind::TypeParam
        ) {
            let head_ty = self.alloc_type(Type {
                kind: TypeKind::TypeVar(binding),
                span,
            });
            return self.fold_apply(head_ty, &args, span);
        }

        self.invalid_type("type is not a parametric constructor", span)
    }

    /// Build a curried `Apply` spine: `fold_apply(F, [A, B])` → `Apply{Apply{F,A},B}`.
    pub(in crate::lower) fn fold_apply(
        &mut self,
        head: TypeId,
        args: &[TypeId],
        span: Span,
    ) -> TypeId {
        let mut spine = head;
        for &arg in args {
            spine = self.alloc_type(Type {
                kind: TypeKind::Apply { func: spine, arg },
                span,
            });
        }
        spine
    }

    pub(in crate::lower) fn alias_or_builtin_type(
        &mut self,
        binding: BindingId,
        span: Span,
    ) -> TypeId {
        // A bare reference to a parametric constructor (without application) is
        // a zero-argument arity error. Check before the binding-kind match so
        // both TopType and TopFunction aliases can be caught here.
        if let Some(params) = self.alias_params.get(&binding).cloned() {
            let name = self.hir.bindings[binding.0 as usize].name.clone();
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                    name,
                    expected: params.len(),
                    found: 0,
                },
                span,
            });
            return self.error_type;
        }
        let binding_info = &self.hir.bindings[binding.0 as usize];
        match binding_info.kind {
            BindingKind::BuiltinType => match binding_info.name.as_str() {
                // Bare `List`/`Optional` constructors (kind `Type -> Type`), used
                // unapplied as higher-kinded witness/constraint targets.
                "List" | "Optional" => self.alloc_type(Type {
                    kind: TypeKind::Con(binding),
                    span,
                }),
                name => self
                    .builtin_type_by_name(name, span)
                    .unwrap_or_else(|| self.invalid_type("unknown built-in type", span)),
            },
            BindingKind::TopType => self.alias_type(binding, span),
            BindingKind::TypeParam => self.alloc_type(Type {
                kind: TypeKind::TypeVar(binding),
                span,
            }),
            BindingKind::Param | BindingKind::Local if self.type_param_scope.contains(&binding) => {
                // A `Param` or `Local` binding that was registered in
                // `type_param_scope` during type-level function body lowering
                // acts as a substitutable type variable.
                self.alloc_type(Type {
                    kind: TypeKind::TypeVar(binding),
                    span,
                })
            }
            _ => self.invalid_type("value binding used as a type", span),
        }
    }

    pub(in crate::lower) fn builtin_type_by_name(
        &mut self,
        name: &str,
        span: Span,
    ) -> Option<TypeId> {
        let kind = match name {
            "Type" => TypeKind::Type,
            "Text" => TypeKind::Text,
            "Bool" => TypeKind::Bool,
            "Int" => TypeKind::Int,
            "Float" => TypeKind::Float,
            _ => return None,
        };
        Some(self.alloc_type(Type { kind, span }))
    }

    pub(in crate::lower) fn alias_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Alias(binding),
            span,
        })
    }

    pub(in crate::lower) fn bool_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Bool,
            span,
        })
    }

    pub(in crate::lower) fn int_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Int,
            span,
        })
    }

    pub(in crate::lower) fn float_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Float,
            span,
        })
    }

    pub(in crate::lower) fn text_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Text,
            span,
        })
    }

    pub(in crate::lower) fn optional_type(&mut self, inner: TypeId, span: Span) -> TypeId {
        let normalized = self.resolve_alias(inner, &mut HashSet::new(), span);
        if matches!(self.ty(normalized).kind, TypeKind::Optional(_)) {
            return normalized;
        }
        self.alloc_type(Type {
            kind: TypeKind::Optional(inner),
            span,
        })
    }
}
