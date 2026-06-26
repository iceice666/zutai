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
                        payload: v
                            .payload
                            .map(|payload| self.lower_predicative_type(payload)),
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
                            ty: self.lower_predicative_type(*ty),
                            span: *span,
                        },
                        HirTypeTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.lower_predicative_type(*ty))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(items),
                    span: ty.span,
                })
            }
            HirTypeKind::Optional(inner) => {
                let inner = self.lower_predicative_type(*inner);
                self.optional_type(inner, ty.span)
            }
            HirTypeKind::Arrow { from, to } => {
                let from = self.lower_type(*from);
                let to = self.lower_predicative_type(*to);
                self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span: ty.span,
                })
            }
            HirTypeKind::Apply { func, arg } => self.lower_type_apply(*func, *arg, ty.span),
            HirTypeKind::Effect { base, row } => {
                let base = self.lower_predicative_type(*base);
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
                let receiver_ty = self.lower_predicative_type(*receiver);
                let resolved = self.resolve_alias(receiver_ty, &mut FxHashSet::default(), ty.span);
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
            HirTypeKind::ForAll { params, body } => {
                let mut binding_ids = Vec::with_capacity(params.len());
                let mut bounds_per_param = Vec::with_capacity(params.len());
                for p in params {
                    self.type_param_scope.insert(p.binding);
                    if let Some(k) = p.kind {
                        let kind = self.hir_kind_of(k);
                        self.type_param_kinds.insert(p.binding, kind);
                    }
                    let bounds: Vec<BindingId> = p.bounds.clone();
                    binding_ids.push(p.binding);
                    bounds_per_param.push(bounds);
                }
                let body_ty = self.lower_type(*body);
                for &binding in &binding_ids {
                    self.type_param_scope.remove(&binding);
                }
                self.alloc_type(Type {
                    kind: TypeKind::ForAll {
                        params: binding_ids,
                        param_bounds: bounds_per_param,
                        body: body_ty,
                    },
                    span: ty.span,
                })
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
                // record type of `serverLib ::= import "server.zt"`).
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
                if let Some(import_source) = self.binding_import_key.get(&binding).cloned() {
                    if let Some(&denotation) = self
                        .import_type_denotations
                        .get(&(import_source.clone(), field.clone()))
                    {
                        return denotation;
                    }
                    // A parametric constructor used bare (`x :: s.Stream`) is a
                    // zero-argument arity error, just like a local generic alias.
                    if let Some(&ctor_binding) = self
                        .import_type_constructors
                        .get(&(import_source, field.clone()))
                    {
                        let expected = self
                            .alias_params
                            .get(&ctor_binding)
                            .map_or(0, |params| params.len());
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                                name: field.clone(),
                                expected,
                                found: 0,
                            },
                            span: access_span,
                        });
                        return self.error_type;
                    }
                }
                record_field.ty
            }
            HirTypeKind::UniverseLevel(level) => {
                let ul = self.lower_level(level);
                self.alloc_type(Type {
                    kind: TypeKind::Type(ul),
                    span: ty.span,
                })
            }
            HirTypeKind::ExprEscape(_) => {
                self.invalid_type("type expression escapes are not supported yet", ty.span)
            }
        }
    }

    /// Map a resolved `HirLevel` to the internal `UniverseLevel`. Level binders
    /// (`$l`) share one fresh meta per binding (per-use linking), minted lazily
    /// so every occurrence of the same `$l` unifies to a single level.
    fn lower_level(&mut self, level: &HirLevel) -> UniverseLevel {
        match level {
            HirLevel::Known(n) => UniverseLevel::Known(*n),
            HirLevel::Var(binding) => {
                if let Some(existing) = self.level_param_metas.get(binding) {
                    existing.clone()
                } else {
                    let meta = self.fresh_level_meta();
                    self.level_param_metas.insert(*binding, meta.clone());
                    meta
                }
            }
            // Already diagnosed in HIR (`UnknownLevelVar` / `NonLevelAsLevel`);
            // keep going with a fresh meta so checking doesn't cascade.
            HirLevel::Unresolved(_) => self.fresh_level_meta(),
            HirLevel::Succ { base, by } => {
                let mut current = self.lower_level(base);
                for _ in 0..*by {
                    current = UniverseLevel::succ(current);
                }
                current
            }
            HirLevel::Max { left, right } => {
                let left = self.lower_level(left);
                let right = self.lower_level(right);
                UniverseLevel::max([left, right])
            }
        }
    }

    pub(in crate::lower) fn lower_predicative_type(&mut self, id: HirTypeId) -> TypeId {
        let ty = self.lower_type(id);
        let resolved = self.resolve(ty);
        if matches!(
            self.type_arena[resolved.0 as usize].kind,
            TypeKind::ForAll { .. }
        ) {
            let span = self.type_arena[ty.0 as usize].span;
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnsupportedFeature {
                    feature: "impredicative type: ForAll is only valid as a direct function argument annotation",
                },
                span,
            });
            return self.error_type;
        }
        ty
    }

    pub(in crate::lower) fn lower_effect_row(&mut self, row: &HirEffectRow) -> EffectRow {
        let ops = row
            .ops
            .iter()
            .map(|op| {
                let name = op.path.join(".");
                let (param, result) = if let Some(sig) = op.signature {
                    let sig = self.lower_type(sig);
                    let resolved = self.resolve_alias(sig, &mut FxHashSet::default(), op.span);
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
            tail: self.lower_effect_row_tail(row.tail.as_ref()),
        }
    }

    /// Lower an effect-row tail. A row variable `...e` becomes a rigid `Param`
    /// (threaded through signatures by exact-tail unification, like record/union
    /// row variables); anonymous `...` / an unresolved name becomes `Open`. A
    /// `...Shape` spread of a named type is not supported for effect rows — it is
    /// refused precisely rather than silently dropped.
    fn lower_effect_row_tail(&mut self, tail: Option<&HirRowTail>) -> RowTail {
        let Some(tail) = tail else {
            return RowTail::Closed;
        };
        match &tail.kind {
            HirRowTailKind::Anonymous | HirRowTailKind::Unresolved(_) => RowTail::Open,
            HirRowTailKind::Var(binding) => RowTail::Param(*binding),
            HirRowTailKind::Spread(_) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::InvalidTypeExpression {
                        reason:
                            "effect-row spread is not supported; use a row variable (...e) or an explicit op list",
                    },
                    span: tail.span,
                });
                RowTail::Closed
            }
        }
    }

    pub(in crate::lower) fn lower_type_record_field(
        &mut self,
        field: &HirTypeRecordField,
    ) -> TypeRecordField {
        TypeRecordField {
            name: field.name.clone(),
            optional: field.optional,
            ty: self.lower_predicative_type(field.ty),
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
                let source = self.hir.bindings[binding.0 as usize].name.clone();
                let spread = self.alias_or_builtin_type(*binding, tail.span);
                let resolved = self.resolve_alias(spread, &mut FxHashSet::default(), tail.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Record(spread_fields, spread_tail) => {
                        for sf in spread_fields {
                            if let Some(existing) =
                                fields.iter().find(|f| f.name == sf.name).cloned()
                            {
                                let existing = self.record_field_type_name(&existing);
                                let incoming = self.record_field_type_name(&sf);
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::OverlappingRowField {
                                        item: RowOverlapItem::RecordField,
                                        source: source.clone(),
                                        name: sf.name.clone(),
                                        existing,
                                        incoming,
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
                let source = self.hir.bindings[binding.0 as usize].name.clone();
                let spread = self.alias_or_builtin_type(*binding, tail.span);
                let resolved = self.resolve_alias(spread, &mut FxHashSet::default(), tail.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Union(spread_variants, spread_tail) => {
                        for sv in spread_variants {
                            if let Some(existing) =
                                variants.iter().find(|v| v.name == sv.name).cloned()
                            {
                                let existing = self.union_variant_type_name(&existing);
                                let incoming = self.union_variant_type_name(&sv);
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::OverlappingRowField {
                                        item: RowOverlapItem::UnionMember,
                                        source: source.clone(),
                                        name: sv.name.clone(),
                                        existing,
                                        incoming,
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
}
