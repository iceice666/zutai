use std::collections::{HashMap, HashSet};

use zutai_hir::{
    BindingId, BindingKind, HirExprId, HirExprKind, HirLocalBinding, HirRecordField, HirTupleItem,
};
use zutai_syntax::Span;
use zutai_syntax::ast;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirExpr, ThirExprId, ThirExprKind, ThirLocalBinding, ThirRecordField, Type, TypeId, TypeKind,
    TypeRecordField, TypeTupleItem,
};

use super::Lowerer;

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
            HirExprKind::Lambda { .. } => {
                self.unsupported_expr(id, "lambda expressions", expr.span)
            }
            HirExprKind::Match { .. } => self.unsupported_expr(id, "match expressions", expr.span),
            HirExprKind::Import(_) => self.unsupported_expr(id, "imports", expr.span),
            HirExprKind::OptAccess { .. } => {
                self.unsupported_expr(id, "optional access expressions", expr.span)
            }
        }
    }

    fn lower_block_expr(
        &mut self,
        id: HirExprId,
        bindings: &[HirLocalBinding],
        result: HirExprId,
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let mut scoped_bindings = Vec::with_capacity(bindings.len());
        let bindings = bindings
            .iter()
            .map(|binding| {
                let value = self.infer_expr(binding.value);
                let ty = self.expr(value).ty;
                self.value_types.insert(binding.binding, ty);
                scoped_bindings.push(binding.binding);
                ThirLocalBinding {
                    binding: binding.binding,
                    ty,
                    value,
                    span: binding.span,
                }
            })
            .collect();
        let result = match expected {
            Some(expected) => self.check_expr(result, expected),
            None => self.infer_expr(result),
        };
        self.clear_scoped_value_types(&scoped_bindings);
        let ty = self.expr(result).ty;

        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Block { bindings, result },
            span,
        })
    }

    fn lower_apply_expr(
        &mut self,
        id: HirExprId,
        func: HirExprId,
        arg: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let func = self.infer_expr(func);
        let func_ty = self.expr(func).ty;
        let Some((from, to)) = self.function_input_output(func_ty, span) else {
            let found = self.type_name(func_ty);
            if !matches!(self.ty(func_ty).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedFunction { found },
                    span,
                });
            }
            let arg = self.infer_expr(arg);
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.error_type,
                kind: ThirExprKind::Apply {
                    func,
                    arg,
                    instantiation: Vec::new(),
                },
                span,
            });
        };
        let arg = self.check_expr(arg, from);
        self.alloc_expr(ThirExpr {
            source: id,
            ty: to,
            kind: ThirExprKind::Apply {
                func,
                arg,
                instantiation: Vec::new(),
            },
            span,
        })
    }

    fn lower_binding_ref(&mut self, id: HirExprId, binding: BindingId, span: Span) -> ThirExprId {
        let binding_info = &self.hir.bindings[binding.0 as usize];
        if matches!(
            binding_info.kind,
            BindingKind::BuiltinType | BindingKind::TopType
        ) {
            let value = if binding_info.kind == BindingKind::TopType {
                self.alias_type(binding, span)
            } else {
                self.builtin_type_by_name(&binding_info.name, span)
                    .unwrap_or(self.error_type)
            };
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.type_type,
                kind: ThirExprKind::TypeValue(value),
                span,
            });
        }

        match self.value_types.get(&binding).copied() {
            Some(ty) => self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::BindingRef(binding),
                span,
            }),
            None => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ValueTypeUnavailable {
                        name: binding_info.name.clone(),
                    },
                    span,
                });
                self.error_expr(id, span)
            }
        }
    }

    fn infer_record_expr(
        &mut self,
        id: HirExprId,
        fields: &[HirRecordField],
        span: Span,
    ) -> ThirExprId {
        let mut thir_fields = Vec::with_capacity(fields.len());
        let mut type_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let value = self.infer_expr(field.value);
            let ty = self.expr(value).ty;
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
            type_fields.push(TypeRecordField {
                name: field.name.clone(),
                optional: false,
                ty,
                span: field.span,
            });
        }
        let ty = self.alloc_type(Type {
            kind: TypeKind::Record(type_fields),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    fn infer_tuple_expr(
        &mut self,
        id: HirExprId,
        items: &[HirTupleItem],
        span: Span,
    ) -> ThirExprId {
        let mut thir_items = Vec::with_capacity(items.len());
        let mut type_items = Vec::with_capacity(items.len());
        for item in items {
            match item {
                HirTupleItem::Named { name, value, span } => {
                    let value = self.infer_expr(*value);
                    let ty = self.expr(value).ty;
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                    type_items.push(TypeTupleItem::Named {
                        name: name.clone(),
                        ty,
                        span: *span,
                    });
                }
                HirTupleItem::Positional(value) => {
                    let value = self.infer_expr(*value);
                    let ty = self.expr(value).ty;
                    thir_items.push(crate::ir::ThirTupleItem::Positional(value));
                    type_items.push(TypeTupleItem::Positional(ty));
                }
            }
        }
        let ty = self.alloc_type(Type {
            kind: TypeKind::Tuple(type_items),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Tuple(thir_items),
            span,
        })
    }

    fn check_tuple_expr(
        &mut self,
        id: HirExprId,
        items: &[HirTupleItem],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let resolved = self.resolve_alias_for_expr(expected);
        let TypeKind::Tuple(expected_items) = self.ty(resolved).kind.clone() else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedTuple { found },
                span,
            });
            return self.infer_tuple_expr(id, items, span);
        };
        if expected_items.len() != items.len() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TupleArityMismatch {
                    expected: expected_items.len(),
                    found: items.len(),
                },
                span,
            });
        }

        let mut thir_items = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let expected_item = expected_items.get(index);
            match (item, expected_item) {
                (
                    HirTupleItem::Named { name, value, span },
                    Some(TypeTupleItem::Named {
                        name: expected_name,
                        ty,
                        ..
                    }),
                ) => {
                    if name != expected_name {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                                expected: expected_name.clone(),
                                found: name.clone(),
                            },
                            span: *span,
                        });
                    }
                    let value = self.check_expr(*value, *ty);
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                }
                (
                    HirTupleItem::Named { name, value, span },
                    Some(TypeTupleItem::Positional(ty)),
                ) => {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                            expected: "<positional>".to_string(),
                            found: name.clone(),
                        },
                        span: *span,
                    });
                    let value = self.check_expr(*value, *ty);
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                }
                (HirTupleItem::Positional(value), Some(TypeTupleItem::Positional(ty))) => {
                    thir_items.push(crate::ir::ThirTupleItem::Positional(
                        self.check_expr(*value, *ty),
                    ));
                }
                (
                    HirTupleItem::Positional(value),
                    Some(TypeTupleItem::Named {
                        name: expected_name,
                        ty,
                        ..
                    }),
                ) => {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::TupleFieldNameMismatch {
                            expected: expected_name.clone(),
                            found: "<positional>".to_string(),
                        },
                        span,
                    });
                    thir_items.push(crate::ir::ThirTupleItem::Positional(
                        self.check_expr(*value, *ty),
                    ));
                }
                (HirTupleItem::Named { name, value, span }, None) => {
                    let value = self.infer_expr(*value);
                    thir_items.push(crate::ir::ThirTupleItem::Named {
                        name: name.clone(),
                        value,
                        span: *span,
                    });
                }
                (HirTupleItem::Positional(value), None) => {
                    thir_items.push(crate::ir::ThirTupleItem::Positional(
                        self.infer_expr(*value),
                    ));
                }
            }
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Tuple(thir_items),
            span,
        })
    }

    fn infer_list_expr(&mut self, id: HirExprId, items: &[HirExprId], span: Span) -> ThirExprId {
        let Some((first, rest)) = items.split_first() else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::EmptyListNeedsType,
                span,
            });
            return self.error_expr(id, span);
        };
        let first = self.infer_expr(*first);
        let item_ty = self.expr(first).ty;
        let mut lowered_items = Vec::with_capacity(items.len());
        lowered_items.push(first);
        lowered_items.extend(rest.iter().map(|item| self.check_expr(*item, item_ty)));
        let ty = self.alloc_type(Type {
            kind: TypeKind::List(item_ty),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::List(lowered_items),
            span,
        })
    }

    fn check_list_expr(
        &mut self,
        id: HirExprId,
        items: &[HirExprId],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some(item_ty) = self.list_item_type(expected, span) else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedList { found },
                span,
            });
            return self.infer_list_expr(id, items, span);
        };
        let items = items
            .iter()
            .map(|item| self.check_expr(*item, item_ty))
            .collect();
        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::List(items),
            span,
        })
    }

    fn lower_if_expr(
        &mut self,
        id: HirExprId,
        cond: HirExprId,
        then_branch: HirExprId,
        else_branch: HirExprId,
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let bool_ty = self.bool_type(span);
        let cond = self.check_expr(cond, bool_ty);
        let (then_branch, else_branch, ty) = match expected {
            Some(expected) => {
                let then_branch = self.check_expr(then_branch, expected);
                let else_branch = self.check_expr(else_branch, expected);
                (then_branch, else_branch, expected)
            }
            None => {
                let then_branch = self.infer_expr(then_branch);
                let ty = self.expr(then_branch).ty;
                let else_branch = self.check_expr(else_branch, ty);
                (then_branch, else_branch, ty)
            }
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            },
            span,
        })
    }

    fn lower_binary_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        match op {
            ast::BinOp::And | ast::BinOp::Or => self.lower_bool_binary_expr(id, op, lhs, rhs, span),
            ast::BinOp::Eq | ast::BinOp::Ne => self.lower_equality_expr(id, op, lhs, rhs, span),
            ast::BinOp::Lt | ast::BinOp::Le | ast::BinOp::Gt | ast::BinOp::Ge => {
                self.lower_ordering_expr(id, op, lhs, rhs, span)
            }
            ast::BinOp::Add | ast::BinOp::Sub | ast::BinOp::Mul | ast::BinOp::Div => {
                self.lower_arithmetic_expr(id, op, lhs, rhs, span)
            }
            ast::BinOp::Coalesce => self.lower_coalesce_expr(id, lhs, rhs, span),
        }
    }

    fn lower_bool_binary_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let ty = self.bool_type(span);
        let lhs = self.check_expr(lhs, ty);
        let rhs = self.check_expr(rhs, ty);
        self.alloc_binary_expr(id, op, lhs, rhs, ty, span)
    }

    fn lower_equality_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let rhs = self.check_expr(rhs, lhs_ty);
        let ty = self.bool_type(span);
        self.alloc_binary_expr(id, op, lhs, rhs, ty, span)
    }

    fn lower_ordering_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let rhs = self.check_expr(rhs, lhs_ty);
        if !self.is_ordered_scalar(lhs_ty) {
            let rhs_ty = self.expr(rhs).ty;
            self.invalid_binary_operands(op, lhs_ty, rhs_ty, span);
        }
        let ty = self.bool_type(span);
        self.alloc_binary_expr(id, op, lhs, rhs, ty, span)
    }

    fn lower_arithmetic_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let rhs = self.check_expr(rhs, lhs_ty);
        if !self.is_numeric_scalar(lhs_ty) {
            let rhs_ty = self.expr(rhs).ty;
            self.invalid_binary_operands(op, lhs_ty, rhs_ty, span);
        }
        self.alloc_binary_expr(id, op, lhs, rhs, lhs_ty, span)
    }

    fn lower_coalesce_expr(
        &mut self,
        id: HirExprId,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let Some(inner) = self.optional_inner_type(lhs_ty, span) else {
            let found = self.type_name(lhs_ty);
            if !matches!(self.ty(lhs_ty).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedOptional { found },
                    span,
                });
            }
            let rhs = self.infer_expr(rhs);
            return self.alloc_binary_expr(
                id,
                ast::BinOp::Coalesce,
                lhs,
                rhs,
                self.error_type,
                span,
            );
        };
        let rhs = self.check_expr(rhs, inner);
        self.alloc_binary_expr(id, ast::BinOp::Coalesce, lhs, rhs, inner, span)
    }

    fn alloc_binary_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: ThirExprId,
        rhs: ThirExprId,
        ty: TypeId,
        span: Span,
    ) -> ThirExprId {
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Binary { op, lhs, rhs },
            span,
        })
    }

    fn is_numeric_scalar(&mut self, ty: TypeId) -> bool {
        let resolved = self.resolve_alias_for_expr(ty);
        matches!(self.ty(resolved).kind, TypeKind::Int | TypeKind::Float)
    }

    fn is_ordered_scalar(&mut self, ty: TypeId) -> bool {
        self.is_numeric_scalar(ty) || {
            let resolved = self.resolve_alias_for_expr(ty);
            matches!(self.ty(resolved).kind, TypeKind::Text)
        }
    }

    fn invalid_binary_operands(&mut self, op: ast::BinOp, lhs: TypeId, rhs: TypeId, span: Span) {
        let lhs = self.type_name(lhs);
        let rhs = self.type_name(rhs);
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::InvalidBinaryOperands {
                op: bin_op_name(op),
                lhs,
                rhs,
            },
            span,
        });
    }

    fn check_record_expr(
        &mut self,
        id: HirExprId,
        fields: &[HirRecordField],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some(expected_fields) = self.record_fields(expected, span) else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.infer_record_expr(id, fields, span);
        };

        let expected_by_name: HashMap<_, _> = expected_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        let actual_names: HashSet<_> = fields.iter().map(|field| field.name.as_str()).collect();

        for expected_field in &expected_fields {
            if !expected_field.optional && !actual_names.contains(expected_field.name.as_str()) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::MissingRecordField {
                        name: expected_field.name.clone(),
                    },
                    span,
                });
            }
        }

        let mut thir_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(expected_field) = expected_by_name.get(field.name.as_str()) else {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::UnexpectedRecordField {
                        name: field.name.clone(),
                    },
                    span: field.span,
                });
                let value = self.infer_expr(field.value);
                thir_fields.push(ThirRecordField {
                    name: field.name.clone(),
                    value,
                    span: field.span,
                });
                continue;
            };
            let value = self.check_expr(field.value, expected_field.ty);
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    fn lower_access_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        field: &str,
        span: Span,
    ) -> ThirExprId {
        let receiver = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver).ty;
        let Some(fields) = self.record_fields(receiver_ty, span) else {
            let found = self.type_name(receiver_ty);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.error_expr(id, span);
        };
        let Some(record_field) = fields.iter().find(|candidate| candidate.name == field) else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnknownField {
                    name: field.to_string(),
                },
                span,
            });
            return self.error_expr(id, span);
        };
        let ty = if record_field.optional {
            self.optional_type(record_field.ty, record_field.span)
        } else {
            record_field.ty
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Access {
                receiver,
                field: field.to_string(),
            },
            span,
        })
    }

    fn resolve_alias_for_expr(&mut self, ty: TypeId) -> TypeId {
        use std::collections::HashSet;

        self.resolve_alias(ty, &mut HashSet::new(), self.ty(ty).span)
    }
}

fn bin_op_name(op: ast::BinOp) -> &'static str {
    match op {
        ast::BinOp::Mul => "*",
        ast::BinOp::Div => "/",
        ast::BinOp::Add => "+",
        ast::BinOp::Sub => "-",
        ast::BinOp::Eq => "==",
        ast::BinOp::Ne => "!=",
        ast::BinOp::Lt => "<",
        ast::BinOp::Le => "<=",
        ast::BinOp::Gt => ">",
        ast::BinOp::Ge => ">=",
        ast::BinOp::And => "&&",
        ast::BinOp::Or => "||",
        ast::BinOp::Coalesce => "??",
    }
}
