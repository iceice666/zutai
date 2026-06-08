use std::collections::{HashMap, HashSet};

use zutai_hir::{BindingId, BindingKind, HirExprId, HirExprKind, HirLocalBinding, HirRecordField};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirExpr, ThirExprId, ThirExprKind, ThirLocalBinding, ThirRecordField, Type, TypeId, TypeKind,
    TypeRecordField,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn check_expr(&mut self, id: HirExprId, expected: TypeId) -> ThirExprId {
        let expr = self.hir_expr(id);
        match &expr.kind {
            HirExprKind::Record(fields) => self.check_record_expr(id, fields, expected),
            HirExprKind::Block { bindings, result } => {
                self.lower_block_expr(id, bindings, *result, Some(expected))
            }
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
            HirExprKind::UnresolvedIdent(name) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ValueTypeUnavailable { name: name.clone() },
                    span: expr.span,
                });
                self.error_expr(id, expr.span)
            }
            HirExprKind::Tuple(_) => self.unsupported_expr(id, "tuple expressions", expr.span),
            HirExprKind::List(_) => self.unsupported_expr(id, "list expressions", expr.span),
            HirExprKind::Lambda { .. } => {
                self.unsupported_expr(id, "lambda expressions", expr.span)
            }
            HirExprKind::If { .. } => self.unsupported_expr(id, "if expressions", expr.span),
            HirExprKind::Match { .. } => self.unsupported_expr(id, "match expressions", expr.span),
            HirExprKind::Import(_) => self.unsupported_expr(id, "imports", expr.span),
            HirExprKind::OptAccess { .. } => {
                self.unsupported_expr(id, "optional access expressions", expr.span)
            }
            HirExprKind::Binary { .. } => {
                self.unsupported_expr(id, "binary expressions", expr.span)
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
}
