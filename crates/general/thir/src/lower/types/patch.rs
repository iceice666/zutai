use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn patch_type(
        &mut self,
        target: TypeId,
        deep: bool,
        span: Span,
    ) -> TypeId {
        let resolved = self.resolve_alias(target, &mut FxHashSet::default(), span);
        match self.ty(resolved).kind {
            TypeKind::Record(_, _)
            | TypeKind::InferVar(_)
            | TypeKind::TypeVar(_)
            | TypeKind::Alias(_)
            | TypeKind::AliasApply { .. }
            | TypeKind::Apply { .. }
            | TypeKind::Con(_)
            | TypeKind::Error => {}
            _ => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::InvalidTypeExpression {
                        reason: if deep {
                            "DeepPatch requires a record type"
                        } else {
                            "Patch requires a record type"
                        },
                    },
                    span,
                });
            }
        }
        self.alloc_type(Type {
            kind: TypeKind::Patch { target, deep },
            span,
        })
    }

    pub(in crate::lower) fn expand_patch_type(
        &mut self,
        target: TypeId,
        deep: bool,
        span: Span,
    ) -> Option<(Vec<TypeRecordField>, RowTail)> {
        let resolved = self.resolve_alias(target, &mut FxHashSet::default(), span);
        let TypeKind::Record(fields, tail) = self.ty(resolved).kind.clone() else {
            return None;
        };
        let (fields, tail) = self.flatten_record_row(fields, tail);
        let patch_fields = fields
            .into_iter()
            .map(|field| {
                let ty = if deep {
                    let resolved_field =
                        self.resolve_alias(field.ty, &mut FxHashSet::default(), field.span);
                    if matches!(self.ty(resolved_field).kind, TypeKind::Record(_, _)) {
                        self.alloc_type(Type {
                            kind: TypeKind::Patch {
                                target: field.ty,
                                deep: true,
                            },
                            span: field.span,
                        })
                    } else {
                        field.ty
                    }
                } else {
                    field.ty
                };
                TypeRecordField {
                    name: field.name,
                    optional: true,
                    ty,
                    span: field.span,
                }
            })
            .collect();
        Some((patch_fields, tail))
    }
}
