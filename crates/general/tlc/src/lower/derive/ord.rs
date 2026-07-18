use zutai_hir::BindingId;
use zutai_thir::TypeId;

use crate::ir::{
    BuiltinOp, Literal, PrimTy, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcTupleField,
    TlcTupleItem, TlcType,
};

use super::*;
use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_ord_method(
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

    pub(super) fn synthesize_equality_method(
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

    pub(super) fn derive_ord_expr(
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

    pub(super) fn derive_ord_record(
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

    pub(super) fn derive_ord_union(
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

    pub(super) fn derive_ord_union_payload(
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

    pub(super) fn derive_component_ord(
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

    pub(super) fn derive_leaf_ord(
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

    pub(super) fn derive_compare_expr(
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

    pub(super) fn derive_component_compare(
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

    pub(super) fn derive_leaf_compare(
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

    pub(super) fn derive_tuple_compare(
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

    pub(super) fn derive_union_compare(
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
}
