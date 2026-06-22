use super::*;

impl<'hir> Lowerer<'hir> {
    pub(super) fn infer_record_expr(
        &mut self,
        id: HirExprId,
        fields: &[HirRecordField],
        span: Span,
    ) -> ThirExprId {
        let mut thir_fields = Vec::with_capacity(fields.len());
        let mut type_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let value = self.infer_expr(field.value);
            let ty = self.expr(value).ty;
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
            type_fields.push(TypeRecordField {
                name: field.name.clone(),
                optional: false,
                ty,
                span: field.span,
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
        items: &[HirExprId],
        span: Span,
    ) -> ThirExprId {
        let Some((first, rest)) = items.split_first() else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::EmptyListNeedsType,
                span,
            });
            return self.error_expr(id, span);
        };
        let first = self.infer_expr(*first);
        let item_ty = self.expr(first).ty;
        let mut lowered_items = Vec::with_capacity(items.len());
        lowered_items.push(first);
        lowered_items.extend(rest.iter().map(|item| self.check_expr(*item, item_ty)));
        let ty = self.alloc_type(Type {
            kind: TypeKind::List(item_ty),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::List(lowered_items),
            span,
        })
    }

    pub(super) fn check_list_expr(
        &mut self,
        id: HirExprId,
        items: &[HirExprId],
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
        let items = items
            .iter()
            .map(|item| self.check_expr(*item, item_ty))
            .collect();
        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::List(items),
            span,
        })
    }

    pub(super) fn check_record_expr(
        &mut self,
        id: HirExprId,
        fields: &[HirRecordField],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some((expected_fields, expected_tail)) = self.record_row(expected, span) else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.infer_record_expr(id, fields, span);
        };

        let expected_by_name: FxHashMap<_, _> = expected_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        let actual_names: FxHashSet<_> = fields.iter().map(|field| field.name.as_str()).collect();

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

        let mut thir_fields = Vec::with_capacity(fields.len());
        // Extra actual fields not named by `expected`: rejected for a closed or
        // rigid row, discarded for an anonymous open row, and captured by a
        // flexible row variable so a named tail preserves them.
        let mut captured_extras: Vec<TypeRecordField> = Vec::new();
        for field in fields {
            let Some(expected_field) = expected_by_name.get(field.name.as_str()) else {
                let value = self.infer_expr(field.value);
                match expected_tail {
                    RowTail::Open => {}
                    RowTail::Infer(_) => captured_extras.push(TypeRecordField {
                        name: field.name.clone(),
                        optional: false,
                        ty: self.expr(value).ty,
                        span: field.span,
                    }),
                    RowTail::Closed | RowTail::Param(_) => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::UnexpectedRecordField {
                                name: field.name.clone(),
                            },
                            span: field.span,
                        });
                    }
                }
                thir_fields.push(ThirRecordField {
                    name: field.name.clone(),
                    value,
                    span: field.span,
                });
                continue;
            };
            let value = self.check_expr(field.value, expected_field.ty);
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
        }

        // Solve a flexible expected tail with whatever the literal supplied beyond
        // the named fields, so a row-polymorphic call preserves the extras.
        if let RowTail::Infer(r) = expected_tail {
            self.row_subst.insert(
                r,
                RowSolution::Record {
                    fields: captured_extras,
                    tail: RowTail::Closed,
                },
            );
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
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
                    kind: ThirDiagnosticKind::RowAnnotationRequired,
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
                    kind: ThirDiagnosticKind::RowAnnotationRequired,
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
