use super::*;

#[derive(Clone)]
struct LoweredRecordEntry {
    name: String,
    value: ThirExprId,
    ty: TypeId,
    span: Span,
    from_spread: bool,
}

impl<'hir> Lowerer<'hir> {
    pub(super) fn infer_record_expr(
        &mut self,
        id: HirExprId,
        items: &[HirRecordItem],
        span: Span,
    ) -> ThirExprId {
        let entries = self.lower_record_items(id, items, None, span);
        let thir_fields = entries
            .iter()
            .map(|entry| ThirRecordField {
                name: entry.name.clone(),
                value: entry.value,
                span: entry.span,
            })
            .collect();
        let type_fields = entries
            .iter()
            .map(|entry| TypeRecordField {
                name: entry.name.clone(),
                optional: false,
                ty: entry.ty,
                span: entry.span,
            })
            .collect();
        let ty = self.alloc_type(Type {
            kind: TypeKind::Record(type_fields, RowTail::Closed),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    pub(super) fn infer_tuple_expr(
        &mut self,
        id: HirExprId,
        items: &[HirTupleItem],
        span: Span,
    ) -> ThirExprId {
        let mut thir_items = Vec::with_capacity(items.len());
        let mut type_items = Vec::with_capacity(items.len());
        for item in items {
            match item {
                HirTupleItem::Named { name, value, span } => {
                    let value = self.infer_expr(*value);
                    let ty = self.expr(value).ty;
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                    type_items.push(TypeTupleItem::Named {
                        name: name.clone(),
                        ty,
                        span: *span,
                    });
                }
                HirTupleItem::Positional(value) => {
                    let value = self.infer_expr(*value);
                    let ty = self.expr(value).ty;
                    thir_items.push(crate::ir::ThirTupleItem::Positional(value));
                    type_items.push(TypeTupleItem::Positional(ty));
                }
            }
        }
        let ty = self.alloc_type(Type {
            kind: TypeKind::Tuple(type_items),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Tuple(thir_items),
            span,
        })
    }

    pub(super) fn check_tuple_expr(
        &mut self,
        id: HirExprId,
        items: &[HirTupleItem],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let resolved = self.resolve_alias_for_expr(expected);
        // An unsolved expected type (e.g. an inferred lambda parameter) is not yet
        // known to be a tuple. Infer the literal's own type and unify it, rather
        // than rejecting a well-typed `()` / tuple argument as "expected tuple".
        if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
            let inferred = self.infer_tuple_expr(id, items, span);
            let inferred_ty = self.expr(inferred).ty;
            self.unify(inferred_ty, resolved, span);
            return inferred;
        }
        let TypeKind::Tuple(expected_items) = self.ty(resolved).kind.clone() else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedTuple { found },
                span,
            });
            return self.infer_tuple_expr(id, items, span);
        };
        if expected_items.len() != items.len() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TupleArityMismatch {
                    expected: expected_items.len(),
                    found: items.len(),
                },
                span,
            });
        }

        let mut thir_items = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let expected_item = expected_items.get(index);
            match (item, expected_item) {
                (
                    HirTupleItem::Named { name, value, span },
                    Some(TypeTupleItem::Named {
                        name: expected_name,
                        ty,
                        ..
                    }),
                ) => {
                    if name != expected_name {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                                expected: expected_name.clone(),
                                found: name.clone(),
                            },
                            span: *span,
                        });
                    }
                    let value = self.check_expr(*value, *ty);
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                }
                (
                    HirTupleItem::Named { name, value, span },
                    Some(TypeTupleItem::Positional(ty)),
                ) => {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                            expected: "<positional>".to_string(),
                            found: name.clone(),
                        },
                        span: *span,
                    });
                    let value = self.check_expr(*value, *ty);
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                }
                (HirTupleItem::Positional(value), Some(TypeTupleItem::Positional(ty))) => {
                    thir_items.push(crate::ir::ThirTupleItem::Positional(
                        self.check_expr(*value, *ty),
                    ));
                }
                (
                    HirTupleItem::Positional(value),
                    Some(TypeTupleItem::Named {
                        name: expected_name,
                        ty,
                        ..
                    }),
                ) => {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                            expected: expected_name.clone(),
                            found: "<positional>".to_string(),
                        },
                        span,
                    });
                    thir_items.push(crate::ir::ThirTupleItem::Positional(
                        self.check_expr(*value, *ty),
                    ));
                }
                (HirTupleItem::Named { name, value, span }, None) => {
                    let value = self.infer_expr(*value);
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                }
                (HirTupleItem::Positional(value), None) => {
                    thir_items.push(crate::ir::ThirTupleItem::Positional(
                        self.infer_expr(*value),
                    ));
                }
            }
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Tuple(thir_items),
            span,
        })
    }

    pub(super) fn infer_list_expr(
        &mut self,
        id: HirExprId,
        items: &[HirListItem],
        span: Span,
    ) -> ThirExprId {
        if items.is_empty() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::EmptyListNeedsType,
                span,
            });
            return self.error_expr(id, span);
        }

        let mut item_ty = None;
        let mut chunk = Vec::new();
        let mut parts = Vec::new();
        let mut saw_spread = false;

        for item in items {
            match item {
                HirListItem::Item(value) => {
                    let lowered = if let Some(ty) = item_ty {
                        self.check_expr(*value, ty)
                    } else {
                        let lowered = self.infer_expr(*value);
                        item_ty = Some(self.expr(lowered).ty);
                        lowered
                    };
                    chunk.push(lowered);
                }
                HirListItem::Spread(spread) => {
                    saw_spread = true;
                    if let Some(ty) = item_ty {
                        let list_ty = self.alloc_type(Type {
                            kind: TypeKind::List(ty),
                            span: spread.span,
                        });
                        self.flush_list_chunk(id, &mut chunk, list_ty, span, &mut parts);
                        parts.push(self.check_expr(spread.value, list_ty));
                    } else {
                        self.flush_list_chunk_if_typed(id, &mut chunk, item_ty, span, &mut parts);
                        let spread_value = self.infer_expr(spread.value);
                        let spread_ty = self.expr(spread_value).ty;
                        let Some(spread_item_ty) = self.list_item_type(spread_ty, spread.span)
                        else {
                            let found = self.type_name(spread_ty);
                            self.diagnostics.push(ThirDiagnostic {
                                kind: ThirDiagnosticKind::ExpectedList { found },
                                span: spread.span,
                            });
                            continue;
                        };
                        item_ty = Some(spread_item_ty);
                        parts.push(spread_value);
                    }
                }
            }
        }

        let Some(item_ty) = item_ty else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::EmptyListNeedsType,
                span,
            });
            return self.error_expr(id, span);
        };
        let ty = self.alloc_type(Type {
            kind: TypeKind::List(item_ty),
            span,
        });
        if !saw_spread {
            return self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::List(chunk),
                span,
            });
        }
        self.flush_list_chunk(id, &mut chunk, ty, span, &mut parts);
        self.list_expr_from_parts(id, parts, ty, span)
    }

    pub(super) fn check_spread_only_expr(
        &mut self,
        id: HirExprId,
        spreads: &[HirValueSpread],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let resolved = self.resolve_alias_for_expr(expected);
        let resolved = self.resolve(resolved);
        if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::SpreadOnlyLiteralNeedsType,
                span,
            });
            return self.error_expr(id, span);
        }
        if self.record_row(expected, span).is_some() {
            let items: Vec<_> = spreads.iter().cloned().map(HirRecordItem::Spread).collect();
            return self.check_record_expr(id, &items, expected);
        }
        if self.list_item_type(expected, span).is_some() {
            let items: Vec<_> = spreads.iter().cloned().map(HirListItem::Spread).collect();
            return self.check_list_expr(id, &items, expected);
        }
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::SpreadOnlyLiteralNeedsType,
            span,
        });
        self.error_expr(id, span)
    }

    pub(super) fn check_list_expr(
        &mut self,
        id: HirExprId,
        items: &[HirListItem],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some(item_ty) = self.list_item_type(expected, span) else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedList { found },
                span,
            });
            return self.infer_list_expr(id, items, span);
        };
        let mut chunk = Vec::new();
        let mut parts = Vec::new();
        let mut saw_spread = false;
        for item in items {
            match item {
                HirListItem::Item(value) => chunk.push(self.check_expr(*value, item_ty)),
                HirListItem::Spread(spread) => {
                    saw_spread = true;
                    self.flush_list_chunk(id, &mut chunk, expected, span, &mut parts);
                    parts.push(self.check_expr(spread.value, expected));
                }
            }
        }
        if !saw_spread {
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: expected,
                kind: ThirExprKind::List(chunk),
                span,
            });
        }
        self.flush_list_chunk(id, &mut chunk, expected, span, &mut parts);
        self.list_expr_from_parts(id, parts, expected, span)
    }

    fn flush_list_chunk_if_typed(
        &mut self,
        id: HirExprId,
        chunk: &mut Vec<ThirExprId>,
        item_ty: Option<TypeId>,
        span: Span,
        parts: &mut Vec<ThirExprId>,
    ) {
        if let Some(item_ty) = item_ty {
            let list_ty = self.alloc_type(Type {
                kind: TypeKind::List(item_ty),
                span,
            });
            self.flush_list_chunk(id, chunk, list_ty, span, parts);
        }
    }

    fn flush_list_chunk(
        &mut self,
        id: HirExprId,
        chunk: &mut Vec<ThirExprId>,
        list_ty: TypeId,
        span: Span,
        parts: &mut Vec<ThirExprId>,
    ) {
        if chunk.is_empty() {
            return;
        }
        let items = std::mem::take(chunk);
        parts.push(self.alloc_expr(ThirExpr {
            source: id,
            ty: list_ty,
            kind: ThirExprKind::List(items),
            span,
        }));
    }

    fn list_expr_from_parts(
        &mut self,
        id: HirExprId,
        mut parts: Vec<ThirExprId>,
        ty: TypeId,
        span: Span,
    ) -> ThirExprId {
        if parts.is_empty() {
            return self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::List(Vec::new()),
                span,
            });
        }
        let mut current = parts.remove(0);
        for right in parts {
            current = self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::ListAppend {
                    left: current,
                    right,
                },
                span,
            });
        }
        current
    }

    pub(super) fn check_record_expr(
        &mut self,
        id: HirExprId,
        items: &[HirRecordItem],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some((expected_fields, expected_tail)) = self.record_row(expected, span) else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.infer_record_expr(id, items, span);
        };

        let expected_by_name: FxHashMap<_, _> = expected_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        let entries = self.lower_record_items(id, items, Some(&expected_by_name), span);
        let actual_names: FxHashSet<_> = entries.iter().map(|field| field.name.as_str()).collect();

        for expected_field in &expected_fields {
            if !expected_field.optional && !actual_names.contains(expected_field.name.as_str()) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::MissingRecordField {
                        name: expected_field.name.clone(),
                    },
                    span,
                });
            }
        }

        let mut captured_extras: Vec<TypeRecordField> = Vec::new();
        for entry in &entries {
            let expected_ty = expected_by_name
                .get(entry.name.as_str())
                .map(|field| field.ty);
            if let Some(expected_ty) = expected_ty {
                if entry.from_spread && !self.type_matches(expected_ty, entry.ty) {
                    self.type_mismatch(expected_ty, entry.ty, entry.span);
                }
                continue;
            }
            match expected_tail {
                RowTail::Open => {}
                RowTail::Infer(_) => captured_extras.push(TypeRecordField {
                    name: entry.name.clone(),
                    optional: false,
                    ty: entry.ty,
                    span: entry.span,
                }),
                RowTail::Closed | RowTail::Param(_) => {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::UnexpectedRecordField {
                            name: entry.name.clone(),
                        },
                        span: entry.span,
                    });
                }
            }
        }

        if let RowTail::Infer(r) = expected_tail {
            self.row_subst.insert(
                r,
                RowSolution::Record {
                    fields: captured_extras,
                    tail: RowTail::Closed,
                },
            );
        }

        let thir_fields = entries
            .into_iter()
            .map(|entry| ThirRecordField {
                name: entry.name,
                value: entry.value,
                span: entry.span,
            })
            .collect();
        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    fn lower_record_items(
        &mut self,
        id: HirExprId,
        items: &[HirRecordItem],
        expected_by_name: Option<&FxHashMap<&str, &TypeRecordField>>,
        span: Span,
    ) -> Vec<LoweredRecordEntry> {
        let mut entries = Vec::with_capacity(items.len());
        for item in items {
            match item {
                HirRecordItem::Field(field) => {
                    let expected = expected_by_name
                        .and_then(|fields| fields.get(field.name.as_str()))
                        .map(|field| field.ty);
                    let value = if let Some(expected) = expected {
                        self.check_expr(field.value, expected)
                    } else {
                        self.infer_expr(field.value)
                    };
                    let ty = self.expr(value).ty;
                    self.insert_record_entry(
                        &mut entries,
                        LoweredRecordEntry {
                            name: field.name.clone(),
                            value,
                            ty,
                            span: field.span,
                            from_spread: false,
                        },
                    );
                }
                HirRecordItem::Spread(spread) => {
                    self.lower_record_value_spread(id, spread, &mut entries, span);
                }
            }
        }
        entries
    }

    fn lower_record_value_spread(
        &mut self,
        id: HirExprId,
        spread: &HirValueSpread,
        entries: &mut Vec<LoweredRecordEntry>,
        span: Span,
    ) {
        let value = self.infer_expr(spread.value);
        let value_ty = self.expr(value).ty;
        let Some((fields, tail)) = self.record_row(value_ty, spread.span) else {
            let found = self.type_name(value_ty);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span: spread.span,
            });
            return;
        };
        if tail != RowTail::Closed {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::InvalidTypeExpression {
                    reason: "record value spread requires a closed record type",
                },
                span: spread.span,
            });
            return;
        }
        for field in fields {
            let field_ty = if field.optional {
                self.maybe_type(field.ty, field.span)
            } else {
                field.ty
            };
            let access = self.alloc_expr(ThirExpr {
                source: id,
                ty: field_ty,
                kind: ThirExprKind::Access {
                    receiver: value,
                    field: field.name.clone(),
                },
                span,
            });
            self.insert_record_entry(
                entries,
                LoweredRecordEntry {
                    name: field.name,
                    value: access,
                    ty: field_ty,
                    span: field.span,
                    from_spread: true,
                },
            );
        }
    }

    fn insert_record_entry(
        &mut self,
        entries: &mut Vec<LoweredRecordEntry>,
        entry: LoweredRecordEntry,
    ) {
        if let Some(existing) = entries
            .iter()
            .position(|candidate| candidate.name == entry.name)
        {
            entries.remove(existing);
        }
        entries.push(entry);
    }

    pub(super) fn lower_record_update_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        fields: &[HirRecordField],
        span: Span,
    ) -> ThirExprId {
        let receiver = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver).ty;
        let Some((record_fields, _tail)) = self.record_row(receiver_ty, span) else {
            let resolved = self.resolve(receiver_ty);
            if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::RowAnnotationRequired { field: None },
                    span,
                });
            } else {
                let found = self.type_name(receiver_ty);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedRecord { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        };

        if fields.is_empty() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnsupportedFeature {
                    feature: "empty record update",
                },
                span,
            });
            return self.error_expr(id, span);
        }

        let by_name: FxHashMap<&str, &TypeRecordField> = record_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        let mut thir_fields = Vec::with_capacity(fields.len());
        let mut had_unknown = false;
        for field in fields {
            let Some(record_field) = by_name.get(field.name.as_str()) else {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::UnknownField {
                        name: field.name.clone(),
                    },
                    span: field.span,
                });
                self.infer_expr(field.value);
                had_unknown = true;
                continue;
            };
            let value = self.check_expr(field.value, record_field.ty);
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
        }

        if had_unknown {
            return self.error_expr(id, span);
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: receiver_ty,
            kind: ThirExprKind::RecordUpdate {
                receiver,
                fields: thir_fields,
            },
            span,
        })
    }

    pub(super) fn lower_access_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        field: &str,
        span: Span,
    ) -> ThirExprId {
        let receiver = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver).ty;
        let Some(fields) = self.record_fields(receiver_ty, span) else {
            let resolved = self.resolve(receiver_ty);
            if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
                // Field access on an un-inferred value: row-polymorphic inference
                // is not principal here, so an explicit annotation is required.
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::RowAnnotationRequired {
                        field: Some(field.to_string()),
                    },
                    span,
                });
            } else {
                let found = self.type_name(receiver_ty);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedRecord { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        };
        let Some(record_field) = fields.iter().find(|candidate| candidate.name == field) else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnknownField {
                    name: field.to_string(),
                },
                span,
            });
            return self.error_expr(id, span);
        };
        let ty = if record_field.optional {
            self.maybe_type(record_field.ty, record_field.span)
        } else {
            record_field.ty
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Access {
                receiver,
                field: field.to_string(),
            },
            span,
        })
    }

    pub(super) fn check_access_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        field: &str,
        expected: TypeId,
        span: Span,
    ) -> ThirExprId {
        let receiver_tail = self.fresh_row_var();
        let receiver_expected = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![TypeRecordField {
                    name: field.to_string(),
                    optional: false,
                    ty: expected,
                    span,
                }],
                receiver_tail,
            ),
            span,
        });

        let receiver = if matches!(self.hir_expr(receiver).kind, HirExprKind::Access { .. }) {
            self.check_expr(receiver, receiver_expected)
        } else {
            let receiver = self.infer_expr(receiver);
            let receiver_ty = self.expr(receiver).ty;
            let resolved = self.resolve(receiver_ty);
            if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
                self.type_matches(receiver_expected, receiver_ty);
            }
            receiver
        };
        let receiver_ty = self.expr(receiver).ty;
        let Some(fields) = self.record_fields(receiver_ty, span) else {
            let resolved = self.resolve(receiver_ty);
            if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::RowAnnotationRequired {
                        field: Some(field.to_string()),
                    },
                    span,
                });
            } else {
                let found = self.type_name(receiver_ty);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedRecord { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        };
        let Some(record_field) = fields.iter().find(|candidate| candidate.name == field) else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnknownField {
                    name: field.to_string(),
                },
                span,
            });
            return self.error_expr(id, span);
        };
        let ty = if record_field.optional {
            self.maybe_type(record_field.ty, record_field.span)
        } else {
            record_field.ty
        };
        if !self.type_matches(expected, ty) {
            self.type_mismatch(expected, ty, span);
        }
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Access {
                receiver,
                field: field.to_string(),
            },
            span,
        })
    }

    /// Type-check `select receiver { f1; f2; }` as a closed record built from the
    /// selected fields in requested order. Desugars to record construction over
    /// field accesses so downstream stages reuse existing record/access nodes.
    pub(super) fn lower_select_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        fields: &[HirSelectField],
        span: Span,
    ) -> ThirExprId {
        let receiver = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver).ty;
        let Some(rec_fields) = self.record_fields(receiver_ty, span) else {
            let found = self.type_name(receiver_ty);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.error_expr(id, span);
        };
        let mut thir_fields = Vec::with_capacity(fields.len());
        let mut type_fields = Vec::with_capacity(fields.len());
        for sf in fields {
            let Some(rf) = rec_fields.iter().find(|f| f.name == sf.name) else {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::UnknownField {
                        name: sf.name.clone(),
                    },
                    span: sf.span,
                });
                continue;
            };
            let field_ty = if rf.optional {
                self.maybe_type(rf.ty, rf.span)
            } else {
                rf.ty
            };
            let access = self.alloc_expr(ThirExpr {
                source: id,
                ty: field_ty,
                kind: ThirExprKind::Access {
                    receiver,
                    field: sf.name.clone(),
                },
                span: sf.span,
            });
            thir_fields.push(ThirRecordField {
                name: sf.name.clone(),
                value: access,
                span: sf.span,
            });
            type_fields.push(TypeRecordField {
                name: sf.name.clone(),
                optional: false,
                ty: field_ty,
                span: sf.span,
            });
        }
        let ty = self.alloc_type(Type {
            kind: TypeKind::Record(type_fields, RowTail::Closed),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    pub(super) fn lower_opt_access_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        field: &str,
        span: Span,
    ) -> ThirExprId {
        let receiver_thir = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver_thir).ty;

        let Some((wrapper_kind, inner)) = self.optional_or_maybe_inner_type(receiver_ty, span)
        else {
            let found = self.type_name(receiver_ty);
            if !matches!(self.ty(receiver_ty).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedOptionalOrMaybe { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        };

        let Some(fields) = self.record_fields(inner, span) else {
            let found = self.type_name(inner);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.error_expr(id, span);
        };

        let Some(record_field) = fields.iter().find(|f| f.name == field).cloned() else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnknownField {
                    name: field.to_string(),
                },
                span,
            });
            return self.error_expr(id, span);
        };

        let field_ty = if record_field.optional {
            self.maybe_type(record_field.ty, record_field.span)
        } else {
            record_field.ty
        };
        let ty = match wrapper_kind {
            WrapperKind::Optional => self.optional_type(field_ty, span),
            WrapperKind::Maybe => self.maybe_type(field_ty, span),
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::OptionalAccess {
                receiver: receiver_thir,
                field: field.to_string(),
            },
            span,
        })
    }

    pub(super) fn resolve_alias_for_expr(&mut self, ty: TypeId) -> TypeId {
        use rustc_hash::FxHashSet;

        self.resolve_alias(ty, &mut FxHashSet::default(), self.ty(ty).span)
    }
}
