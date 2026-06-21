use super::*;

impl Lowerer {
    pub(super) fn lower_effect_row(&mut self, row: &ast::EffectRow) -> HirEffectRow {
        let ops = row
            .ops
            .iter()
            .map(|op| HirEffectOp {
                path: op.path.clone(),
                payload: op.payload.as_ref().map(|payload| self.lower_type(payload)),
                signature: op.signature.as_ref().map(|sig| self.lower_type(sig)),
                span: op.span,
            })
            .collect();
        HirEffectRow {
            ops,
            span: row.span,
        }
    }

    /// Lower a row tail, distinguishing a row variable (`...Rest`, an in-scope
    /// type parameter) from a named-type spread (`...Shape`). A tail naming a
    /// non-type binding is reported as an invalid row-tail target.
    pub(super) fn lower_row_tail(&mut self, tail: &ast::RowTail) -> HirRowTail {
        let span = tail.span();
        let kind = match tail {
            ast::RowTail::Anonymous { .. } => HirRowTailKind::Anonymous,
            ast::RowTail::Named { name, .. } => match self.resolve(name) {
                Some(binding) => match self.bindings[binding.0 as usize].kind {
                    BindingKind::TypeParam => HirRowTailKind::Var(binding),
                    BindingKind::TopType | BindingKind::BuiltinType => {
                        HirRowTailKind::Spread(binding)
                    }
                    _ => {
                        self.diagnostics.push(HirDiagnostic {
                            kind: HirDiagnosticKind::InvalidRowTailTarget { name: name.clone() },
                            span,
                        });
                        HirRowTailKind::Unresolved(name.clone())
                    }
                },
                None => {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::UnknownIdentifier { name: name.clone() },
                        span,
                    });
                    HirRowTailKind::Unresolved(name.clone())
                }
            },
        };
        HirRowTail { kind, span }
    }

    pub(super) fn lower_pattern(&mut self, pattern: &ast::Pattern) -> HirPatId {
        let span = pattern.span();
        let kind = match pattern {
            ast::Pattern::Wildcard(_) => HirPatKind::Wildcard,
            ast::Pattern::Ident { name, span } => {
                let binding = self.define_current(name.clone(), BindingKind::Param, *span);
                HirPatKind::Bind(binding)
            }
            ast::Pattern::True(_) => HirPatKind::True,
            ast::Pattern::False(_) => HirPatKind::False,
            ast::Pattern::Integer { value, postfix, .. } => HirPatKind::Integer(*value, *postfix),
            ast::Pattern::Float { value, postfix, .. } => HirPatKind::Float(*value, *postfix),
            ast::Pattern::String { value, .. } => HirPatKind::String(value.clone()),
            ast::Pattern::Atom { name, .. } => HirPatKind::Atom(name.clone()),
            ast::Pattern::TaggedValue { tag, payload, .. } => HirPatKind::TaggedValue {
                tag: tag.clone(),
                payload: payload
                    .iter()
                    .map(|field| HirRecordPatField {
                        name: field.name.clone(),
                        pattern: self.lower_pattern(&field.pattern),
                        span: field.span,
                    })
                    .collect(),
            },
            ast::Pattern::Tuple { items, .. } => HirPatKind::Tuple(
                items
                    .iter()
                    .map(|item| match item {
                        ast::TuplePatternItem::Named {
                            name,
                            pattern,
                            span,
                        } => HirTuplePatItem::Named {
                            name: name.clone(),
                            pattern: self.lower_pattern(pattern),
                            span: *span,
                        },
                        ast::TuplePatternItem::Positional(pattern) => {
                            HirTuplePatItem::Positional(self.lower_pattern(pattern))
                        }
                    })
                    .collect(),
            ),
            ast::Pattern::Record { fields, .. } => HirPatKind::Record(
                fields
                    .iter()
                    .map(|field| HirRecordPatField {
                        name: field.name.clone(),
                        pattern: self.lower_pattern(&field.pattern),
                        span: field.span,
                    })
                    .collect(),
            ),
        };
        self.alloc_pat(HirPat { kind, span })
    }

    pub(super) fn lower_type(&mut self, ty: &ast::TypeExpr) -> HirTypeId {
        let span = ty.span();
        let kind = match ty {
            ast::TypeExpr::Ident { name, span } => match self.resolve(name) {
                Some(binding) => HirTypeKind::BindingRef(binding),
                None => {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::UnknownIdentifier {
                            name: name.to_string(),
                        },
                        span: *span,
                    });
                    HirTypeKind::UnresolvedIdent(name.clone())
                }
            },
            ast::TypeExpr::Record { fields, tail, .. } => {
                let fields = fields
                    .iter()
                    .map(|field| HirTypeRecordField {
                        name: field.name.clone(),
                        optional: field.optional,
                        ty: self.lower_type(&field.ty),
                        span: field.span,
                    })
                    .collect();
                let tail = tail.as_ref().map(|tail| self.lower_row_tail(tail));
                HirTypeKind::Record { fields, tail }
            }
            ast::TypeExpr::Union { variants, tail, .. } => {
                let variants = variants
                    .iter()
                    .map(|v| HirUnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.as_ref().map(|payload| self.lower_type(payload)),
                        span: v.span,
                    })
                    .collect();
                let tail = tail.as_ref().map(|tail| self.lower_row_tail(tail));
                HirTypeKind::Union { variants, tail }
            }
            ast::TypeExpr::Tuple { items, .. } => HirTypeKind::Tuple(
                items
                    .iter()
                    .map(|item| match item {
                        ast::TypeTupleItem::Named { name, ty, span } => HirTypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.lower_type(ty),
                            span: *span,
                        },
                        ast::TypeTupleItem::Positional(ty) => {
                            HirTypeTupleItem::Positional(self.lower_type(ty))
                        }
                    })
                    .collect(),
            ),
            ast::TypeExpr::Optional { inner, .. } => HirTypeKind::Optional(self.lower_type(inner)),
            ast::TypeExpr::Arrow { from, to, .. } => HirTypeKind::Arrow {
                from: self.lower_type(from),
                to: self.lower_type(to),
            },
            ast::TypeExpr::Effect { base, effects, .. } => {
                let base = self.lower_type(base);
                let row = self.lower_effect_row(effects);
                HirTypeKind::Effect { base, row }
            }
            ast::TypeExpr::Select {
                receiver, fields, ..
            } => {
                let receiver = self.lower_type(receiver);
                let fields = self.lower_select_fields(fields);
                HirTypeKind::Select { receiver, fields }
            }
            ast::TypeExpr::Apply { func, arg, .. } => HirTypeKind::Apply {
                func: self.lower_type(func),
                arg: self.lower_type(arg),
            },
            ast::TypeExpr::Access {
                receiver, field, ..
            } => HirTypeKind::Access {
                receiver: self.lower_type(receiver),
                field: field.clone(),
            },
            ast::TypeExpr::Atom { name, .. } => HirTypeKind::Atom(name.clone()),
            ast::TypeExpr::True(_) => HirTypeKind::True,
            ast::TypeExpr::False(_) => HirTypeKind::False,
            ast::TypeExpr::ExprEscape(expr) => HirTypeKind::ExprEscape(self.lower_expr(expr)),
        };
        self.alloc_type(HirTypeExpr { kind, span })
    }
}

pub(super) fn clone_import_source(source: &ast::ImportSource) -> HirImportSource {
    match source {
        ast::ImportSource::String(value) => HirImportSource::String(value.clone()),
        ast::ImportSource::Path(parts) => HirImportSource::Path(parts.clone()),
    }
}
