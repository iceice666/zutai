use zutai_hir::BindingId;
use zutai_thir::{RowTail, ThirDeclKind, TypeId, TypeKind};

use crate::ir::{
    BuiltinOp, Literal, PrimTy, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcTupleField,
    TlcTupleItem, TlcType,
};

use super::*;
use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_from_data_method(
        &mut self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<TlcExprId> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        let subst: rustc_hash::FxHashMap<BindingId, TypeId> =
            [(constraint_param, target)].into_iter().collect();
        let data_ty = self.lower_expanded_type_with_subst(from, &subst);
        let result_ty = self.lower_expanded_type_with_subst(to, &subst);
        let target_ty = self.lower_type(target);
        let span = zutai_syntax::Span::default();
        let data_binding = self.fresh_synth_binding();
        let data = self.alloc_expr(TlcExpr::Var(data_binding), data_ty, span);
        let body =
            self.derive_from_data_value(target, target_ty, data, data_ty, result_ty, span)?;
        let fn_ty = self.alloc_type(TlcType::Fun(data_ty, result_ty, Row::REmpty));
        Some(self.alloc_expr(TlcExpr::Lam(data_binding, data_ty, body), fn_ty, span))
    }

    pub(in crate::lower) fn derive_from_data_value(
        &mut self,
        target: TypeId,
        target_ty: crate::ir::TlcTypeId,
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let shape = self.resolve_alias_shape(target);
        if let TypeKind::Record(fields, RowTail::Closed) = shape.clone() {
            return self
                .derive_record_from_data(target_ty, &fields, data, data_ty, result_ty, span);
        }
        if let TypeKind::List(inner) = shape.clone() {
            return self.derive_list_from_data(inner, target_ty, data, data_ty, result_ty, span);
        }
        if let TypeKind::Optional(inner) = shape.clone() {
            return self
                .derive_optional_from_data(inner, target_ty, data, data_ty, result_ty, span);
        }
        if let TypeKind::Union(variants, RowTail::Closed) = shape.clone() {
            return self
                .derive_union_from_data(&variants, target_ty, data, data_ty, result_ty, span);
        }
        if let TypeKind::Atom(name) = shape.clone() {
            return self.derive_atom_from_data(&name, target_ty, data, data_ty, result_ty, span);
        }
        let (tag, expected) = match shape {
            TypeKind::Bool => ("bool", "Bool"),
            TypeKind::Int => ("int", "Int"),
            TypeKind::Float => ("float", "Float"),
            TypeKind::Text => ("text", "Text"),
            _ => return None,
        };
        let payload_ty = self.tlc_row_field_type(data_ty, tag)?;
        let value_ty = self.tlc_row_field_type(payload_ty, "value")?;
        let value_binding = self.fresh_synth_binding();
        let value = self.alloc_expr(TlcExpr::Var(value_binding), value_ty, span);
        let valid = self.from_data_valid(result_ty, target_ty, value, span)?;
        let invalid = self.from_data_invalid(result_ty, expected, span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                data,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            tag.to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "value".to_string(),
                                TlcPat::Bind(value_binding),
                            )])),
                        ),
                        guard: None,
                        body: valid,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: invalid,
                    },
                ],
            ),
            result_ty,
            span,
        ))
    }

    pub(super) fn derive_atom_from_data(
        &mut self,
        expected_atom: &str,
        target_ty: crate::ir::TlcTypeId,
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, "atom")?;
        let text_ty = self.tlc_row_field_type(payload_ty, "value")?;
        let binding = self.fresh_synth_binding();
        let actual = self.alloc_expr(TlcExpr::Var(binding), text_ty, span);
        let expected = self.alloc_expr(
            TlcExpr::Lit(Literal::Str(expected_atom.to_string())),
            text_ty,
            span,
        );
        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
        let equal = self.alloc_expr(
            TlcExpr::Builtin(BuiltinOp::Eq, actual, expected),
            bool_ty,
            span,
        );
        let atom = self.alloc_expr(
            TlcExpr::Lit(Literal::Atom(expected_atom.to_string())),
            target_ty,
            span,
        );
        let valid = self.from_data_valid(result_ty, target_ty, atom, span)?;
        let invalid = self.from_data_invalid(result_ty, expected_atom, span)?;
        let checked = self.alloc_expr(
            TlcExpr::Case(
                equal,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: valid,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: invalid,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let wrong = self.from_data_invalid(result_ty, expected_atom, span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                data,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "atom".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "value".to_string(),
                                TlcPat::Bind(binding),
                            )])),
                        ),
                        guard: None,
                        body: checked,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: wrong,
                    },
                ],
            ),
            result_ty,
            span,
        ))
    }

    pub(super) fn derive_optional_from_data(
        &mut self,
        inner: TypeId,
        target_ty: crate::ir::TlcTypeId,
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let tagged_payload_ty = self.tlc_row_field_type(data_ty, "tagged")?;
        let tag_ty = self.tlc_row_field_type(tagged_payload_ty, "tag")?;
        let payload_data_ty = self.tlc_row_field_type(tagged_payload_ty, "payload")?;
        let tag_binding = self.fresh_synth_binding();
        let payload_binding = self.fresh_synth_binding();
        let tag = self.alloc_expr(TlcExpr::Var(tag_binding), tag_ty, span);
        let payload = self.alloc_expr(TlcExpr::Var(payload_binding), payload_data_ty, span);
        let inner_ty = self.lower_type(inner);
        let inner_result_ty = self.validation_type_with_value(result_ty, inner_ty)?;
        let decoded =
            self.derive_from_data_value(inner, inner_ty, payload, data_ty, inner_result_ty, span)?;
        let path_item_ty = self.decode_path_item_type(inner_result_ty)?;
        let variant_payload_ty = self.tlc_row_field_type(path_item_ty, "variant")?;
        let name_ty = self.tlc_row_field_type(variant_payload_ty, "name")?;
        let name = self.alloc_expr(
            TlcExpr::Lit(Literal::Str("some".to_string())),
            name_ty,
            span,
        );
        let variant_payload = self.alloc_expr(
            TlcExpr::Record(vec![("name".to_string(), name)]),
            variant_payload_ty,
            span,
        );
        let segment = self.alloc_expr(
            TlcExpr::Variant("variant".to_string(), variant_payload),
            path_item_ty,
            span,
        );
        let decoded = self.prefix_validation_segment(decoded, inner_result_ty, segment, span)?;
        let value_binding = self.fresh_synth_binding();
        let inner_value = self.alloc_expr(TlcExpr::Var(value_binding), inner_ty, span);
        let tuple_ty = self.alloc_type(TlcType::Tuple(vec![TlcTupleField::Positional(inner_ty)]));
        let tuple = self.alloc_expr(
            TlcExpr::Tuple(vec![TlcTupleItem::Positional(inner_value)]),
            tuple_ty,
            span,
        );
        let some_value =
            self.alloc_expr(TlcExpr::Variant("some".to_string(), tuple), target_ty, span);
        let some_valid = self.from_data_valid(result_ty, target_ty, some_value, span)?;
        let errors_binding = self.fresh_synth_binding();
        let errors_ty = self.validation_errors_type(result_ty)?;
        let errors = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
        let some_invalid = self.from_data_invalid_with_errors(result_ty, errors, span)?;
        let some_result = self.alloc_expr(
            TlcExpr::Case(
                decoded,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "valid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "value".to_string(),
                                TlcPat::Bind(value_binding),
                            )])),
                        ),
                        guard: None,
                        body: some_valid,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "invalid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "errors".to_string(),
                                TlcPat::Bind(errors_binding),
                            )])),
                        ),
                        guard: None,
                        body: some_invalid,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let none = self.alloc_expr(
            TlcExpr::Lit(Literal::Atom("none".to_string())),
            target_ty,
            span,
        );
        let none_valid = self.from_data_valid(result_ty, target_ty, none, span)?;
        let bad_tag = self.from_data_invalid(result_ty, "Optional tag", span)?;
        let tag_match = self.alloc_expr(
            TlcExpr::Case(
                tag,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Str("none".to_string())),
                        guard: None,
                        body: none_valid,
                    },
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Str("some".to_string())),
                        guard: None,
                        body: some_result,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: bad_tag,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let wrong = self.from_data_invalid(result_ty, "tagged Optional", span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                data,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "tagged".to_string(),
                            Box::new(TlcPat::Record(vec![
                                ("tag".to_string(), TlcPat::Bind(tag_binding)),
                                ("payload".to_string(), TlcPat::Bind(payload_binding)),
                            ])),
                        ),
                        guard: None,
                        body: tag_match,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: wrong,
                    },
                ],
            ),
            result_ty,
            span,
        ))
    }

    pub(super) fn derive_list_from_data(
        &mut self,
        inner: TypeId,
        target_ty: crate::ir::TlcTypeId,
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(data_ty, "list")?;
        let items_ty = self.tlc_row_field_type(payload_ty, "items")?;
        let inner_ty = self.lower_type(inner);
        let items_binding = self.fresh_synth_binding();
        let items = self.alloc_expr(TlcExpr::Var(items_binding), items_ty, span);
        let go_binding = self.fresh_synth_binding();
        let int_ty = self.alloc_type(TlcType::Prim(PrimTy::Int));
        let go_tail_ty = self.alloc_type(TlcType::Fun(items_ty, result_ty, Row::REmpty));
        let go_ty = self.alloc_type(TlcType::Fun(int_ty, go_tail_ty, Row::REmpty));
        let go = self.alloc_expr(TlcExpr::Var(go_binding), go_ty, span);
        let index_binding = self.fresh_synth_binding();
        let xs_binding = self.fresh_synth_binding();
        let head_binding = self.fresh_synth_binding();
        let tail_binding = self.fresh_synth_binding();
        let xs = self.alloc_expr(TlcExpr::Var(xs_binding), items_ty, span);
        let index = self.alloc_expr(TlcExpr::Var(index_binding), int_ty, span);
        let head = self.alloc_expr(TlcExpr::Var(head_binding), data_ty, span);
        let tail = self.alloc_expr(TlcExpr::Var(tail_binding), items_ty, span);
        let empty = self.alloc_expr(TlcExpr::List(Vec::new()), target_ty, span);
        let empty_valid = self.from_data_valid(result_ty, target_ty, empty, span)?;
        let inner_result_ty = self.validation_type_with_value(result_ty, inner_ty)?;
        let head_result =
            self.derive_from_data_value(inner, inner_ty, head, data_ty, inner_result_ty, span)?;
        let path_item_ty = self.decode_path_item_type(result_ty)?;
        let index_payload_ty = self.tlc_row_field_type(path_item_ty, "index")?;
        let index_payload = self.alloc_expr(
            TlcExpr::Record(vec![("index".to_string(), index)]),
            index_payload_ty,
            span,
        );
        let index_segment = self.alloc_expr(
            TlcExpr::Variant("index".to_string(), index_payload),
            path_item_ty,
            span,
        );
        let head_result =
            self.prefix_validation_segment(head_result, inner_result_ty, index_segment, span)?;
        let one_int = self.alloc_expr(TlcExpr::Lit(Literal::Int(1)), int_ty, span);
        let next_index = self.alloc_expr(
            TlcExpr::Builtin(BuiltinOp::Add, index, one_int),
            int_ty,
            span,
        );
        let go_next = self.alloc_expr(TlcExpr::App(go, next_index), go_tail_ty, span);
        let tail_result = self.alloc_expr(TlcExpr::App(go_next, tail), result_ty, span);
        let hv = self.fresh_synth_binding();
        let tv = self.fresh_synth_binding();
        let he = self.fresh_synth_binding();
        let te = self.fresh_synth_binding();
        let hv_expr = self.alloc_expr(TlcExpr::Var(hv), inner_ty, span);
        let tv_expr = self.alloc_expr(TlcExpr::Var(tv), target_ty, span);
        let one = self.alloc_expr(TlcExpr::List(vec![hv_expr]), target_ty, span);
        let combined = self.alloc_expr(TlcExpr::ListAppend(one, tv_expr), target_ty, span);
        let combined_valid = self.from_data_valid(result_ty, target_ty, combined, span)?;
        let errors_ty = self.validation_errors_type(result_ty)?;
        let he_expr = self.alloc_expr(TlcExpr::Var(he), errors_ty, span);
        let te_expr = self.alloc_expr(TlcExpr::Var(te), errors_ty, span);
        let both_errors = self.alloc_expr(TlcExpr::ListAppend(he_expr, te_expr), errors_ty, span);
        let both_invalid = self.from_data_invalid_with_errors(result_ty, both_errors, span)?;
        let head_invalid_only = self.from_data_invalid_with_errors(result_ty, he_expr, span)?;
        let tail_invalid_only = self.from_data_invalid_with_errors(result_ty, te_expr, span)?;
        let tail_when_head_valid = self.alloc_expr(
            TlcExpr::Case(
                tail_result,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "valid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "value".to_string(),
                                TlcPat::Bind(tv),
                            )])),
                        ),
                        guard: None,
                        body: combined_valid,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "invalid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "errors".to_string(),
                                TlcPat::Bind(te),
                            )])),
                        ),
                        guard: None,
                        body: tail_invalid_only,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let tail_when_head_invalid = self.alloc_expr(
            TlcExpr::Case(
                tail_result,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant("valid".to_string(), Box::new(TlcPat::Wildcard)),
                        guard: None,
                        body: head_invalid_only,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "invalid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "errors".to_string(),
                                TlcPat::Bind(te),
                            )])),
                        ),
                        guard: None,
                        body: both_invalid,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let cons_body = self.alloc_expr(
            TlcExpr::Case(
                head_result,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "valid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "value".to_string(),
                                TlcPat::Bind(hv),
                            )])),
                        ),
                        guard: None,
                        body: tail_when_head_valid,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "invalid".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "errors".to_string(),
                                TlcPat::Bind(he),
                            )])),
                        ),
                        guard: None,
                        body: tail_when_head_invalid,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let go_body = self.alloc_expr(
            TlcExpr::Case(
                xs,
                vec![
                    TlcAlt {
                        pat: TlcPat::ListNil,
                        guard: None,
                        body: empty_valid,
                    },
                    TlcAlt {
                        pat: TlcPat::ListCons(
                            Box::new(TlcPat::Bind(head_binding)),
                            Box::new(TlcPat::Bind(tail_binding)),
                        ),
                        guard: None,
                        body: cons_body,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let go_inner = self.alloc_expr(
            TlcExpr::Lam(xs_binding, items_ty, go_body),
            go_tail_ty,
            span,
        );
        let go_lam = self.alloc_expr(TlcExpr::Lam(index_binding, int_ty, go_inner), go_ty, span);
        let zero = self.alloc_expr(TlcExpr::Lit(Literal::Int(0)), int_ty, span);
        let go_zero = self.alloc_expr(TlcExpr::App(go, zero), go_tail_ty, span);
        let call = self.alloc_expr(TlcExpr::App(go_zero, items), result_ty, span);
        let decoded = self.alloc_expr(
            TlcExpr::Letrec {
                bindings: vec![(go_binding, go_ty, go_lam)],
                body: call,
            },
            result_ty,
            span,
        );
        let wrong = self.from_data_invalid(result_ty, "list", span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                data,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "list".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "items".to_string(),
                                TlcPat::Bind(items_binding),
                            )])),
                        ),
                        guard: None,
                        body: decoded,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: wrong,
                    },
                ],
            ),
            result_ty,
            span,
        ))
    }

    pub(super) fn derive_union_from_data(
        &mut self,
        variants: &[zutai_thir::ir::UnionVariant],
        target_ty: crate::ir::TlcTypeId,
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let tagged_ty = self.tlc_row_field_type(data_ty, "tagged")?;
        let tag_ty = self.tlc_row_field_type(tagged_ty, "tag")?;
        let payload_ty = self.tlc_row_field_type(tagged_ty, "payload")?;
        let tag_binding = self.fresh_synth_binding();
        let payload_binding = self.fresh_synth_binding();
        let tag = self.alloc_expr(TlcExpr::Var(tag_binding), tag_ty, span);
        let payload = self.alloc_expr(TlcExpr::Var(payload_binding), payload_ty, span);
        let mut arms = Vec::new();
        for variant in variants {
            let body = if let Some(payload_target) = variant.payload {
                let payload_target_ty = self.lower_type(payload_target);
                let payload_result_ty =
                    self.validation_type_with_value(result_ty, payload_target_ty)?;
                let decoded = self.derive_from_data_value(
                    payload_target,
                    payload_target_ty,
                    payload,
                    data_ty,
                    payload_result_ty,
                    span,
                )?;
                let path_item_ty = self.decode_path_item_type(payload_result_ty)?;
                let variant_payload_ty = self.tlc_row_field_type(path_item_ty, "variant")?;
                let name_ty = self.tlc_row_field_type(variant_payload_ty, "name")?;
                let name = self.alloc_expr(
                    TlcExpr::Lit(Literal::Str(variant.name.clone())),
                    name_ty,
                    span,
                );
                let variant_payload = self.alloc_expr(
                    TlcExpr::Record(vec![("name".to_string(), name)]),
                    variant_payload_ty,
                    span,
                );
                let segment = self.alloc_expr(
                    TlcExpr::Variant("variant".to_string(), variant_payload),
                    path_item_ty,
                    span,
                );
                let decoded =
                    self.prefix_validation_segment(decoded, payload_result_ty, segment, span)?;
                let value_binding = self.fresh_synth_binding();
                let value = self.alloc_expr(TlcExpr::Var(value_binding), payload_target_ty, span);
                let injected = self.alloc_expr(
                    TlcExpr::Variant(variant.name.clone(), value),
                    target_ty,
                    span,
                );
                let valid = self.from_data_valid(result_ty, target_ty, injected, span)?;
                let errors_binding = self.fresh_synth_binding();
                let errors_ty = self.validation_errors_type(result_ty)?;
                let errors = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
                let invalid = self.from_data_invalid_with_errors(result_ty, errors, span)?;
                self.alloc_expr(
                    TlcExpr::Case(
                        decoded,
                        vec![
                            TlcAlt {
                                pat: TlcPat::Variant(
                                    "valid".to_string(),
                                    Box::new(TlcPat::Record(vec![(
                                        "value".to_string(),
                                        TlcPat::Bind(value_binding),
                                    )])),
                                ),
                                guard: None,
                                body: valid,
                            },
                            TlcAlt {
                                pat: TlcPat::Variant(
                                    "invalid".to_string(),
                                    Box::new(TlcPat::Record(vec![(
                                        "errors".to_string(),
                                        TlcPat::Bind(errors_binding),
                                    )])),
                                ),
                                guard: None,
                                body: invalid,
                            },
                        ],
                    ),
                    result_ty,
                    span,
                )
            } else {
                let atom = self.alloc_expr(
                    TlcExpr::Lit(Literal::Atom(variant.name.clone())),
                    target_ty,
                    span,
                );
                self.from_data_valid(result_ty, target_ty, atom, span)?
            };
            arms.push(TlcAlt {
                pat: TlcPat::Lit(Literal::Str(variant.name.clone())),
                guard: None,
                body,
            });
        }
        arms.push(TlcAlt {
            pat: TlcPat::Wildcard,
            guard: None,
            body: self.from_data_invalid(result_ty, "known variant", span)?,
        });
        let by_tag = self.alloc_expr(TlcExpr::Case(tag, arms), result_ty, span);
        let wrong = self.from_data_invalid(result_ty, "tagged union", span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                data,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "tagged".to_string(),
                            Box::new(TlcPat::Record(vec![
                                ("tag".to_string(), TlcPat::Bind(tag_binding)),
                                ("payload".to_string(), TlcPat::Bind(payload_binding)),
                            ])),
                        ),
                        guard: None,
                        body: by_tag,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: wrong,
                    },
                ],
            ),
            result_ty,
            span,
        ))
    }

    pub(super) fn derive_record_from_data(
        &mut self,
        target_ty: crate::ir::TlcTypeId,
        fields: &[zutai_thir::TypeRecordField],
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let data_payload_ty = self.tlc_row_field_type(data_ty, "record")?;
        let data_fields_ty = self.tlc_row_field_type(data_payload_ty, "fields")?;
        let data_field_ty = match self.type_arena[data_fields_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let input_fields_binding = self.fresh_synth_binding();
        let input_fields =
            self.alloc_expr(TlcExpr::Var(input_fields_binding), data_fields_ty, span);

        let mut decoded = Vec::with_capacity(fields.len());
        for field in fields {
            let field_ty = self.lower_type(field.ty);
            let value_ty = if field.optional {
                self.alloc_type(TlcType::Optional(field_ty))
            } else {
                field_ty
            };
            let field_result_ty = self.validation_type_with_value(result_ty, value_ty)?;
            let found = self.find_data_field(
                input_fields,
                data_fields_ty,
                data_field_ty,
                data_ty,
                &field.name,
                span,
            )?;
            let optional_data_ty = self.expr_types[&found];
            let value_binding = self.fresh_synth_binding();
            let field_data = self.alloc_expr(TlcExpr::Var(value_binding), data_ty, span);
            let decoded_present_ty = self.validation_type_with_value(result_ty, field_ty)?;
            let present = if matches!(
                self.thir.type_arena[field.ty.0 as usize].kind,
                TypeKind::Alias(_) | TypeKind::AliasApply { .. }
            ) {
                self.decode_from_data_component_witness(
                    field.ty,
                    field_data,
                    data_ty,
                    decoded_present_ty,
                    span,
                )
                .or_else(|| {
                    self.derive_from_data_value(
                        field.ty,
                        field_ty,
                        field_data,
                        data_ty,
                        decoded_present_ty,
                        span,
                    )
                })?
            } else {
                self.derive_from_data_value(
                    field.ty,
                    field_ty,
                    field_data,
                    data_ty,
                    decoded_present_ty,
                    span,
                )?
            };
            let present = if field.optional {
                self.validation_present_optional(
                    present,
                    decoded_present_ty,
                    field_result_ty,
                    field_ty,
                    value_ty,
                    span,
                )?
            } else {
                present
            };
            let missing = if field.optional {
                let none = self.alloc_expr(
                    TlcExpr::Lit(Literal::Atom("none".to_string())),
                    value_ty,
                    span,
                );
                self.from_data_valid(field_result_ty, value_ty, none, span)?
            } else {
                self.from_data_invalid(field_result_ty, &format!("field {}", field.name), span)?
            };
            let field_result = self.alloc_expr(
                TlcExpr::Case(
                    found,
                    vec![
                        TlcAlt {
                            pat: TlcPat::Atom("none".to_string()),
                            guard: None,
                            body: missing,
                        },
                        TlcAlt {
                            pat: TlcPat::Variant(
                                "some".to_string(),
                                Box::new(TlcPat::Record(vec![(
                                    "0".to_string(),
                                    TlcPat::Bind(value_binding),
                                )])),
                            ),
                            guard: None,
                            body: present,
                        },
                    ],
                ),
                field_result_ty,
                span,
            );
            let field_result =
                self.prefix_validation_field(field_result, field_result_ty, &field.name, span)?;
            decoded.push(DecodedRecordField {
                name: field.name.clone(),
                optional: field.optional,
                field_ty,
                value_ty,
                result_ty: field_result_ty,
                result: field_result,
            });
            let _ = optional_data_ty;
        }

        let errors_ty = self.validation_errors_type(result_ty)?;
        let empty_errors = self.alloc_expr(TlcExpr::List(Vec::new()), errors_ty, span);
        let mut all_errors = empty_errors;
        let mut result_bindings = Vec::with_capacity(decoded.len());
        for field in &decoded {
            let binding = self.fresh_synth_binding();
            result_bindings.push(binding);
            let result_var = self.alloc_expr(TlcExpr::Var(binding), field.result_ty, span);
            let errors_binding = self.fresh_synth_binding();
            let errors_var = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
            let no_errors = self.alloc_expr(TlcExpr::List(Vec::new()), errors_ty, span);
            let field_errors = self.alloc_expr(
                TlcExpr::Case(
                    result_var,
                    vec![
                        TlcAlt {
                            pat: TlcPat::Variant("valid".to_string(), Box::new(TlcPat::Wildcard)),
                            guard: None,
                            body: no_errors,
                        },
                        TlcAlt {
                            pat: TlcPat::Variant(
                                "invalid".to_string(),
                                Box::new(TlcPat::Record(vec![(
                                    "errors".to_string(),
                                    TlcPat::Bind(errors_binding),
                                )])),
                            ),
                            guard: None,
                            body: errors_var,
                        },
                    ],
                ),
                errors_ty,
                span,
            );
            all_errors = self.alloc_expr(
                TlcExpr::ListAppend(all_errors, field_errors),
                errors_ty,
                span,
            );
        }

        let errors_binding = self.fresh_synth_binding();
        let errors_var = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
        let invalid = self.from_data_invalid_with_errors(result_ty, errors_var, span)?;
        let valid = self.record_valid_from_results(
            target_ty,
            result_ty,
            &decoded,
            &result_bindings,
            invalid,
            span,
        )?;
        let errors_head = self.fresh_synth_binding();
        let errors_tail = self.fresh_synth_binding();
        let finish = self.alloc_expr(
            TlcExpr::Case(
                errors_var,
                vec![
                    TlcAlt {
                        pat: TlcPat::ListNil,
                        guard: None,
                        body: valid,
                    },
                    TlcAlt {
                        pat: TlcPat::ListCons(
                            Box::new(TlcPat::Bind(errors_head)),
                            Box::new(TlcPat::Bind(errors_tail)),
                        ),
                        guard: None,
                        body: invalid,
                    },
                ],
            ),
            result_ty,
            span,
        );
        let mut body = self.alloc_expr(
            TlcExpr::Let {
                binding: errors_binding,
                ty: errors_ty,
                value: all_errors,
                body: finish,
            },
            result_ty,
            span,
        );
        for (field, binding) in decoded.iter().zip(result_bindings.iter()).rev() {
            body = self.alloc_expr(
                TlcExpr::Let {
                    binding: *binding,
                    ty: field.result_ty,
                    value: field.result,
                    body,
                },
                result_ty,
                span,
            );
        }

        let wrong_shape = self.from_data_invalid(result_ty, "record", span)?;
        Some(self.alloc_expr(
            TlcExpr::Case(
                data,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "record".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "fields".to_string(),
                                TlcPat::Bind(input_fields_binding),
                            )])),
                        ),
                        guard: None,
                        body,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: wrong_shape,
                    },
                ],
            ),
            result_ty,
            span,
        ))
    }

    pub(super) fn decode_from_data_component_witness(
        &mut self,
        target: TypeId,
        data: TlcExprId,
        data_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let constraint = self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            matches!(decl.kind, ThirDeclKind::Constraint { .. })
                .then_some(decl.binding)
                .filter(|binding| self.thir.binding_names[binding.0 as usize] == "FromData")
        })?;
        let dict = self.try_get_dict_expr(constraint, target, span)?;
        let method_ty = self.alloc_type(TlcType::Fun(data_ty, result_ty, Row::REmpty));
        let method = self.alloc_expr(
            TlcExpr::GetField(dict, "fromData".to_string()),
            method_ty,
            span,
        );
        Some(self.alloc_expr(TlcExpr::App(method, data), result_ty, span))
    }
}
