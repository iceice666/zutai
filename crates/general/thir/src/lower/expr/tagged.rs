use super::*;

impl<'hir> Lowerer<'hir> {
    /// Lower a `#tag { payload }` expression.
    ///
    /// In check mode (`expected == Some(union_ty)`), the variant's record payload
    /// type is threaded into the payload expression.  In infer mode, the payload
    /// is inferred and a singleton union type is synthesised.
    pub(super) fn lower_tagged_value_expr(
        &mut self,
        id: HirExprId,
        tag: &str,
        payload: HirExprId,
        expected: Option<TypeId>,
        span: Span,
    ) -> ThirExprId {
        use std::collections::HashSet;

        if let Some(expected_ty) = expected {
            let resolved = self.resolve_alias(expected_ty, &mut HashSet::new(), span);
            let kind = self.ty(resolved).kind.clone();

            match kind {
                TypeKind::Union(variants, _) => {
                    let variant = variants.iter().find(|v| v.name == tag).cloned();
                    match variant {
                        Some(v) => {
                            let payload_expr = match v.payload {
                                Some(record_ty) => self.check_expr(payload, record_ty),
                                None => {
                                    // No payload expected — infer it anyway (will unify to unit)
                                    self.infer_expr(payload)
                                }
                            };
                            return self.alloc_expr(ThirExpr {
                                source: id,
                                ty: expected_ty,
                                kind: ThirExprKind::TaggedValue {
                                    tag: tag.to_string(),
                                    payload: payload_expr,
                                },
                                span,
                            });
                        }
                        None => {
                            // Unknown variant — fall through to infer+check below
                        }
                    }
                }
                TypeKind::Optional(inner) if tag == "some" => {
                    let tuple_ty = self.alloc_type(crate::ir::Type {
                        kind: TypeKind::Tuple(vec![TypeTupleItem::Positional(inner)]),
                        span,
                    });
                    let payload_expr = self.check_expr(payload, tuple_ty);
                    return self.alloc_expr(ThirExpr {
                        source: id,
                        ty: expected_ty,
                        kind: ThirExprKind::TaggedValue {
                            tag: tag.to_string(),
                            payload: payload_expr,
                        },
                        span,
                    });
                }
                TypeKind::Maybe(inner) if tag == "present" => {
                    let tuple_ty = self.alloc_type(crate::ir::Type {
                        kind: TypeKind::Tuple(vec![TypeTupleItem::Positional(inner)]),
                        span,
                    });
                    let payload_expr = self.check_expr(payload, tuple_ty);
                    return self.alloc_expr(ThirExpr {
                        source: id,
                        ty: expected_ty,
                        kind: ThirExprKind::TaggedValue {
                            tag: tag.to_string(),
                            payload: payload_expr,
                        },
                        span,
                    });
                }
                _ => {}
            }
        }

        // Infer mode (or unknown variant): infer payload, synthesise a singleton union type.
        let payload_expr = self.infer_expr(payload);
        let payload_ty = self.expr(payload_expr).ty;
        let variant = crate::ir::UnionVariant {
            name: tag.to_string(),
            payload: Some(payload_ty),
            span,
        };
        let ty = self.alloc_type(crate::ir::Type {
            kind: TypeKind::Union(vec![variant], RowTail::Closed),
            span,
        });

        if let Some(expected_ty) = expected
            && !self.type_matches(expected_ty, ty)
        {
            self.type_mismatch(expected_ty, ty, span);
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::TaggedValue {
                tag: tag.to_string(),
                payload: payload_expr,
            },
            span,
        })
    }
}
