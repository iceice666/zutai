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
            HirExprKind::Integer(value) => {
                let ty = self.int_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Integer(*value),
                    span: expr.span,
                })
            }
            HirExprKind::Float(value) => {
                let ty = self.float_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Float(*value),
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
}
