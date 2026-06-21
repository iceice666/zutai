use zutai_hir::BindingId;
use zutai_thir::{RowTail, ThirConstraintMethod, ThirDeclKind, TypeId, TypeKind, TypeTupleItem};

use crate::ir::{
    BuiltinOp, Literal, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcTupleField,
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

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_derive_fields(
        &mut self,
        constraint: BindingId,
        target: TypeId,
    ) -> Vec<(String, TlcExprId)> {
        let Some((constraint_param, methods)) = self.constraint_info(constraint) else {
            return Vec::new();
        };

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
                let subst: std::collections::HashMap<BindingId, TypeId> =
                    params.into_iter().zip(args).collect();
                self.substitute_alias_shape(body, &subst)
            }
            kind => kind,
        }
    }

    fn substitute_alias_shape(
        &self,
        ty: TypeId,
        subst: &std::collections::HashMap<BindingId, TypeId>,
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
        subst: &std::collections::HashMap<BindingId, TypeId>,
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
