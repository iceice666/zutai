use zutai_hir::BindingId;

use crate::ir::{
    BuiltinOp, Literal, PrimTy, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcTupleField,
    TlcTupleItem, TlcType,
};

use super::*;
use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn prefix_validation_field(
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

    pub(super) fn prefix_validation_segment(
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
        let path_item_ty = match self.type_arena[path_ty] {
            TlcType::List(inner) => inner,
            _ => return None,
        };
        // `map` prefixes `segment` onto every issue path. To lift it to a recursive
        // global (the only recursion the backend realizes natively), the lambda must be
        // closed: `segment` is threaded through an explicit parameter rather than
        // captured, so the value references only its own params and the recursive global.
        let map_binding = self.fresh_synth_binding();
        let seg_binding = self.fresh_synth_binding();
        let xs_binding = self.fresh_synth_binding();
        let tail_binding = self.fresh_synth_binding();
        let path_binding = self.fresh_synth_binding();
        let error_binding = self.fresh_synth_binding();
        let map_inner_ty = self.alloc_type(TlcType::Fun(errors_ty, errors_ty, Row::REmpty));
        let map_ty = self.alloc_type(TlcType::Fun(path_item_ty, map_inner_ty, Row::REmpty));
        let map_var = self.alloc_expr(TlcExpr::Var(map_binding), map_ty, span);
        let seg_var = self.alloc_expr(TlcExpr::Var(seg_binding), path_item_ty, span);
        let xs = self.alloc_expr(TlcExpr::Var(xs_binding), errors_ty, span);
        let tail = self.alloc_expr(TlcExpr::Var(tail_binding), errors_ty, span);
        let old_path = self.alloc_expr(TlcExpr::Var(path_binding), path_ty, span);
        let old_error = self.alloc_expr(TlcExpr::Var(error_binding), error_ty, span);
        let prefix = self.alloc_expr(TlcExpr::List(vec![seg_var]), path_ty, span);
        let new_path = self.alloc_expr(TlcExpr::ListAppend(prefix, old_path), path_ty, span);
        let new_issue = self.alloc_expr(
            TlcExpr::Record(vec![
                ("path".to_string(), new_path),
                ("error".to_string(), old_error),
            ]),
            issue_ty,
            span,
        );
        let map_seg = self.alloc_expr(TlcExpr::App(map_var, seg_var), map_inner_ty, span);
        let mapped_tail = self.alloc_expr(TlcExpr::App(map_seg, tail), errors_ty, span);
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
        let map_inner_lam = self.alloc_expr(
            TlcExpr::Lam(xs_binding, errors_ty, map_body),
            map_inner_ty,
            span,
        );
        let map_lam = self.alloc_expr(
            TlcExpr::Lam(seg_binding, path_item_ty, map_inner_lam),
            map_ty,
            span,
        );
        let errors_binding = self.fresh_synth_binding();
        let errors = self.alloc_expr(TlcExpr::Var(errors_binding), errors_ty, span);
        let map_seg_outer = self.alloc_expr(TlcExpr::App(map_var, segment), map_inner_ty, span);
        let mapped = self.alloc_expr(TlcExpr::App(map_seg_outer, errors), errors_ty, span);
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

    pub(super) fn validation_type_with_value(
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

    pub(super) fn validation_errors_type(
        &self,
        result_ty: crate::ir::TlcTypeId,
    ) -> Option<crate::ir::TlcTypeId> {
        let invalid = self.tlc_row_field_type(result_ty, "invalid")?;
        self.tlc_row_field_type(invalid, "errors")
    }

    pub(super) fn decode_path_item_type(
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
    pub(super) fn from_data_invalid_with_errors(
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

    pub(super) fn validation_present_optional(
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

    pub(super) fn record_valid_from_results(
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

    pub(super) fn find_data_field(
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
    pub(super) fn from_data_valid(
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
    pub(super) fn from_data_invalid(
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
}
