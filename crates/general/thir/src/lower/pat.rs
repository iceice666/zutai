use std::collections::{HashMap, HashSet};

use zutai_hir::{BindingId, HirPatId, HirPatKind, HirRecordPatField, HirTuplePatItem};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirPat, ThirPatId, ThirPatKind, ThirRecordPatField, ThirTuplePatItem, Type, TypeId, TypeKind,
    TypeTupleItem,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn check_pattern(
        &mut self,
        id: HirPatId,
        expected: TypeId,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> ThirPatId {
        use crate::ir::{Type, TypeKind};
        let pattern = self.hir_pat(id);
        let kind = match &pattern.kind {
            HirPatKind::Wildcard => ThirPatKind::Wildcard,
            HirPatKind::Bind(binding) => {
                self.value_types.insert(*binding, expected);
                scoped_bindings.push(*binding);
                ThirPatKind::Bind(*binding)
            }
            HirPatKind::True => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::True,
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::True
            }
            HirPatKind::False => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::False,
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::False
            }
            HirPatKind::Integer(value) => {
                let ty = self.int_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Integer(*value)
            }
            HirPatKind::Float(value) => {
                let ty = self.float_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Float(*value)
            }
            HirPatKind::String(value) => {
                let ty = self.text_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::String(value.clone())
            }
            HirPatKind::Atom(name) => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::Atom(name.clone()),
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Atom(name.clone())
            }
            HirPatKind::Tuple(items) => {
                return self.check_tuple_pattern(
                    id,
                    items,
                    expected,
                    pattern.span,
                    scoped_bindings,
                );
            }
            HirPatKind::Record(fields) => {
                return self.check_record_pattern(
                    id,
                    fields,
                    expected,
                    pattern.span,
                    scoped_bindings,
                );
            }
            HirPatKind::TaggedValue { tag, payload } => {
                return self.check_tagged_value_pattern(
                    id,
                    tag,
                    payload,
                    expected,
                    pattern.span,
                    scoped_bindings,
                );
            }
        };
        self.alloc_pat(ThirPat {
            source: id,
            ty: expected,
            kind,
            span: pattern.span,
        })
    }

    fn check_tuple_pattern(
        &mut self,
        id: HirPatId,
        items: &[HirTuplePatItem],
        expected: TypeId,
        span: Span,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> ThirPatId {
        let resolved = self.resolve_alias(expected, &mut HashSet::new(), span);
        let type_items = match self.tuple_scrutinee_items(resolved, items, span) {
            Some(type_items) => type_items,
            None => {
                // An already-`Error` scrutinee was diagnosed upstream; stay quiet.
                if !matches!(self.ty(resolved).kind, TypeKind::Error) {
                    let found = self.type_name(expected);
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::ExpectedTuple { found },
                        span,
                    });
                }
                let lowered = self.lower_tuple_pat_items_with_error(items, span, scoped_bindings);
                return self.alloc_pat(ThirPat {
                    source: id,
                    ty: expected,
                    kind: ThirPatKind::Tuple(lowered),
                    span,
                });
            }
        };

        if type_items.len() != items.len() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TupleArityMismatch {
                    expected: type_items.len(),
                    found: items.len(),
                },
                span,
            });
        }

        let lowered: Vec<ThirTuplePatItem> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let expected_item = type_items.get(i);
                match (item, expected_item) {
                    (
                        HirTuplePatItem::Named {
                            name,
                            pattern: pat_id,
                            span: item_span,
                        },
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
                                span: *item_span,
                            });
                        }
                        let pat = self.check_pattern(*pat_id, *ty, scoped_bindings);
                        ThirTuplePatItem::Named {
                            name: name.clone(),
                            pattern: pat,
                            span: *item_span,
                        }
                    }
                    (
                        HirTuplePatItem::Named {
                            name,
                            pattern: pat_id,
                            span: item_span,
                        },
                        Some(TypeTupleItem::Positional(ty)),
                    ) => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                                expected: "<positional>".to_string(),
                                found: name.clone(),
                            },
                            span: *item_span,
                        });
                        let pat = self.check_pattern(*pat_id, *ty, scoped_bindings);
                        ThirTuplePatItem::Named {
                            name: name.clone(),
                            pattern: pat,
                            span: *item_span,
                        }
                    }
                    (HirTuplePatItem::Positional(pat_id), Some(TypeTupleItem::Positional(ty))) => {
                        ThirTuplePatItem::Positional(self.check_pattern(
                            *pat_id,
                            *ty,
                            scoped_bindings,
                        ))
                    }
                    (
                        HirTuplePatItem::Positional(pat_id),
                        Some(TypeTupleItem::Named {
                            name: expected_name,
                            ty,
                            span: type_span,
                        }),
                    ) => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                                expected: expected_name.clone(),
                                found: "<positional>".to_string(),
                            },
                            span: *type_span,
                        });
                        ThirTuplePatItem::Positional(self.check_pattern(
                            *pat_id,
                            *ty,
                            scoped_bindings,
                        ))
                    }
                    (
                        HirTuplePatItem::Named {
                            name,
                            pattern: pat_id,
                            span: item_span,
                        },
                        None,
                    ) => {
                        let pat = self.check_pattern(*pat_id, self.error_type, scoped_bindings);
                        ThirTuplePatItem::Named {
                            name: name.clone(),
                            pattern: pat,
                            span: *item_span,
                        }
                    }
                    (HirTuplePatItem::Positional(pat_id), None) => ThirTuplePatItem::Positional(
                        self.check_pattern(*pat_id, self.error_type, scoped_bindings),
                    ),
                }
            })
            .collect();

        self.alloc_pat(ThirPat {
            source: id,
            ty: expected,
            kind: ThirPatKind::Tuple(lowered),
            span,
        })
    }

    fn lower_tuple_pat_items_with_error(
        &mut self,
        items: &[HirTuplePatItem],
        _span: Span,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> Vec<ThirTuplePatItem> {
        items
            .iter()
            .map(|item| match item {
                HirTuplePatItem::Named {
                    name,
                    pattern,
                    span,
                } => {
                    let pat = self.check_pattern(*pattern, self.error_type, scoped_bindings);
                    ThirTuplePatItem::Named {
                        name: name.clone(),
                        pattern: pat,
                        span: *span,
                    }
                }
                HirTuplePatItem::Positional(pat_id) => ThirTuplePatItem::Positional(
                    self.check_pattern(*pat_id, self.error_type, scoped_bindings),
                ),
            })
            .collect()
    }

    fn check_record_pattern(
        &mut self,
        id: HirPatId,
        fields: &[zutai_hir::HirRecordPatField],
        expected: TypeId,
        span: Span,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> ThirPatId {
        let Some(type_fields) = self.record_fields(expected, span) else {
            let found = self.type_name(expected);
            if !matches!(self.ty(expected).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedRecord { found },
                    span,
                });
            }
            let lowered: Vec<ThirRecordPatField> = fields
                .iter()
                .map(|f| {
                    let pat = self.check_pattern(f.pattern, self.error_type, scoped_bindings);
                    ThirRecordPatField {
                        name: f.name.clone(),
                        pattern: pat,
                        span: f.span,
                    }
                })
                .collect();
            return self.alloc_pat(ThirPat {
                source: id,
                ty: expected,
                kind: ThirPatKind::Record(lowered),
                span,
            });
        };

        use std::collections::HashMap;
        let type_by_name: HashMap<&str, _> =
            type_fields.iter().map(|f| (f.name.as_str(), f)).collect();

        let lowered: Vec<ThirRecordPatField> = fields
            .iter()
            .map(|f| {
                let field_ty = match type_by_name.get(f.name.as_str()) {
                    Some(tf) => tf.ty,
                    None => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::UnknownField {
                                name: f.name.clone(),
                            },
                            span: f.span,
                        });
                        self.error_type
                    }
                };
                let pat = self.check_pattern(f.pattern, field_ty, scoped_bindings);
                ThirRecordPatField {
                    name: f.name.clone(),
                    pattern: pat,
                    span: f.span,
                }
            })
            .collect();

        self.alloc_pat(ThirPat {
            source: id,
            ty: expected,
            kind: ThirPatKind::Record(lowered),
            span,
        })
    }

    fn check_tagged_value_pattern(
        &mut self,
        id: HirPatId,
        tag: &str,
        payload: &[HirRecordPatField],
        expected: TypeId,
        span: Span,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> ThirPatId {
        let resolved = self.resolve_alias(expected, &mut HashSet::new(), span);
        let kind = self.ty(resolved).kind.clone();

        let lowered_payload: Vec<ThirRecordPatField> = match kind {
            TypeKind::Union(variants) => {
                match variants.iter().find(|v| v.name == tag).cloned() {
                    Some(v) => match v.payload {
                        None => {
                            if !payload.is_empty() {
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::UnexpectedRecordField {
                                        name: payload[0].name.clone(),
                                    },
                                    span,
                                });
                            }
                            vec![]
                        }
                        Some(record_ty) => self.check_tagged_payload_fields(
                            payload,
                            record_ty,
                            span,
                            scoped_bindings,
                        ),
                    },
                    None => {
                        // Unknown variant — type mismatch
                        let found = self.alloc_type(Type {
                            kind: TypeKind::Atom(tag.to_string()),
                            span,
                        });
                        self.type_mismatch(expected, found, span);
                        payload
                            .iter()
                            .map(|f| {
                                let pat =
                                    self.check_pattern(f.pattern, self.error_type, scoped_bindings);
                                ThirRecordPatField {
                                    name: f.name.clone(),
                                    pattern: pat,
                                    span: f.span,
                                }
                            })
                            .collect()
                    }
                }
            }
            TypeKind::Optional(inner) => {
                if tag == "none" {
                    vec![]
                } else if tag == "some" {
                    // Optional #some { value = x }
                    payload
                        .iter()
                        .map(|f| {
                            let field_ty = if f.name == "value" {
                                inner
                            } else {
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::UnknownField {
                                        name: f.name.clone(),
                                    },
                                    span: f.span,
                                });
                                self.error_type
                            };
                            let pat = self.check_pattern(f.pattern, field_ty, scoped_bindings);
                            ThirRecordPatField {
                                name: f.name.clone(),
                                pattern: pat,
                                span: f.span,
                            }
                        })
                        .collect()
                } else {
                    let found = self.alloc_type(Type {
                        kind: TypeKind::Atom(tag.to_string()),
                        span,
                    });
                    self.type_mismatch(expected, found, span);
                    vec![]
                }
            }
            TypeKind::Error => vec![],
            _ => {
                let found = self.alloc_type(Type {
                    kind: TypeKind::Atom(tag.to_string()),
                    span,
                });
                self.type_mismatch(expected, found, span);
                payload
                    .iter()
                    .map(|f| {
                        let pat = self.check_pattern(f.pattern, self.error_type, scoped_bindings);
                        ThirRecordPatField {
                            name: f.name.clone(),
                            pattern: pat,
                            span: f.span,
                        }
                    })
                    .collect()
            }
        };

        self.alloc_pat(ThirPat {
            source: id,
            ty: expected,
            kind: ThirPatKind::TaggedValue {
                tag: tag.to_string(),
                payload: lowered_payload,
            },
            span,
        })
    }

    /// Check `payload` pattern fields against the record type `record_ty`.
    fn check_tagged_payload_fields(
        &mut self,
        payload: &[HirRecordPatField],
        record_ty: TypeId,
        span: Span,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> Vec<ThirRecordPatField> {
        let Some(type_fields) = self.record_fields(record_ty, span) else {
            return payload
                .iter()
                .map(|f| {
                    let pat = self.check_pattern(f.pattern, self.error_type, scoped_bindings);
                    ThirRecordPatField {
                        name: f.name.clone(),
                        pattern: pat,
                        span: f.span,
                    }
                })
                .collect();
        };
        let type_by_name: HashMap<&str, TypeId> = type_fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty))
            .collect();
        payload
            .iter()
            .map(|f| {
                let field_ty = match type_by_name.get(f.name.as_str()) {
                    Some(&ty) => ty,
                    None => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::UnknownField {
                                name: f.name.clone(),
                            },
                            span: f.span,
                        });
                        self.error_type
                    }
                };
                let pat = self.check_pattern(f.pattern, field_ty, scoped_bindings);
                ThirRecordPatField {
                    name: f.name.clone(),
                    pattern: pat,
                    span: f.span,
                }
            })
            .collect()
    }

    pub(super) fn clear_scoped_value_types(&mut self, scoped_bindings: &[BindingId]) {
        for binding in scoped_bindings {
            self.value_types.remove(binding);
        }
    }

    fn check_pattern_type(&mut self, expected: TypeId, found: TypeId, span: Span) {
        if !self.type_matches(expected, found) {
            self.type_mismatch(expected, found, span);
        }
    }

    /// Resolve the tuple-shaped field types a tuple pattern is matched against,
    /// narrowing a `Union` scrutinee by the pattern's leading `#tag` and an
    /// `Optional` scrutinee by a leading `#some`. Returns `None` when the
    /// scrutinee is not a tuple-compatible shape for this pattern.
    fn tuple_scrutinee_items(
        &mut self,
        resolved: TypeId,
        items: &[HirTuplePatItem],
        span: Span,
    ) -> Option<Vec<TypeTupleItem>> {
        match self.ty(resolved).kind.clone() {
            TypeKind::Tuple(type_items) => Some(type_items),
            TypeKind::Optional(inner) => match self.pattern_leading_atom_hir(items) {
                Some(tag) if tag == "some" => {
                    let some_ty = self.alloc_type(Type {
                        kind: TypeKind::Atom("some".to_string()),
                        span,
                    });
                    Some(vec![
                        TypeTupleItem::Positional(some_ty),
                        TypeTupleItem::Named {
                            name: "value".to_string(),
                            ty: inner,
                            span,
                        },
                    ])
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The leading positional atom name of a tuple *pattern* (its union tag).
    fn pattern_leading_atom_hir(&self, items: &[HirTuplePatItem]) -> Option<String> {
        let HirTuplePatItem::Positional(first) = items.first()? else {
            return None;
        };
        match &self.hir_pat(*first).kind {
            HirPatKind::Atom(name) => Some(name.clone()),
            _ => None,
        }
    }
}
