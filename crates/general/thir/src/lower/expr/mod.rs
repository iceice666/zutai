use std::collections::{HashMap, HashSet};

use zutai_hir::{
    BindingId, BindingKind, HirDeclKind, HirExprId, HirExprKind, HirHandleClause, HirHandleOp,
    HirLocalBinding, HirRecordField, HirSelectField, HirTupleItem,
};
use zutai_syntax::Span;
use zutai_syntax::ast;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    RowTail, ThirExpr, ThirExprId, ThirExprKind, ThirHandleClause, ThirLocalBinding,
    ThirRecordField, Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
};

use super::{Lowerer, RowSolution, WrapperKind};

mod aggregate;
mod call;
mod control;
mod effects;
mod operators;
mod tagged;

impl<'hir> Lowerer<'hir> {
    pub(super) fn check_expr(&mut self, id: HirExprId, expected: TypeId) -> ThirExprId {
        let expr = self.hir_expr(id);
        match &expr.kind {
            HirExprKind::Record(fields) => self.check_record_expr(id, fields, expected),
            HirExprKind::List(items) => self.check_list_expr(id, items, expected),
            HirExprKind::Tuple(items) => self.check_tuple_expr(id, items, expected),
            HirExprKind::Block { bindings, result } => {
                self.lower_block_expr(id, bindings, *result, Some(expected))
            }
            HirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.lower_if_expr(id, *cond, *then_branch, *else_branch, Some(expected)),
            HirExprKind::Lambda { params, body } => {
                self.check_lambda_expr(id, params, *body, expected)
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.lower_match_expr(id, *scrutinee, arms, Some(expected))
            }
            HirExprKind::TaggedValue { tag, payload } => {
                self.lower_tagged_value_expr(id, tag, *payload, Some(expected), expr.span)
            }
            HirExprKind::Atom(name) => {
                let atom_ty = self.alloc_type(Type {
                    kind: TypeKind::Atom(name.clone()),
                    span: expr.span,
                });
                let contextual_atom = {
                    let resolved = self.resolve_alias(expected, &mut HashSet::new(), expr.span);
                    matches!(
                        self.type_arena[resolved.0 as usize].kind,
                        TypeKind::Union(_, _) | TypeKind::Optional(_) | TypeKind::Maybe(_)
                    )
                };
                let matches = self.type_matches(expected, atom_ty);
                if contextual_atom && matches {
                    self.alloc_expr(ThirExpr {
                        source: id,
                        ty: expected,
                        kind: ThirExprKind::Atom(name.clone()),
                        span: expr.span,
                    })
                } else {
                    if !matches {
                        self.type_mismatch(expected, atom_ty, expr.span);
                    }
                    self.alloc_expr(ThirExpr {
                        source: id,
                        ty: atom_ty,
                        kind: ThirExprKind::Atom(name.clone()),
                        span: expr.span,
                    })
                }
            }
            HirExprKind::Sequence(items) => self.lower_sequence_expr(id, items, Some(expected)),
            _ => {
                let lowered = self.infer_expr(id);
                let found = self.expr(lowered).ty;
                if !self.type_matches(expected, found) {
                    self.type_mismatch(expected, found, expr.span);
                }
                lowered
            }
        }
    }

