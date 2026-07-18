use zutai_hir::BindingId;
use zutai_thir::{RowTail, TypeId, TypeKind};

use crate::ir::{Literal, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcType};

use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_to_data_method(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<TlcExprId> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        let subst: rustc_hash::FxHashMap<BindingId, TypeId> =
            [(constraint_param, target)].into_iter().collect();
        let value_ty = self.lower_expanded_type_with_subst(from, &subst);
        let data_ty = self.lower_expanded_type_with_subst(to, &subst);
        let span = zutai_syntax::Span::default();
        let value_binding = self.fresh_synth_binding();
        let value = self.alloc_expr(TlcExpr::Var(value_binding), value_ty, span);
        let body =
            self.derive_to_data_value(constraint, method_name, target, value, data_ty, span)?;
        let fn_ty = self.alloc_type(TlcType::Fun(value_ty, data_ty, Row::REmpty));
        Some(self.alloc_expr(TlcExpr::Lam(value_binding, value_ty, body), fn_ty, span))
    }

    pub(super) fn derive_to_data_value(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        target: TypeId,
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        match self.resolve_alias_shape(target) {
            TypeKind::Bool => self.synthesize_data_scalar("bool", value, data_ty, span),
            TypeKind::Int => self.synthesize_data_scalar("int", value, data_ty, span),
            TypeKind::Float => self.synthesize_data_scalar("float", value, data_ty, span),
            TypeKind::Text => self.synthesize_data_scalar("text", value, data_ty, span),
            TypeKind::Atom(name) => self.synthesize_data_atom(&name, data_ty, span),
            TypeKind::List(inner) => {
                self.synthesize_data_list(constraint, method_name, inner, value, data_ty, span)
            }
            TypeKind::Optional(inner) => {
                self.synthesize_data_optional(constraint, method_name, inner, value, data_ty, span)
            }
            TypeKind::Record(fields, RowTail::Closed) => {
                self.synthesize_data_record(constraint, method_name, &fields, value, data_ty, span)
            }
            TypeKind::Union(variants, RowTail::Closed) => {
                self.synthesize_data_union(constraint, method_name, &variants, value, data_ty, span)
            }
            _ => None,
        }
    }

    pub(super) fn synthesize_data_scalar(
        &mut self,
        tag: &str,
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, tag)?;
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("value".to_string(), value)]),
            payload_ty,
            span,
        );
        Some(self.alloc_expr(TlcExpr::Variant(tag.to_string(), payload), data_ty, span))
    }

    pub(super) fn synthesize_data_atom(
        &mut self,
        name: &str,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, "atom")?;
        let text_ty = self.tlc_row_field_type(payload_ty, "value")?;
        let value = self.alloc_expr(TlcExpr::Lit(Literal::Str(name.to_string())), text_ty, span);
        self.synthesize_data_scalar("atom", value, data_ty, span)
    }

    pub(super) fn synthesize_data_component(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        target: TypeId,
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        if self.has_witness_binding(constraint, target) {
            let dict = self.get_dict_expr(constraint, target, span);
            let component_ty = self.lower_type(target);
            let method_ty = self.alloc_type(TlcType::Fun(component_ty, data_ty, Row::REmpty));
            let method = self.alloc_expr(
                TlcExpr::GetField(dict, method_name.to_string()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(method, constraint, method_name);
            return Some(self.alloc_expr(TlcExpr::App(method, value), data_ty, span));
        }
        self.derive_to_data_value(constraint, method_name, target, value, data_ty, span)
    }

    pub(super) fn synthesize_data_list(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        inner: TypeId,
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, "list")?;
        let items_ty = self.tlc_row_field_type(payload_ty, "items")?;
        let source_ty = self.expr_types[&value];
        let go_binding = self.fresh_synth_binding();
        let go_ty = self.alloc_type(TlcType::Fun(source_ty, items_ty, Row::REmpty));
        let go = self.alloc_expr(TlcExpr::Var(go_binding), go_ty, span);
        let xs_binding = self.fresh_synth_binding();
        let xs = self.alloc_expr(TlcExpr::Var(xs_binding), source_ty, span);
        let head_binding = self.fresh_synth_binding();
        let tail_binding = self.fresh_synth_binding();
        let inner_ty = self.lower_type(inner);
        let head = self.alloc_expr(TlcExpr::Var(head_binding), inner_ty, span);
        let tail = self.alloc_expr(TlcExpr::Var(tail_binding), source_ty, span);
        let encoded_head =
            self.synthesize_data_component(constraint, method_name, inner, head, data_ty, span)?;
        let encoded_tail = self.alloc_expr(TlcExpr::App(go, tail), items_ty, span);
        let one = self.alloc_expr(TlcExpr::List(vec![encoded_head]), items_ty, span);
        let cons = self.alloc_expr(TlcExpr::ListAppend(one, encoded_tail), items_ty, span);
        let empty = self.alloc_expr(TlcExpr::List(Vec::new()), items_ty, span);
        let body = self.alloc_expr(
            TlcExpr::Case(
                xs,
                vec![
                    TlcAlt {
                        pat: TlcPat::ListNil,
                        guard: None,
                        body: empty,
                    },
                    TlcAlt {
                        pat: TlcPat::ListCons(
                            Box::new(TlcPat::Bind(head_binding)),
                            Box::new(TlcPat::Bind(tail_binding)),
                        ),
                        guard: None,
                        body: cons,
                    },
                ],
            ),
            items_ty,
            span,
        );
        let go_lam = self.alloc_expr(TlcExpr::Lam(xs_binding, source_ty, body), go_ty, span);
        let call = self.alloc_expr(TlcExpr::App(go, value), items_ty, span);
        let items = self.alloc_expr(
            TlcExpr::Letrec {
                bindings: vec![(go_binding, go_ty, go_lam)],
                body: call,
            },
            items_ty,
            span,
        );
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("items".to_string(), items)]),
            payload_ty,
            span,
        );
        Some(self.alloc_expr(TlcExpr::Variant("list".to_string(), payload), data_ty, span))
    }

    pub(super) fn synthesize_data_optional(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        inner: TypeId,
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let none_payload = self.synthesize_data_empty_record(data_ty, span)?;
        let none = self.synthesize_data_tagged("none", none_payload, data_ty, span)?;
        let value_binding = self.fresh_synth_binding();
        let inner_ty = self.lower_type(inner);
        let inner_value = self.alloc_expr(TlcExpr::Var(value_binding), inner_ty, span);
        let some_payload = self.synthesize_data_component(
            constraint,
            method_name,
            inner,
            inner_value,
            data_ty,
            span,
        )?;
        let some = self.synthesize_data_tagged("some", some_payload, data_ty, span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                value,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom("none".to_string()),
                        guard: None,
                        body: none,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "some".to_string(),
                            Box::new(TlcPat::Tuple(vec![TlcPatItem::Positional(TlcPat::Bind(
                                value_binding,
                            ))])),
                        ),
                        guard: None,
                        body: some,
                    },
                ],
            ),
            data_ty,
            span,
        ))
    }

    pub(super) fn synthesize_data_record(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        fields: &[zutai_thir::TypeRecordField],
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, "record")?;
        let fields_ty = self.tlc_row_field_type(payload_ty, "fields")?;
        let field_ty = match self.type_arena[fields_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let name_ty = self.tlc_row_field_type(field_ty, "name")?;
        let mut encoded = Vec::with_capacity(fields.len());
        for field in fields {
            let field_value = self.derive_get_field(value, field.name.as_str(), field.ty);
            let data = self.synthesize_data_component(
                constraint,
                method_name,
                field.ty,
                field_value,
                data_ty,
                span,
            )?;
            let name = self.alloc_expr(
                TlcExpr::Lit(Literal::Str(field.name.clone())),
                name_ty,
                span,
            );
            encoded.push(self.alloc_expr(
                TlcExpr::Record(vec![
                    ("name".to_string(), name),
                    ("value".to_string(), data),
                ]),
                field_ty,
                span,
            ));
        }
        let fields = self.alloc_expr(TlcExpr::List(encoded), fields_ty, span);
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("fields".to_string(), fields)]),
            payload_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Variant("record".to_string(), payload),
            data_ty,
            span,
        ))
    }

    pub(super) fn synthesize_data_union(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        variants: &[zutai_thir::ir::UnionVariant],
        value: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let mut arms = Vec::with_capacity(variants.len());
        for variant in variants {
            let (pat, payload) = if let Some(payload_target) = variant.payload {
                let binding = self.fresh_synth_binding();
                let payload_ty = self.lower_type(payload_target);
                let payload_value = self.alloc_expr(TlcExpr::Var(binding), payload_ty, span);
                let encoded = self.synthesize_data_component(
                    constraint,
                    method_name,
                    payload_target,
                    payload_value,
                    data_ty,
                    span,
                )?;
                (
                    TlcPat::Variant(variant.name.clone(), Box::new(TlcPat::Bind(binding))),
                    encoded,
                )
            } else {
                (
                    TlcPat::Atom(variant.name.clone()),
                    self.synthesize_data_empty_record(data_ty, span)?,
                )
            };
            arms.push(TlcAlt {
                pat,
                guard: None,
                body: self.synthesize_data_tagged(&variant.name, payload, data_ty, span)?,
            });
        }
        Some(self.alloc_expr(TlcExpr::Case(value, arms), data_ty, span))
    }

    pub(super) fn synthesize_data_empty_record(
        &mut self,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, "record")?;
        let fields_ty = self.tlc_row_field_type(payload_ty, "fields")?;
        let fields = self.alloc_expr(TlcExpr::List(Vec::new()), fields_ty, span);
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("fields".to_string(), fields)]),
            payload_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Variant("record".to_string(), payload),
            data_ty,
            span,
        ))
    }

    pub(super) fn synthesize_data_tagged(
        &mut self,
        tag: &str,
        payload: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let tagged_ty = self.tlc_row_field_type(data_ty, "tagged")?;
        let tag_ty = self.tlc_row_field_type(tagged_ty, "tag")?;
        let tag = self.alloc_expr(TlcExpr::Lit(Literal::Str(tag.to_string())), tag_ty, span);
        let tagged = self.alloc_expr(
            TlcExpr::Record(vec![
                ("tag".to_string(), tag),
                ("payload".to_string(), payload),
            ]),
            tagged_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Variant("tagged".to_string(), tagged),
            data_ty,
            span,
        ))
    }
}
