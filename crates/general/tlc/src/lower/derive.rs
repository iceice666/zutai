use zutai_hir::BindingId;
use zutai_thir::{RowTail, ThirConstraintMethod, ThirDeclKind, TypeId, TypeKind, TypeTupleItem};

use crate::ir::{
    BuiltinOp, Literal, PrimTy, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcTupleField,
    TlcTupleItem, TlcType,
};

use super::Lowerer;

#[derive(Clone)]
enum DeriveShape {
    Leaf,
    Record(Vec<(String, TypeId)>),
    Tuple(Vec<(Option<String>, TypeId)>),
    Union(Vec<DeriveVariant>),
}

#[derive(Clone)]
struct DeriveVariant {
    name: String,
    payload_fields: Vec<(String, TypeId)>,
}

#[derive(Clone)]
struct DecodedRecordField {
    name: String,
    optional: bool,
    field_ty: crate::ir::TlcTypeId,
    value_ty: crate::ir::TlcTypeId,
    result_ty: crate::ir::TlcTypeId,
    result: TlcExprId,
}

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_derive_fields(
        &mut self,
        constraint: BindingId,
        target: TypeId,
    ) -> Vec<(String, TlcExprId)> {
        let Some((constraint_param, methods)) = self.constraint_info(constraint) else {
            return Vec::new();
        };

        if self.thir.binding_names[constraint.0 as usize] == "FromData" {
            return methods
                .iter()
                .filter(|method| method.name == "fromData")
                .filter_map(|method| {
                    self.synthesize_from_data_method(method.sig, constraint_param, target)
                        .map(|value| (method.name.clone(), value))
                })
                .collect();
        }

        if self.constraint_has_recipe(constraint) {
            if let Some(fields) = self.lower_quoted_recipe_record(constraint) {
                return fields;
            }
            return methods
                .iter()
                .filter_map(|method| match derive_recipe_kind(&method.name) {
                    Some(DeriveRecipeKind::Show) => {
                        let value =
                            self.synthesize_show_method(method.sig, constraint_param, target)?;
                        Some((method.name.clone(), value))
                    }
                    Some(DeriveRecipeKind::Ord) => {
                        let value = self.synthesize_ord_method(
                            constraint,
                            &method.name,
                            method.sig,
                            constraint_param,
                            target,
                        )?;
                        Some((method.name.clone(), value))
                    }
                    None => None,
                })
                .collect();
        }

        let Some(eq_method_name) = methods
            .iter()
            .find(|method| matches!(derive_equality_kind(&method.name), Some(EqualityKind::Eq)))
            .map(|method| method.name.clone())
        else {
            // No positive `eq`/`==` recipe to build on; THIR has already rejected
            // this derive, so emit no fields rather than risk a wrong recipe.
            return Vec::new();
        };

        methods
            .iter()
            .filter_map(|method| {
                let kind = derive_equality_kind(&method.name)?;
                let (arg_ty, result_ty) =
                    self.binary_bool_method_parts(method.sig, constraint_param, target)?;
                let value = self.synthesize_equality_method(
                    constraint,
                    kind,
                    &eq_method_name,
                    target,
                    arg_ty,
                    result_ty,
                );
                Some((method.name.clone(), value))
            })
            .collect()
    }

    fn synthesize_from_data_method(
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

    pub(super) fn derive_from_data_value(
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

    fn derive_atom_from_data(
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

    fn derive_optional_from_data(
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

    fn derive_list_from_data(
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

    fn derive_union_from_data(
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

    fn derive_record_from_data(
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

    fn prefix_validation_field(
        &mut self,
        result: TlcExprId,
        result_ty: crate::ir::TlcTypeId,
        field_name: &str,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let errors_ty = self.validation_errors_type(result_ty)?;
        let issue_ty = match self.type_arena[errors_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let path_ty = self.tlc_row_field_type(issue_ty, "path")?;
        let path_item_ty = match self.type_arena[path_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let field_payload_ty = self.tlc_row_field_type(path_item_ty, "field")?;
        let name_ty = self.tlc_row_field_type(field_payload_ty, "name")?;
        let name = self.alloc_expr(
            TlcExpr::Lit(Literal::Str(field_name.to_string())),
            name_ty,
            span,
        );
        let field_payload = self.alloc_expr(
            TlcExpr::Record(vec![("name".to_string(), name)]),
            field_payload_ty,
            span,
        );
        let segment = self.alloc_expr(
            TlcExpr::Variant("field".to_string(), field_payload),
            path_item_ty,
            span,
        );

        self.prefix_validation_segment(result, result_ty, segment, span)
    }

    fn decode_from_data_component_witness(
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

    fn prefix_validation_segment(
        &mut self,
        result: TlcExprId,
        result_ty: crate::ir::TlcTypeId,
        segment: TlcExprId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let errors_ty = self.validation_errors_type(result_ty)?;
        let issue_ty = match self.type_arena[errors_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let path_ty = self.tlc_row_field_type(issue_ty, "path")?;
        let error_ty = self.tlc_row_field_type(issue_ty, "error")?;
        let map_binding = self.fresh_synth_binding();
        let xs_binding = self.fresh_synth_binding();
        let tail_binding = self.fresh_synth_binding();
        let path_binding = self.fresh_synth_binding();
        let error_binding = self.fresh_synth_binding();
        let map_ty = self.alloc_type(TlcType::Fun(errors_ty, errors_ty, Row::REmpty));
        let map_var = self.alloc_expr(TlcExpr::Var(map_binding), map_ty, span);
        let xs = self.alloc_expr(TlcExpr::Var(xs_binding), errors_ty, span);
        let tail = self.alloc_expr(TlcExpr::Var(tail_binding), errors_ty, span);
        let old_path = self.alloc_expr(TlcExpr::Var(path_binding), path_ty, span);
        let old_error = self.alloc_expr(TlcExpr::Var(error_binding), error_ty, span);
        let prefix = self.alloc_expr(TlcExpr::List(vec![segment]), path_ty, span);
        let new_path = self.alloc_expr(TlcExpr::ListAppend(prefix, old_path), path_ty, span);
        let new_issue = self.alloc_expr(
            TlcExpr::Record(vec![
                ("path".to_string(), new_path),
                ("error".to_string(), old_error),
            ]),
            issue_ty,
            span,
        );
        let mapped_tail = self.alloc_expr(TlcExpr::App(map_var, tail), errors_ty, span);
        let one = self.alloc_expr(TlcExpr::List(vec![new_issue]), errors_ty, span);
        let cons = self.alloc_expr(TlcExpr::ListAppend(one, mapped_tail), errors_ty, span);
        let empty = self.alloc_expr(TlcExpr::List(Vec::new()), errors_ty, span);
        let map_body = self.alloc_expr(
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
                            Box::new(TlcPat::Record(vec![
                                ("path".to_string(), TlcPat::Bind(path_binding)),
                                ("error".to_string(), TlcPat::Bind(error_binding)),
                            ])),
                            Box::new(TlcPat::Bind(tail_binding)),
                        ),
                        guard: None,
                        body: cons,
                    },
                ],
            ),
            errors_ty,
            span,
        );
        let map_lam = self.alloc_expr(TlcExpr::Lam(xs_binding, errors_ty, map_body), map_ty, span);
        let errors_binding = self.fresh_synth_binding();
        let errors = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
        let mapped = self.alloc_expr(TlcExpr::App(map_var, errors), errors_ty, span);
        let invalid = self.from_data_invalid_with_errors(result_ty, mapped, span)?;
        let result_binding = self.fresh_synth_binding();
        let result_var = self.alloc_expr(TlcExpr::Var(result_binding), result_ty, span);
        let switched = self.alloc_expr(
            TlcExpr::Case(
                result_var,
                vec![
                    TlcAlt {
                        pat: TlcPat::Variant("valid".to_string(), Box::new(TlcPat::Wildcard)),
                        guard: None,
                        body: result_var,
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
        );
        let mapped_case = self.alloc_expr(
            TlcExpr::Letrec {
                bindings: vec![(map_binding, map_ty, map_lam)],
                body: switched,
            },
            result_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Let {
                binding: result_binding,
                ty: result_ty,
                value: result,
                body: mapped_case,
            },
            result_ty,
            span,
        ))
    }

    fn validation_type_with_value(
        &mut self,
        result_ty: crate::ir::TlcTypeId,
        value_ty: crate::ir::TlcTypeId,
    ) -> Option<crate::ir::TlcTypeId> {
        let valid_payload = self.tlc_row_field_type(result_ty, "valid")?;
        let invalid_payload = self.tlc_row_field_type(result_ty, "invalid")?;
        let valid_payload = match &self.type_arena[valid_payload] {
            TlcType::Record(row) => {
                let fields = row.fields().map(|(name, ty)| {
                    (
                        name.to_string(),
                        if name == "value" { value_ty } else { ty },
                    )
                });
                self.alloc_type(TlcType::Record(Row::from_fields(fields)))
            }
            _ => return None,
        };
        Some(self.alloc_type(TlcType::VariantT(Row::from_fields([
            ("valid".to_string(), valid_payload),
            ("invalid".to_string(), invalid_payload),
        ]))))
    }

    fn validation_errors_type(
        &self,
        result_ty: crate::ir::TlcTypeId,
    ) -> Option<crate::ir::TlcTypeId> {
        let invalid = self.tlc_row_field_type(result_ty, "invalid")?;
        self.tlc_row_field_type(invalid, "errors")
    }

    fn decode_path_item_type(
        &self,
        result_ty: crate::ir::TlcTypeId,
    ) -> Option<crate::ir::TlcTypeId> {
        let errors_ty = self.validation_errors_type(result_ty)?;
        let issue_ty = match self.type_arena[errors_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let path_ty = self.tlc_row_field_type(issue_ty, "path")?;
        match self.type_arena[path_ty] {
            TlcType::List(inner) => Some(inner),
            _ => None,
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn from_data_invalid_with_errors(
        &mut self,
        result_ty: crate::ir::TlcTypeId,
        errors: TlcExprId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(result_ty, "invalid")?;
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("errors".to_string(), errors)]),
            payload_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Variant("invalid".to_string(), payload),
            result_ty,
            span,
        ))
    }

    fn validation_present_optional(
        &mut self,
        decoded: TlcExprId,
        decoded_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        field_ty: crate::ir::TlcTypeId,
        optional_ty: crate::ir::TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let value_binding = self.fresh_synth_binding();
        let value = self.alloc_expr(TlcExpr::Var(value_binding), field_ty, span);
        let tuple_ty = self.alloc_type(TlcType::Tuple(vec![TlcTupleField::Positional(field_ty)]));
        let tuple = self.alloc_expr(
            TlcExpr::Tuple(vec![TlcTupleItem::Positional(value)]),
            tuple_ty,
            span,
        );
        let some = self.alloc_expr(
            TlcExpr::Variant("some".to_string(), tuple),
            optional_ty,
            span,
        );
        let valid = self.from_data_valid(result_ty, optional_ty, some, span)?;
        let errors_binding = self.fresh_synth_binding();
        let errors_ty = self.validation_errors_type(decoded_ty)?;
        let errors = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
        let invalid = self.from_data_invalid_with_errors(result_ty, errors, span)?;
        Some(self.alloc_expr(
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
        ))
    }

    fn record_valid_from_results(
        &mut self,
        target_ty: crate::ir::TlcTypeId,
        result_ty: crate::ir::TlcTypeId,
        decoded: &[DecodedRecordField],
        bindings: &[BindingId],
        fallback: TlcExprId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        #[allow(clippy::too_many_arguments)]
        fn build(
            lowerer: &mut Lowerer<'_>,
            index: usize,
            target_ty: crate::ir::TlcTypeId,
            result_ty: crate::ir::TlcTypeId,
            decoded: &[DecodedRecordField],
            bindings: &[BindingId],
            values: &mut Vec<(String, TlcExprId)>,
            fallback: TlcExprId,
            span: zutai_syntax::Span,
        ) -> Option<TlcExprId> {
            if index == decoded.len() {
                let record = lowerer.alloc_expr(TlcExpr::Record(values.clone()), target_ty, span);
                return lowerer.from_data_valid(result_ty, target_ty, record, span);
            }
            let field = &decoded[index];
            let result_var =
                lowerer.alloc_expr(TlcExpr::Var(bindings[index]), field.result_ty, span);
            let value_binding = lowerer.fresh_synth_binding();
            let valid_body = if field.optional {
                let optional_value =
                    lowerer.alloc_expr(TlcExpr::Var(value_binding), field.value_ty, span);
                let absent = build(
                    lowerer,
                    index + 1,
                    target_ty,
                    result_ty,
                    decoded,
                    bindings,
                    values,
                    fallback,
                    span,
                )?;
                let present_binding = lowerer.fresh_synth_binding();
                let present_value =
                    lowerer.alloc_expr(TlcExpr::Var(present_binding), field.field_ty, span);
                values.push((field.name.clone(), present_value));
                let present = build(
                    lowerer,
                    index + 1,
                    target_ty,
                    result_ty,
                    decoded,
                    bindings,
                    values,
                    fallback,
                    span,
                )?;
                values.pop();
                lowerer.alloc_expr(
                    TlcExpr::Case(
                        optional_value,
                        vec![
                            TlcAlt {
                                pat: TlcPat::Atom("none".to_string()),
                                guard: None,
                                body: absent,
                            },
                            TlcAlt {
                                pat: TlcPat::Variant(
                                    "some".to_string(),
                                    Box::new(TlcPat::Record(vec![(
                                        "0".to_string(),
                                        TlcPat::Bind(present_binding),
                                    )])),
                                ),
                                guard: None,
                                body: present,
                            },
                        ],
                    ),
                    result_ty,
                    span,
                )
            } else {
                let value = lowerer.alloc_expr(TlcExpr::Var(value_binding), field.field_ty, span);
                values.push((field.name.clone(), value));
                let next = build(
                    lowerer,
                    index + 1,
                    target_ty,
                    result_ty,
                    decoded,
                    bindings,
                    values,
                    fallback,
                    span,
                )?;
                values.pop();
                next
            };
            Some(lowerer.alloc_expr(
                TlcExpr::Case(
                    result_var,
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
                            body: valid_body,
                        },
                        TlcAlt {
                            pat: TlcPat::Wildcard,
                            guard: None,
                            body: fallback,
                        },
                    ],
                ),
                result_ty,
                span,
            ))
        }
        build(
            self,
            0,
            target_ty,
            result_ty,
            decoded,
            bindings,
            &mut Vec::new(),
            fallback,
            span,
        )
    }

    fn find_data_field(
        &mut self,
        fields: TlcExprId,
        fields_ty: crate::ir::TlcTypeId,
        field_ty: crate::ir::TlcTypeId,
        data_ty: crate::ir::TlcTypeId,
        wanted: &str,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let optional_ty = self.alloc_type(TlcType::Optional(data_ty));
        let find_binding = self.fresh_synth_binding();
        let xs_binding = self.fresh_synth_binding();
        let head_binding = self.fresh_synth_binding();
        let tail_binding = self.fresh_synth_binding();
        let xs = self.alloc_expr(TlcExpr::Var(xs_binding), fields_ty, span);
        let head = self.alloc_expr(TlcExpr::Var(head_binding), field_ty, span);
        let tail = self.alloc_expr(TlcExpr::Var(tail_binding), fields_ty, span);
        let name_ty = self.tlc_row_field_type(field_ty, "name")?;
        let value_ty = self.tlc_row_field_type(field_ty, "value")?;
        let name = self.alloc_expr(TlcExpr::GetField(head, "name".to_string()), name_ty, span);
        let wanted_expr = self.alloc_expr(
            TlcExpr::Lit(Literal::Str(wanted.to_string())),
            name_ty,
            span,
        );
        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
        let same = self.alloc_expr(
            TlcExpr::Builtin(BuiltinOp::Eq, name, wanted_expr),
            bool_ty,
            span,
        );
        let value = self.alloc_expr(TlcExpr::GetField(head, "value".to_string()), value_ty, span);
        let payload_ty = self.alloc_type(TlcType::Tuple(vec![TlcTupleField::Positional(data_ty)]));
        let payload = self.alloc_expr(
            TlcExpr::Tuple(vec![TlcTupleItem::Positional(value)]),
            payload_ty,
            span,
        );
        let some = self.alloc_expr(
            TlcExpr::Variant("some".to_string(), payload),
            optional_ty,
            span,
        );
        let find_ty = self.alloc_type(TlcType::Fun(fields_ty, optional_ty, Row::REmpty));
        let find_var = self.alloc_expr(TlcExpr::Var(find_binding), find_ty, span);
        let recur = self.alloc_expr(TlcExpr::App(find_var, tail), optional_ty, span);
        let choose = self.alloc_expr(
            TlcExpr::Case(
                same,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: some,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: recur,
                    },
                ],
            ),
            optional_ty,
            span,
        );
        let none = self.alloc_expr(
            TlcExpr::Lit(Literal::Atom("none".to_string())),
            optional_ty,
            span,
        );
        let find_body = self.alloc_expr(
            TlcExpr::Case(
                xs,
                vec![
                    TlcAlt {
                        pat: TlcPat::ListNil,
                        guard: None,
                        body: none,
                    },
                    TlcAlt {
                        pat: TlcPat::ListCons(
                            Box::new(TlcPat::Bind(head_binding)),
                            Box::new(TlcPat::Bind(tail_binding)),
                        ),
                        guard: None,
                        body: choose,
                    },
                ],
            ),
            optional_ty,
            span,
        );
        let find = self.alloc_expr(
            TlcExpr::Lam(xs_binding, fields_ty, find_body),
            find_ty,
            span,
        );
        let call = self.alloc_expr(TlcExpr::App(find_var, fields), optional_ty, span);
        Some(self.alloc_expr(
            TlcExpr::Letrec {
                bindings: vec![(find_binding, find_ty, find)],
                body: call,
            },
            optional_ty,
            span,
        ))
    }

    #[allow(clippy::wrong_self_convention)]
    fn from_data_valid(
        &mut self,
        result_ty: crate::ir::TlcTypeId,
        _target_ty: crate::ir::TlcTypeId,
        value: TlcExprId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let payload_ty = self.tlc_row_field_type(result_ty, "valid")?;
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("value".to_string(), value)]),
            payload_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Variant("valid".to_string(), payload),
            result_ty,
            span,
        ))
    }

    #[allow(clippy::wrong_self_convention)]
    fn from_data_invalid(
        &mut self,
        result_ty: crate::ir::TlcTypeId,
        expected: &str,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let invalid_payload_ty = self.tlc_row_field_type(result_ty, "invalid")?;
        let errors_ty = self.tlc_row_field_type(invalid_payload_ty, "errors")?;
        let issue_ty = match self.type_arena[errors_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        let path_ty = self.tlc_row_field_type(issue_ty, "path")?;
        let error_ty = self.tlc_row_field_type(issue_ty, "error")?;
        let custom_payload_ty = self.tlc_row_field_type(error_ty, "custom")?;
        let message_ty = self.tlc_row_field_type(custom_payload_ty, "message")?;
        let message = self.alloc_expr(
            TlcExpr::Lit(Literal::Str(format!("expected {expected}"))),
            message_ty,
            span,
        );
        let custom_payload = self.alloc_expr(
            TlcExpr::Record(vec![("message".to_string(), message)]),
            custom_payload_ty,
            span,
        );
        let error = self.alloc_expr(
            TlcExpr::Variant("custom".to_string(), custom_payload),
            error_ty,
            span,
        );
        let path = self.alloc_expr(TlcExpr::List(Vec::new()), path_ty, span);
        let issue = self.alloc_expr(
            TlcExpr::Record(vec![
                ("path".to_string(), path),
                ("error".to_string(), error),
            ]),
            issue_ty,
            span,
        );
        let errors = self.alloc_expr(TlcExpr::List(vec![issue]), errors_ty, span);
        let payload = self.alloc_expr(
            TlcExpr::Record(vec![("errors".to_string(), errors)]),
            invalid_payload_ty,
            span,
        );
        Some(self.alloc_expr(
            TlcExpr::Variant("invalid".to_string(), payload),
            result_ty,
            span,
        ))
    }

    fn tlc_row_field_type(
        &self,
        ty: crate::ir::TlcTypeId,
        label: &str,
    ) -> Option<crate::ir::TlcTypeId> {
        let row = match &self.type_arena[ty] {
            TlcType::Record(row) | TlcType::VariantT(row) => row,
            _ => return None,
        };
        row.fields()
            .find_map(|(name, field_ty)| (name == label).then_some(field_ty))
    }

    fn constraint_has_recipe(&self, constraint: BindingId) -> bool {
        self.thir.decls.iter().any(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            decl.binding == constraint
                && matches!(
                    &decl.kind,
                    ThirDeclKind::Constraint {
                        recipe: Some(_),
                        ..
                    }
                )
        })
    }
    fn constraint_info(
        &self,
        constraint: BindingId,
    ) -> Option<(BindingId, Vec<ThirConstraintMethod>)> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint {
                    params, methods, ..
                } = &decl.kind
            {
                return params
                    .first()
                    .copied()
                    .map(|param| (param, methods.clone()));
            }
            None
        })
    }

    fn binary_bool_method_parts(
        &self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<(TypeId, TypeId)> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        let TypeKind::Function {
            from: second,
            to: result,
        } = self.thir.type_arena[to.0 as usize].kind
        else {
            return None;
        };
        if !matches!(self.thir.type_arena[result.0 as usize].kind, TypeKind::Bool) {
            return None;
        }

        let first = self.substitute_constraint_arg(from, constraint_param, target);
        let second = self.substitute_constraint_arg(second, constraint_param, target);
        (first == target && second == target).then_some((target, result))
    }

    fn substitute_constraint_arg(
        &self,
        ty: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> TypeId {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(binding) if binding == constraint_param => target,
            _ => ty,
        }
    }

    fn unary_text_method_parts(
        &self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<(TypeId, TypeId)> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        if !matches!(self.thir.type_arena[to.0 as usize].kind, TypeKind::Text) {
            return None;
        }
        let arg = self.substitute_constraint_arg(from, constraint_param, target);
        (arg == target).then_some((target, to))
    }

    fn binary_ord_method_parts(
        &self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<(TypeId, TypeId)> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        let TypeKind::Function {
            from: second,
            to: result,
        } = self.thir.type_arena[to.0 as usize].kind
        else {
            return None;
        };
        let first = self.substitute_constraint_arg(from, constraint_param, target);
        let second = self.substitute_constraint_arg(second, constraint_param, target);
        (first == target && second == target).then_some((target, result))
    }

    fn synthesize_show_method(
        &mut self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<TlcExprId> {
        let (arg_ty, result_ty) = self.unary_text_method_parts(sig, constraint_param, target)?;
        let span = zutai_syntax::Span::default();
        let arg = self.fresh_synth_binding();
        let arg_tlc_ty = self.lower_type(arg_ty);
        let result_tlc_ty = self.lower_type(result_ty);
        let arg_expr = self.alloc_expr(TlcExpr::Var(arg), arg_tlc_ty, span);
        let body = self.derive_show_expr(target, arg_expr);
        let fn_ty = self.alloc_type(TlcType::Fun(arg_tlc_ty, result_tlc_ty, Row::REmpty));
        Some(self.alloc_expr(TlcExpr::Lam(arg, arg_tlc_ty, body), fn_ty, span))
    }

    fn synthesize_ord_method(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<TlcExprId> {
        let (arg_ty, result_ty) = self.binary_ord_method_parts(sig, constraint_param, target)?;
        let span = zutai_syntax::Span::default();
        let lhs = self.fresh_synth_binding();
        let rhs = self.fresh_synth_binding();
        let arg_tlc_ty = self.lower_type(arg_ty);
        let result_tlc_ty = self.lower_type(result_ty);
        let lhs_expr = self.alloc_expr(TlcExpr::Var(lhs), arg_tlc_ty, span);
        let rhs_expr = self.alloc_expr(TlcExpr::Var(rhs), arg_tlc_ty, span);
        let body = self.derive_ord_expr(
            constraint,
            method_name,
            target,
            result_ty,
            lhs_expr,
            rhs_expr,
        );
        let inner_ty = self.alloc_type(TlcType::Fun(arg_tlc_ty, result_tlc_ty, Row::REmpty));
        let inner = self.alloc_expr(TlcExpr::Lam(rhs, arg_tlc_ty, body), inner_ty, span);
        let outer_ty = self.alloc_type(TlcType::Fun(arg_tlc_ty, inner_ty, Row::REmpty));
        Some(self.alloc_expr(TlcExpr::Lam(lhs, arg_tlc_ty, inner), outer_ty, span))
    }

    fn derive_show_expr(&mut self, ty: TypeId, arg: TlcExprId) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let text_ty = self.alloc_type(TlcType::Prim(PrimTy::Str));
        match self.derive_shape(ty) {
            DeriveShape::Union(variants) => {
                let mut alts = Vec::with_capacity(variants.len() + 1);
                for variant in variants {
                    let body = self.alloc_expr(
                        TlcExpr::Lit(Literal::Str(format!("#{}", variant.name))),
                        text_ty,
                        span,
                    );
                    alts.push(TlcAlt {
                        pat: self.variant_wildcard_pat(&variant),
                        guard: None,
                        body,
                    });
                }
                let fallback = self.alloc_expr(
                    TlcExpr::Lit(Literal::Str("union".to_string())),
                    text_ty,
                    span,
                );
                alts.push(TlcAlt {
                    pat: TlcPat::Wildcard,
                    guard: None,
                    body: fallback,
                });
                self.alloc_expr(TlcExpr::Case(arg, alts), text_ty, span)
            }
            DeriveShape::Record(fields) => {
                let names = fields
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>()
                    .join(", ");
                self.alloc_expr(
                    TlcExpr::Lit(Literal::Str(format!("{{{names}}}"))),
                    text_ty,
                    span,
                )
            }
            DeriveShape::Tuple(items) => {
                let names = items
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, _))| name.unwrap_or_else(|| index.to_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.alloc_expr(
                    TlcExpr::Lit(Literal::Str(format!("({names})"))),
                    text_ty,
                    span,
                )
            }
            DeriveShape::Leaf => self.alloc_expr(
                TlcExpr::Lit(Literal::Str(self.type_label_for_derive(ty))),
                text_ty,
                span,
            ),
        }
    }

    fn derive_ord_expr(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        ty: TypeId,
        result_ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        match self.derive_shape(ty) {
            DeriveShape::Record(fields) => {
                self.derive_ord_record(constraint, method_name, result_ty, lhs, rhs, fields)
            }
            DeriveShape::Union(variants) => {
                self.derive_ord_union(constraint, method_name, result_ty, lhs, rhs, variants)
            }
            _ => self.derive_leaf_ord(ty, result_ty, lhs, rhs),
        }
    }

    fn derive_ord_record(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        result_ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
        fields: Vec<(String, TypeId)>,
    ) -> TlcExprId {
        let mut expr = self.ordering_atom("eq", result_ty);
        for (name, field_ty) in fields.into_iter().rev() {
            let left = self.derive_get_field(lhs, name.as_str(), field_ty);
            let right = self.derive_get_field(rhs, name.as_str(), field_ty);
            let cmp = self.derive_component_ord(
                constraint,
                method_name,
                field_ty,
                result_ty,
                left,
                right,
            );
            expr = self.if_ordering_eq(cmp, expr, cmp, result_ty);
        }
        expr
    }

    fn derive_ord_union(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        result_ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
        variants: Vec<DeriveVariant>,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let lhs_ty = self
            .expr_types
            .get(&lhs)
            .copied()
            .unwrap_or_else(|| self.lower_type(result_ty));
        let rhs_ty = self.expr_types.get(&rhs).copied().unwrap_or(lhs_ty);
        let scrut_ty = self.alloc_type(TlcType::Tuple(vec![
            TlcTupleField::Positional(lhs_ty),
            TlcTupleField::Positional(rhs_ty),
        ]));
        let scrutinee = self.alloc_expr(
            TlcExpr::Tuple(vec![
                TlcTupleItem::Positional(lhs),
                TlcTupleItem::Positional(rhs),
            ]),
            scrut_ty,
            span,
        );
        let mut alts = Vec::new();
        for (left_index, left_variant) in variants.iter().enumerate() {
            for (right_index, right_variant) in variants.iter().enumerate() {
                let (left_pat, right_pat, body) = if left_index == right_index {
                    let (left_pat, right_pat, body) = self.derive_ord_union_payload(
                        constraint,
                        method_name,
                        result_ty,
                        left_variant,
                    );
                    (left_pat, right_pat, body)
                } else if left_index < right_index {
                    (
                        self.variant_wildcard_pat(left_variant),
                        self.variant_wildcard_pat(right_variant),
                        self.ordering_atom("lt", result_ty),
                    )
                } else {
                    (
                        self.variant_wildcard_pat(left_variant),
                        self.variant_wildcard_pat(right_variant),
                        self.ordering_atom("gt", result_ty),
                    )
                };
                alts.push(TlcAlt {
                    pat: TlcPat::Tuple(vec![
                        TlcPatItem::Positional(left_pat),
                        TlcPatItem::Positional(right_pat),
                    ]),
                    guard: None,
                    body,
                });
            }
        }
        let fallback = self.ordering_atom("eq", result_ty);
        alts.push(TlcAlt {
            pat: TlcPat::Wildcard,
            guard: None,
            body: fallback,
        });
        let result_tlc_ty = self.lower_type(result_ty);
        self.alloc_expr(TlcExpr::Case(scrutinee, alts), result_tlc_ty, span)
    }

    fn derive_ord_union_payload(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        result_ty: TypeId,
        variant: &DeriveVariant,
    ) -> (TlcPat, TlcPat, TlcExprId) {
        let span = zutai_syntax::Span::default();
        let mut left_fields = Vec::with_capacity(variant.payload_fields.len());
        let mut right_fields = Vec::with_capacity(variant.payload_fields.len());
        let mut components = Vec::with_capacity(variant.payload_fields.len());
        for (field_name, field_ty) in &variant.payload_fields {
            let left_binding = self.fresh_synth_binding();
            let right_binding = self.fresh_synth_binding();
            let field_tlc_ty = self.lower_type(*field_ty);
            let left_expr = self.alloc_expr(TlcExpr::Var(left_binding), field_tlc_ty, span);
            let right_expr = self.alloc_expr(TlcExpr::Var(right_binding), field_tlc_ty, span);
            left_fields.push((field_name.clone(), TlcPat::Bind(left_binding)));
            right_fields.push((field_name.clone(), TlcPat::Bind(right_binding)));
            components.push((*field_ty, left_expr, right_expr));
        }
        let left_pat = if left_fields.is_empty() {
            TlcPat::Atom(variant.name.clone())
        } else {
            TlcPat::Variant(variant.name.clone(), Box::new(TlcPat::Record(left_fields)))
        };
        let right_pat = if right_fields.is_empty() {
            TlcPat::Atom(variant.name.clone())
        } else {
            TlcPat::Variant(variant.name.clone(), Box::new(TlcPat::Record(right_fields)))
        };
        let mut body = self.ordering_atom("eq", result_ty);
        for (field_ty, left_expr, right_expr) in components.into_iter().rev() {
            let cmp = self.derive_component_ord(
                constraint,
                method_name,
                field_ty,
                result_ty,
                left_expr,
                right_expr,
            );
            body = self.if_ordering_eq(cmp, body, cmp, result_ty);
        }
        (left_pat, right_pat, body)
    }

    fn variant_wildcard_pat(&self, variant: &DeriveVariant) -> TlcPat {
        if variant.payload_fields.is_empty() {
            TlcPat::Atom(variant.name.clone())
        } else {
            TlcPat::Variant(variant.name.clone(), Box::new(TlcPat::Wildcard))
        }
    }

    fn derive_component_ord(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        ty: TypeId,
        result_ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        if self.has_witness_binding(constraint, ty) {
            let span = zutai_syntax::Span::default();
            let component_ty = self.lower_type(ty);
            let result_tlc_ty = self.lower_type(result_ty);
            let after_first_ty =
                self.alloc_type(TlcType::Fun(component_ty, result_tlc_ty, Row::REmpty));
            let method_ty =
                self.alloc_type(TlcType::Fun(component_ty, after_first_ty, Row::REmpty));
            let dict = self.get_dict_expr(constraint, ty, span);
            let method = self.alloc_expr(
                TlcExpr::GetField(dict, method_name.to_string()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(method, constraint, method_name);
            let first = self.alloc_expr(TlcExpr::App(method, lhs), after_first_ty, span);
            return self.alloc_expr(TlcExpr::App(first, rhs), result_tlc_ty, span);
        }
        self.derive_leaf_ord(ty, result_ty, lhs, rhs)
    }

    fn derive_leaf_ord(
        &mut self,
        _ty: TypeId,
        result_ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
        let result_tlc_ty = self.lower_type(result_ty);
        let lt = self.alloc_expr(TlcExpr::Builtin(BuiltinOp::Lt, lhs, rhs), bool_ty, span);
        let gt = self.alloc_expr(TlcExpr::Builtin(BuiltinOp::Gt, lhs, rhs), bool_ty, span);
        let lt_atom = self.ordering_atom("lt", result_ty);
        let gt_atom = self.ordering_atom("gt", result_ty);
        let eq_atom = self.ordering_atom("eq", result_ty);
        let ge = self.alloc_expr(
            TlcExpr::Case(
                gt,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: gt_atom,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: eq_atom,
                    },
                ],
            ),
            result_tlc_ty,
            span,
        );
        self.alloc_expr(
            TlcExpr::Case(
                lt,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: lt_atom,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: ge,
                    },
                ],
            ),
            result_tlc_ty,
            span,
        )
    }

    fn if_ordering_eq(
        &mut self,
        cmp: TlcExprId,
        then_expr: TlcExprId,
        else_expr: TlcExprId,
        result_ty: TypeId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
        let result_tlc_ty = self.lower_type(result_ty);
        let eq_atom = self.ordering_atom("eq", result_ty);
        let is_eq = self.alloc_expr(TlcExpr::Builtin(BuiltinOp::Eq, cmp, eq_atom), bool_ty, span);
        self.alloc_expr(
            TlcExpr::Case(
                is_eq,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: then_expr,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: else_expr,
                    },
                ],
            ),
            result_tlc_ty,
            span,
        )
    }

    fn ordering_atom(&mut self, name: &str, result_ty: TypeId) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let ty = self.lower_type(result_ty);
        self.alloc_expr(TlcExpr::Lit(Literal::Atom(name.to_string())), ty, span)
    }

    fn type_label_for_derive(&self, ty: TypeId) -> String {
        match self.resolve_alias_shape(ty) {
            TypeKind::Bool | TypeKind::True | TypeKind::False => "Bool".to_string(),
            TypeKind::Text => "Text".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Posit(spec) => spec.type_name(),
            TypeKind::Atom(name) => format!("#{name}"),
            TypeKind::Opaque(name) => name,
            _ => "value".to_string(),
        }
    }

    fn synthesize_equality_method(
        &mut self,
        constraint: BindingId,
        kind: EqualityKind,
        eq_field: &str,
        target: TypeId,
        arg_ty: TypeId,
        result_ty: TypeId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let lhs = self.fresh_synth_binding();
        let rhs = self.fresh_synth_binding();
        let arg_tlc_ty = self.lower_type(arg_ty);
        let result_tlc_ty = self.lower_type(result_ty);
        let lhs_expr = self.alloc_expr(TlcExpr::Var(lhs), arg_tlc_ty, span);
        let rhs_expr = self.alloc_expr(TlcExpr::Var(rhs), arg_tlc_ty, span);
        // Always build structural equality, then negate for `neq`/`!=`.
        let eq_body = self.derive_compare_expr(
            constraint,
            eq_field,
            BuiltinOp::Eq,
            target,
            lhs_expr,
            rhs_expr,
        );
        let body = match kind {
            EqualityKind::Eq => eq_body,
            EqualityKind::Ne => {
                let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
                let truth = self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), bool_ty, span);
                self.alloc_expr(
                    TlcExpr::Builtin(BuiltinOp::Ne, eq_body, truth),
                    bool_ty,
                    span,
                )
            }
        };
        let inner_ty = self.alloc_type(TlcType::Fun(arg_tlc_ty, result_tlc_ty, Row::REmpty));
        let inner = self.alloc_expr(TlcExpr::Lam(rhs, arg_tlc_ty, body), inner_ty, span);
        let outer_ty = self.alloc_type(TlcType::Fun(arg_tlc_ty, inner_ty, Row::REmpty));
        self.alloc_expr(TlcExpr::Lam(lhs, arg_tlc_ty, inner), outer_ty, span)
    }

    fn derive_compare_expr(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        op: BuiltinOp,
        ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        match self.derive_shape(ty) {
            DeriveShape::Leaf => {
                self.derive_leaf_compare(constraint, method_name, op, ty, lhs, rhs)
            }
            DeriveShape::Record(fields) => {
                let mut parts = Vec::with_capacity(fields.len());
                for (name, field_ty) in fields {
                    let left = self.derive_get_field(lhs, name.as_str(), field_ty);
                    let right = self.derive_get_field(rhs, name.as_str(), field_ty);
                    parts.push(self.derive_component_compare(
                        constraint,
                        method_name,
                        op,
                        field_ty,
                        left,
                        right,
                    ));
                }
                self.fold_bool_parts(parts)
            }
            DeriveShape::Tuple(items) => {
                self.derive_tuple_compare(constraint, method_name, op, items, lhs, rhs)
            }
            DeriveShape::Union(variants) => {
                self.derive_union_compare(constraint, method_name, op, variants, lhs, rhs)
            }
        }
    }

    fn derive_component_compare(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        op: BuiltinOp,
        ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        if self.has_witness_binding(constraint, ty) {
            return self.derive_witness_call(constraint, method_name, ty, lhs, rhs);
        }
        self.derive_leaf_compare(constraint, method_name, op, ty, lhs, rhs)
    }

    fn derive_leaf_compare(
        &mut self,
        _constraint: BindingId,
        _method_name: &str,
        op: BuiltinOp,
        ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        if self.derive_builtin_leaf(ty) {
            self.alloc_expr(TlcExpr::Builtin(op, lhs, rhs), bool_ty, span)
        } else {
            self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), bool_ty, span)
        }
    }

    fn derive_witness_call(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let component_ty = self.lower_type(ty);
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        let after_first_ty = self.alloc_type(TlcType::Fun(component_ty, bool_ty, Row::REmpty));
        let method_ty = self.alloc_type(TlcType::Fun(component_ty, after_first_ty, Row::REmpty));
        let dict = self.get_dict_expr(constraint, ty, span);
        let method = self.alloc_expr(
            TlcExpr::GetField(dict, method_name.to_string()),
            method_ty,
            span,
        );
        self.register_dict_field_slot(method, constraint, method_name);
        let first = self.alloc_expr(TlcExpr::App(method, lhs), after_first_ty, span);
        self.alloc_expr(TlcExpr::App(first, rhs), bool_ty, span)
    }

    fn derive_tuple_compare(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        op: BuiltinOp,
        items: Vec<(Option<String>, TypeId)>,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        let lhs_ty = self.expr_types.get(&lhs).copied().unwrap_or(bool_ty);
        let rhs_ty = self.expr_types.get(&rhs).copied().unwrap_or(lhs_ty);
        let scrut_ty = self.alloc_type(TlcType::Tuple(vec![
            TlcTupleField::Positional(lhs_ty),
            TlcTupleField::Positional(rhs_ty),
        ]));
        let scrutinee = self.alloc_expr(
            TlcExpr::Tuple(vec![
                TlcTupleItem::Positional(lhs),
                TlcTupleItem::Positional(rhs),
            ]),
            scrut_ty,
            span,
        );

        let mut left_items = Vec::with_capacity(items.len());
        let mut right_items = Vec::with_capacity(items.len());
        let mut parts = Vec::with_capacity(items.len());
        for (name, item_ty) in items {
            let left_binding = self.fresh_synth_binding();
            let right_binding = self.fresh_synth_binding();
            let item_tlc_ty = self.lower_type(item_ty);
            let left_expr = self.alloc_expr(TlcExpr::Var(left_binding), item_tlc_ty, span);
            let right_expr = self.alloc_expr(TlcExpr::Var(right_binding), item_tlc_ty, span);
            let left_pat = TlcPat::Bind(left_binding);
            let right_pat = TlcPat::Bind(right_binding);
            match name {
                Some(name) => {
                    left_items.push(TlcPatItem::Named {
                        name: name.clone(),
                        pat: left_pat,
                    });
                    right_items.push(TlcPatItem::Named {
                        name,
                        pat: right_pat,
                    });
                }
                None => {
                    left_items.push(TlcPatItem::Positional(left_pat));
                    right_items.push(TlcPatItem::Positional(right_pat));
                }
            }
            parts.push(self.derive_component_compare(
                constraint,
                method_name,
                op,
                item_ty,
                left_expr,
                right_expr,
            ));
        }

        let body = self.fold_bool_parts(parts);
        let fallback = self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), bool_ty, span);
        self.alloc_expr(
            TlcExpr::Case(
                scrutinee,
                vec![
                    TlcAlt {
                        pat: TlcPat::Tuple(vec![
                            TlcPatItem::Positional(TlcPat::Tuple(left_items)),
                            TlcPatItem::Positional(TlcPat::Tuple(right_items)),
                        ]),
                        guard: None,
                        body,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: fallback,
                    },
                ],
            ),
            bool_ty,
            span,
        )
    }

    fn derive_union_compare(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        op: BuiltinOp,
        variants: Vec<DeriveVariant>,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        let lhs_ty = self.expr_types.get(&lhs).copied().unwrap_or(bool_ty);
        let rhs_ty = self.expr_types.get(&rhs).copied().unwrap_or(lhs_ty);
        let scrut_ty = self.alloc_type(TlcType::Tuple(vec![
            TlcTupleField::Positional(lhs_ty),
            TlcTupleField::Positional(rhs_ty),
        ]));
        let scrutinee = self.alloc_expr(
            TlcExpr::Tuple(vec![
                TlcTupleItem::Positional(lhs),
                TlcTupleItem::Positional(rhs),
            ]),
            scrut_ty,
            span,
        );

        let mut alts = Vec::with_capacity(variants.len() + 1);
        for variant in variants {
            let mut left_fields = Vec::new();
            let mut right_fields = Vec::new();
            let mut parts = Vec::with_capacity(variant.payload_fields.len());
            for (field_name, field_ty) in variant.payload_fields {
                let left_binding = self.fresh_synth_binding();
                let right_binding = self.fresh_synth_binding();
                let field_tlc_ty = self.lower_type(field_ty);
                let left_expr = self.alloc_expr(TlcExpr::Var(left_binding), field_tlc_ty, span);
                let right_expr = self.alloc_expr(TlcExpr::Var(right_binding), field_tlc_ty, span);
                left_fields.push((field_name.clone(), TlcPat::Bind(left_binding)));
                right_fields.push((field_name, TlcPat::Bind(right_binding)));
                parts.push(self.derive_component_compare(
                    constraint,
                    method_name,
                    op,
                    field_ty,
                    left_expr,
                    right_expr,
                ));
            }
            let left_pat = if left_fields.is_empty() {
                TlcPat::Atom(variant.name.clone())
            } else {
                TlcPat::Variant(variant.name.clone(), Box::new(TlcPat::Record(left_fields)))
            };
            let right_pat = if right_fields.is_empty() {
                TlcPat::Atom(variant.name)
            } else {
                TlcPat::Variant(variant.name, Box::new(TlcPat::Record(right_fields)))
            };
            let body = self.fold_bool_parts(parts);
            alts.push(TlcAlt {
                pat: TlcPat::Tuple(vec![
                    TlcPatItem::Positional(left_pat),
                    TlcPatItem::Positional(right_pat),
                ]),
                guard: None,
                body,
            });
        }
        let fallback = self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), bool_ty, span);
        alts.push(TlcAlt {
            pat: TlcPat::Wildcard,
            guard: None,
            body: fallback,
        });
        self.alloc_expr(TlcExpr::Case(scrutinee, alts), bool_ty, span)
    }

    fn derive_get_field(&mut self, base: TlcExprId, field: &str, field_ty: TypeId) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let ty = self.lower_type(field_ty);
        self.alloc_expr(TlcExpr::GetField(base, field.to_string()), ty, span)
    }

    fn fold_bool_parts(&mut self, parts: impl IntoIterator<Item = TlcExprId>) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        let mut iter = parts.into_iter();
        let Some(first) = iter.next() else {
            return self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), bool_ty, span);
        };
        iter.fold(first, |lhs, rhs| {
            self.alloc_expr(TlcExpr::Builtin(BuiltinOp::And, lhs, rhs), bool_ty, span)
        })
    }

    fn derive_shape(&self, ty: TypeId) -> DeriveShape {
        match self.resolve_alias_shape(ty) {
            TypeKind::Record(fields, RowTail::Closed) => DeriveShape::Record(
                fields
                    .into_iter()
                    .map(|field| (field.name, field.ty))
                    .collect(),
            ),
            TypeKind::Tuple(items) => DeriveShape::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => (Some(name), ty),
                        TypeTupleItem::Positional(ty) => (None, ty),
                    })
                    .collect(),
            ),
            TypeKind::Union(variants, RowTail::Closed) => DeriveShape::Union(
                variants
                    .into_iter()
                    .map(|variant| {
                        let payload_fields = variant
                            .payload
                            .and_then(|payload| match self.resolve_alias_shape(payload) {
                                TypeKind::Record(fields, RowTail::Closed) => Some(
                                    fields
                                        .into_iter()
                                        .map(|field| (field.name, field.ty))
                                        .collect(),
                                ),
                                _ => None,
                            })
                            .unwrap_or_default();
                        DeriveVariant {
                            name: variant.name,
                            payload_fields,
                        }
                    })
                    .collect(),
            ),
            _ => DeriveShape::Leaf,
        }
    }

    fn resolve_alias_shape(&self, ty: TypeId) -> TypeKind {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Alias(binding) => self
                .type_alias_body(binding)
                .map(|body| self.resolve_alias_shape(body))
                .unwrap_or(TypeKind::Alias(binding)),
            TypeKind::AliasApply { binding, args } => {
                let Some((params, body)) = self.type_alias_params_body(binding) else {
                    return TypeKind::AliasApply { binding, args };
                };
                let subst: rustc_hash::FxHashMap<BindingId, TypeId> =
                    params.into_iter().zip(args).collect();
                self.substitute_alias_shape(body, &subst)
            }
            kind => kind,
        }
    }

    fn substitute_alias_shape(
        &self,
        ty: TypeId,
        subst: &rustc_hash::FxHashMap<BindingId, TypeId>,
    ) -> TypeKind {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(binding) => subst
                .get(&binding)
                .map(|&replacement| self.resolve_alias_shape(replacement))
                .unwrap_or(TypeKind::TypeVar(binding)),
            TypeKind::Record(fields, tail) => TypeKind::Record(
                fields
                    .into_iter()
                    .map(|mut field| {
                        field.ty = self.substitute_component_type(field.ty, subst);
                        field
                    })
                    .collect(),
                tail,
            ),
            TypeKind::Tuple(items) => TypeKind::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, span } => TypeTupleItem::Named {
                            name,
                            ty: self.substitute_component_type(ty, subst),
                            span,
                        },
                        TypeTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.substitute_component_type(ty, subst))
                        }
                    })
                    .collect(),
            ),
            TypeKind::Union(variants, tail) => TypeKind::Union(
                variants
                    .into_iter()
                    .map(|mut variant| {
                        variant.payload = variant
                            .payload
                            .map(|payload| self.substitute_component_type(payload, subst));
                        variant
                    })
                    .collect(),
                tail,
            ),
            kind => kind,
        }
    }

    fn substitute_component_type(
        &self,
        ty: TypeId,
        subst: &rustc_hash::FxHashMap<BindingId, TypeId>,
    ) -> TypeId {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(binding) => subst.get(&binding).copied().unwrap_or(ty),
            _ => ty,
        }
    }

    fn has_witness_binding(&self, constraint: BindingId, ty: TypeId) -> bool {
        self.thir_type_to_witness_key(ty)
            .is_some_and(|key| self.witness_bindings.contains_key(&(constraint.0, key)))
    }

    fn derive_builtin_leaf(&self, ty: TypeId) -> bool {
        matches!(
            self.resolve_alias_shape(ty),
            TypeKind::Bool
                | TypeKind::True
                | TypeKind::False
                | TypeKind::Text
                | TypeKind::Int
                | TypeKind::Float
                | TypeKind::Posit(_)
                | TypeKind::Atom(_)
        )
    }
}

#[derive(Clone, Copy)]
enum EqualityKind {
    Eq,
    Ne,
}

fn derive_equality_kind(method_name: &str) -> Option<EqualityKind> {
    match method_name {
        "eq" | "==" => Some(EqualityKind::Eq),
        "neq" | "!=" => Some(EqualityKind::Ne),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeriveRecipeKind {
    Show,
    Ord,
}

fn derive_recipe_kind(method_name: &str) -> Option<DeriveRecipeKind> {
    match method_name {
        "show" => Some(DeriveRecipeKind::Show),
        "compare" => Some(DeriveRecipeKind::Ord),
        _ => None,
    }
}