    pub(super) fn infer_expr(&mut self, id: HirExprId) -> ThirExprId {
        let expr = self.hir_expr(id);
        match &expr.kind {
            HirExprKind::True => {
                let ty = self.bool_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::True,
                    span: expr.span,
                })
            }
            HirExprKind::False => {
                let ty = self.bool_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::False,
                    span: expr.span,
                })
            }
            HirExprKind::Integer(value, postfix) => {
                let ty = self.integer_literal_type(*value, *postfix, expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Integer(*value),
                    span: expr.span,
                })
            }
            HirExprKind::Float(value, postfix) => {
                let ty = self.float_literal_type(*postfix, expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Float(*value),
                    span: expr.span,
                })
            }
            HirExprKind::Posit(literal) => {
                let ty = self.posit_type(literal.spec, expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Posit(*literal),
                    span: expr.span,
                })
            }
            HirExprKind::String(value) => {
                let ty = self.text_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::String(value.clone()),
                    span: expr.span,
                })
            }
            HirExprKind::Atom(name) => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::Atom(name.clone()),
                    span: expr.span,
                });
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Atom(name.clone()),
                    span: expr.span,
                })
            }
            HirExprKind::BindingRef(binding) => self.lower_binding_ref(id, *binding, expr.span),
            HirExprKind::Record(fields) => self.infer_record_expr(id, fields, expr.span),
            HirExprKind::RecordUpdate { receiver, fields } => {
                self.lower_record_update_expr(id, *receiver, fields, expr.span)
            }
            HirExprKind::Tuple(items) => self.infer_tuple_expr(id, items, expr.span),
            HirExprKind::List(items) => self.infer_list_expr(id, items, expr.span),
            HirExprKind::TypeForm(ty) => {
                let value = self.lower_type(*ty);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty: self.type_type,
                    kind: ThirExprKind::TypeValue(value),
                    span: expr.span,
                })
            }
            HirExprKind::WitnessReflect { constraint, target } => {
                self.infer_witness_reflect_expr(id, *constraint, *target, expr.span)
            }
            HirExprKind::Access { receiver, field } => {
                self.lower_access_expr(id, *receiver, field, expr.span)
            }
            HirExprKind::Block { bindings, result } => {
                self.lower_block_expr(id, bindings, *result, None)
            }
            HirExprKind::Select { receiver, fields } => {
                self.lower_select_expr(id, *receiver, fields, expr.span)
            }
            HirExprKind::Perform { op, arg } => self.infer_perform_expr(id, op, *arg, expr.span),
            HirExprKind::Handle {
                expr: body,
                clauses,
            } => self.infer_handle_expr(id, *body, clauses, expr.span),
            HirExprKind::Resume { value } => self.infer_resume_expr(id, *value, expr.span),
            HirExprKind::Sequence(items) => self.lower_sequence_expr(id, items, None),
            HirExprKind::Apply { func, arg } => self.lower_apply_expr(id, *func, *arg, expr.span),
            HirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.lower_if_expr(id, *cond, *then_branch, *else_branch, None),
            HirExprKind::Binary { op, lhs, rhs } => {
                self.lower_binary_expr(id, *op, *lhs, *rhs, expr.span)
            }
            HirExprKind::UnresolvedIdent(name) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ValueTypeUnavailable { name: name.clone() },
                    span: expr.span,
                });
                self.error_expr(id, expr.span)
            }
            HirExprKind::Lambda { params, body } => {
                self.infer_lambda_expr(id, params, *body, expr.span)
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.lower_match_expr(id, *scrutinee, arms, None)
            }
            HirExprKind::Import(source) => self.lower_import_expr(id, source, expr.span),
            HirExprKind::OptAccess { receiver, field } => {
                self.lower_opt_access_expr(id, *receiver, field, expr.span)
            }
            HirExprKind::TaggedValue { tag, payload } => {
                self.lower_tagged_value_expr(id, tag, *payload, None, expr.span)
            }
        }
    }

    fn infer_witness_reflect_expr(
        &mut self,
        id: HirExprId,
        constraint: Option<BindingId>,
        target: zutai_hir::HirTypeId,
        span: Span,
    ) -> ThirExprId {
        let target = self.lower_type(target);
        let Some(constraint) = constraint else {
            return self.error_expr(id, span);
        };
        let method_sigs = self.witness_method_sigs(constraint, target);
        if method_sigs.is_empty() {
            return self.error_expr(id, span);
        }

        let target_key = self.witness_target_key(target);
        if !self.source_has_witness_for(constraint, target, &target_key) {
            let constraint_name = self.hir.bindings[constraint.0 as usize].name.clone();
            let target_name = self.type_name(target);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::WitnessReflectNotInScope {
                    constraint: constraint_name,
                    target: target_name,
                },
                span,
            });
        }

        let mut fields: Vec<TypeRecordField> = method_sigs
            .into_iter()
            .map(|(name, ty)| TypeRecordField {
                name,
                optional: false,
                ty,
                span,
            })
            .collect();
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        let ty = self.alloc_type(Type {
            kind: TypeKind::Record(fields, RowTail::Closed),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::WitnessReflect {
                constraint: Some(constraint),
                target,
            },
            span,
        })
    }

    fn source_has_witness_for(
        &mut self,
        constraint: BindingId,
        target: TypeId,
        target_key: &str,
    ) -> bool {
        let decls = self.hir.decls.clone();
        decls.into_iter().any(|decl_id| {
            let (witness_constraint, witness_target, params) = {
                let decl = &self.hir.decl_arena[decl_id];
                let HirDeclKind::Witness {
                    constraint: Some(witness_constraint),
                    target,
                    params,
                    ..
                } = &decl.kind
                else {
                    return false;
                };
                (*witness_constraint, *target, params.clone())
            };
            if witness_constraint != constraint {
                return false;
            }
            let witness_target = self.lower_type(witness_target);
            if self.witness_target_key(witness_target) == target_key {
                return true;
            }
            let params: HashSet<_> = params.into_iter().map(|param| param.binding).collect();
            !params.is_empty() && self.witness_pattern_matches(witness_target, target, &params)
        })
    }

    fn witness_pattern_matches(
        &mut self,
        pattern: TypeId,
        actual: TypeId,
        params: &HashSet<BindingId>,
    ) -> bool {
        let pattern = self.resolve_alias(
            pattern,
            &mut HashSet::new(),
            self.type_arena[pattern.0 as usize].span,
        );
        let actual = self.resolve_alias(
            actual,
            &mut HashSet::new(),
            self.type_arena[actual.0 as usize].span,
        );
        if self.witness_target_key(pattern) == self.witness_target_key(actual) {
            return true;
        }
        let pattern_kind = self.type_arena[pattern.0 as usize].kind.clone();
        let actual_kind = self.type_arena[actual.0 as usize].kind.clone();
        match (pattern_kind, actual_kind) {
            (TypeKind::TypeVar(param), _) if params.contains(&param) => true,
            (TypeKind::List(pattern), TypeKind::List(actual))
            | (TypeKind::Optional(pattern), TypeKind::Optional(actual))
            | (TypeKind::Maybe(pattern), TypeKind::Maybe(actual)) => {
                self.witness_pattern_matches(pattern, actual, params)
            }
            (
                TypeKind::Patch {
                    target: p,
                    deep: pd,
                },
                TypeKind::Patch {
                    target: a,
                    deep: ad,
                },
            ) => pd == ad && self.witness_pattern_matches(p, a, params),
            (
                TypeKind::Function {
                    from: pf, to: pt, ..
                },
                TypeKind::Function {
                    from: af, to: at, ..
                },
            ) => {
                self.witness_pattern_matches(pf, af, params)
                    && self.witness_pattern_matches(pt, at, params)
            }
            (TypeKind::Tuple(pattern_items), TypeKind::Tuple(actual_items)) => {
                pattern_items.len() == actual_items.len()
                    && pattern_items
                        .iter()
                        .zip(actual_items.iter())
                        .all(|(p, a)| match (p, a) {
                            (
                                TypeTupleItem::Named {
                                    name: pn, ty: pt, ..
                                },
                                TypeTupleItem::Named {
                                    name: an, ty: at, ..
                                },
                            ) if pn == an => self.witness_pattern_matches(*pt, *at, params),
                            (TypeTupleItem::Positional(pt), TypeTupleItem::Positional(at)) => {
                                self.witness_pattern_matches(*pt, *at, params)
                            }
                            _ => false,
                        })
            }
            (
                TypeKind::Record(pattern_fields, pattern_tail),
                TypeKind::Record(actual_fields, _),
            ) => {
                let all_fields_match = pattern_fields.iter().all(|pattern_field| {
                    actual_fields
                        .iter()
                        .find(|actual_field| actual_field.name == pattern_field.name)
                        .is_some_and(|actual_field| {
                            pattern_field.optional == actual_field.optional
                                && self.witness_pattern_matches(
                                    pattern_field.ty,
                                    actual_field.ty,
                                    params,
                                )
                        })
                });
                all_fields_match
                    && (pattern_tail != RowTail::Closed
                        || pattern_fields.len() == actual_fields.len())
            }
            (
                TypeKind::Union(pattern_variants, pattern_tail),
                TypeKind::Union(actual_variants, _),
            ) => {
                let all_variants_match = pattern_variants.iter().all(|pattern_variant| {
                    actual_variants
                        .iter()
                        .find(|actual_variant| actual_variant.name == pattern_variant.name)
                        .is_some_and(|actual_variant| {
                            match (pattern_variant.payload, actual_variant.payload) {
                                (Some(pattern_payload), Some(actual_payload)) => self
                                    .witness_pattern_matches(
                                        pattern_payload,
                                        actual_payload,
                                        params,
                                    ),
                                (None, None) => true,
                                _ => false,
                            }
                        })
                });
                all_variants_match
                    && (pattern_tail != RowTail::Closed
                        || pattern_variants.len() == actual_variants.len())
            }
            _ => false,
        }
    }
}
