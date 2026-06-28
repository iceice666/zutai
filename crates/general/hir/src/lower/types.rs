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
        let tail = row.tail.as_ref().map(|tail| self.lower_row_tail(tail));
        HirEffectRow {
            ops,
            tail,
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
            ast::Pattern::Posit { literal, .. } => HirPatKind::Posit(*literal),
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
            ast::Pattern::ListNil { .. } => HirPatKind::ListNil,
            ast::Pattern::ListCons { head, tail, .. } => HirPatKind::ListCons {
                head: self.lower_pattern(head),
                tail: self.lower_pattern(tail),
            },
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
                Some(binding)
                    if self.bindings[binding.0 as usize].kind == BindingKind::LevelParam =>
                {
                    // A level variable used in type position (bare `l` where a
                    // type is expected).
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::LevelVarAsType { name: name.clone() },
                        span: *span,
                    });
                    HirTypeKind::UnresolvedIdent(name.clone())
                }
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
            ast::TypeExpr::ForAll { params, body, .. } => {
                self.push_scope();
                let hir_params = self.lower_type_params(params);
                let hir_body = self.lower_type(body);
                self.report_unused_level_params(&hir_params);
                self.pop_scope();
                HirTypeKind::ForAll {
                    params: hir_params,
                    body: hir_body,
                }
            }
            ast::TypeExpr::Access {
                receiver, field, ..
            } => HirTypeKind::Access {
                receiver: self.lower_type(receiver),
                field: field.clone(),
            },
            ast::TypeExpr::Atom { name, .. } => HirTypeKind::Atom(name.clone()),
            ast::TypeExpr::True(_) => HirTypeKind::True,
            ast::TypeExpr::False(_) => HirTypeKind::False,
            ast::TypeExpr::UniverseType { level, .. } => {
                HirTypeKind::UniverseLevel(self.lower_level(level))
            }
            ast::TypeExpr::ExprEscape(expr) => HirTypeKind::ExprEscape(self.lower_expr(expr)),
        };
        self.alloc_type(HirTypeExpr { kind, span })
    }

    /// Resolve a surface `Level` to a `HirLevel`, resolving `$…` variable uses to
    /// their declared level binders. Emits `NonLevelAsLevel` / `UnknownLevelVar`
    /// for a `$`-name that is not a level binder or is undeclared.
    fn lower_level(&mut self, level: &ast::Level) -> HirLevel {
        match level {
            ast::Level::Known { value, .. } => HirLevel::Known(*value),
            ast::Level::Var { name, span } => match self.resolve(name) {
                Some(binding)
                    if self.bindings[binding.0 as usize].kind == BindingKind::LevelParam =>
                {
                    self.used_level_params.insert(binding);
                    HirLevel::Var(binding)
                }
                Some(_) => {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::NonLevelAsLevel { name: name.clone() },
                        span: *span,
                    });
                    HirLevel::Unresolved(name.clone())
                }
                None => {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::UnknownLevelVar { name: name.clone() },
                        span: *span,
                    });
                    HirLevel::Unresolved(name.clone())
                }
            },
            ast::Level::Succ { base, by, .. } => HirLevel::Succ {
                base: Box::new(self.lower_level(base)),
                by: *by,
            },
            ast::Level::Max { left, right, .. } => HirLevel::Max {
                left: Box::new(self.lower_level(left)),
                right: Box::new(self.lower_level(right)),
            },
        }
    }
}

pub(super) fn clone_import_source(source: &ast::ImportSource) -> HirImportSource {
    match source {
        ast::ImportSource::String(value) => HirImportSource::String(value.clone()),
        ast::ImportSource::Path(parts) => HirImportSource::Path(parts.clone()),
    }
}
