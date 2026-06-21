use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn seed_builtin_value_types(&mut self) {
        for index in 0..self.hir.bindings.len() {
            let (kind, name) = {
                let binding = &self.hir.bindings[index];
                (binding.kind, binding.name.clone())
            };
            let id = BindingId(index as u32);
            match kind {
                BindingKind::BuiltinType => {
                    self.value_types.insert(id, self.type_type);
                }
                BindingKind::BuiltinValue => {
                    if let Some(ty) = self.builtin_value_type(&name) {
                        self.value_types.insert(id, ty);
                        let scheme = self.free_infer_vars_in(ty);
                        if !scheme.is_empty() {
                            self.poly_schemes.insert(id, scheme);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Type of compiler-provided value bindings (the prelude). Phase 16
    /// re-points `print` to the `io.print` effect instead of an ambient side
    /// effect. Phase 17 adds `fields` / `schema` as ordinary applications over
    /// first-class `Type` values.
    pub(in crate::lower) fn builtin_value_type(&mut self, name: &str) -> Option<TypeId> {
        let span = self.hir.span;
        match name {
            "print" => {
                let text = self.text_type(span);
                let row = self.io_print_effect_row(span);
                let effect_text = self.alloc_type(Type {
                    kind: TypeKind::Effect { base: text, row },
                    span,
                });
                Some(self.alloc_type(Type {
                    kind: TypeKind::Function {
                        from: text,
                        to: effect_text,
                    },
                    span,
                }))
            }
            "fields" => Some(self.fields_builtin_type(span)),
            "schema" => Some(self.schema_builtin_type(span)),
            "overlay" => Some(self.overlay_builtin_type(span, false)),
            "overlayDeep" => Some(self.overlay_builtin_type(span, true)),
            _ => None,
        }
    }

    pub(in crate::lower) fn overlay_builtin_type(&mut self, span: Span, deep: bool) -> TypeId {
        let target = self.fresh_infer_var(span);
        let patch = self.patch_type(target, deep, span);
        let tail = self.alloc_type(Type {
            kind: TypeKind::Function {
                from: target,
                to: target,
            },
            span,
        });
        self.alloc_type(Type {
            kind: TypeKind::Function {
                from: patch,
                to: tail,
            },
            span,
        })
    }

    pub(in crate::lower) fn fields_builtin_type(&mut self, span: Span) -> TypeId {
        let text = self.text_type(span);
        let bool_ty = self.bool_type(span);
        let field_ty = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![
                    TypeRecordField {
                        name: "name".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                    TypeRecordField {
                        name: "Type".to_string(),
                        optional: false,
                        ty: self.type_type,
                        span,
                    },
                    TypeRecordField {
                        name: "optional".to_string(),
                        optional: false,
                        ty: bool_ty,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        let result = self.alloc_type(Type {
            kind: TypeKind::List(field_ty),
            span,
        });
        self.alloc_type(Type {
            kind: TypeKind::Function {
                from: self.type_type,
                to: result,
            },
            span,
        })
    }

    pub(in crate::lower) fn schema_builtin_type(&mut self, span: Span) -> TypeId {
        let text = self.text_type(span);
        let bool_ty = self.bool_type(span);
        let field_schema_ty = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![
                    TypeRecordField {
                        name: "name".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                    TypeRecordField {
                        name: "type".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                    TypeRecordField {
                        name: "optional".to_string(),
                        optional: false,
                        ty: bool_ty,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        let field_list_ty = self.alloc_type(Type {
            kind: TypeKind::List(field_schema_ty),
            span,
        });
        let variant_schema_ty = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![
                    TypeRecordField {
                        name: "name".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                    TypeRecordField {
                        name: "fields".to_string(),
                        optional: false,
                        ty: field_list_ty,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        let variant_list_ty = self.alloc_type(Type {
            kind: TypeKind::List(variant_schema_ty),
            span,
        });
        let kind_ty = self.alloc_type(Type {
            kind: TypeKind::Union(
                vec![
                    UnionVariant {
                        name: "record".to_string(),
                        payload: None,
                        span,
                    },
                    UnionVariant {
                        name: "union".to_string(),
                        payload: None,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        let result = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![
                    TypeRecordField {
                        name: "kind".to_string(),
                        optional: false,
                        ty: kind_ty,
                        span,
                    },
                    TypeRecordField {
                        name: "fields".to_string(),
                        optional: true,
                        ty: field_list_ty,
                        span,
                    },
                    TypeRecordField {
                        name: "variants".to_string(),
                        optional: true,
                        ty: variant_list_ty,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        self.alloc_type(Type {
            kind: TypeKind::Function {
                from: self.type_type,
                to: result,
            },
            span,
        })
    }
}
