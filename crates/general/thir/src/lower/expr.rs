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

use super::{Lowerer, RowSolution};

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

    fn infer_perform_expr(
        &mut self,
        id: HirExprId,
        op: &[String],
        arg: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let op = op.join(".");
        match self.lookup_op(&op) {
            Some((param, result)) => {
                match op.as_str() {
                    "fail" => {
                        let never = self.never_type(span);
                        self.unify(result, never, span);
                    }
                    "warn" | "log" => {
                        let unit = self.unit_type(span);
                        self.unify(result, unit, span);
                    }
                    "ask" => {
                        let unit = self.unit_type(span);
                        self.unify(param, unit, span);
                    }
                    _ => {}
                }
                let arg = self.check_expr(arg, param);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty: result,
                    kind: ThirExprKind::Perform { op, arg },
                    span,
                })
            }
            None => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::EffectNotInRow { op: op.clone() },
                    span,
                });
                let arg = self.infer_expr(arg);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty: self.error_type,
                    kind: ThirExprKind::Perform { op, arg },
                    span,
                })
            }
        }
    }

    fn infer_handle_expr(
        &mut self,
        id: HirExprId,
        handled: HirExprId,
        clauses: &[HirHandleClause],
        span: Span,
    ) -> ThirExprId {
        let mut value_clause = None;
        let mut op_clauses = Vec::new();
        for clause in clauses {
            match &clause.op {
                HirHandleOp::Value => {
                    if value_clause.is_none() {
                        value_clause = Some(clause);
                    }
                }
                HirHandleOp::Operation(path) => op_clauses.push((path.join("."), clause)),
            }
        }

        let result_ty = self.fresh_infer_var(span);
        let mut layer = HashMap::new();
        for (name, clause) in &op_clauses {
            let param = self.fresh_infer_var(clause.span);
            let result = self.fresh_infer_var(clause.span);
            layer.insert(name.clone(), (param, result));
        }

        self.handled_stack.push(layer);
        let handled_expr = self.infer_expr(handled);
        let handled_ty = self.expr(handled_expr).ty;
        let layer = self
            .handled_stack
            .pop()
            .expect("handler layer pushed before handled expression");

        let value = match value_clause {
            Some(clause) => Some(self.check_handler_lambda(
                clause.body,
                handled_ty,
                result_ty,
                "value",
                clause.span,
            )),
            None => {
                self.unify(result_ty, handled_ty, span);
                None
            }
        };

        let mut ops = Vec::with_capacity(op_clauses.len());
        for (name, clause) in op_clauses {
            let (op_param, op_result) = layer
                .get(&name)
                .copied()
                .unwrap_or((self.error_type, self.error_type));
            self.resume_stack.push((op_result, result_ty));
            let lambda_body = match &self.hir_expr(clause.body).kind {
                HirExprKind::Lambda { body, .. } => Some(*body),
                _ => None,
            };
            let body =
                self.check_handler_lambda(clause.body, op_param, result_ty, &name, clause.span);
            self.resume_stack.pop();
            if let Some(body_id) = lambda_body {
                self.check_one_shot(body_id, &name, clause.span);
            }
            ops.push(ThirHandleClause {
                op: name,
                body,
                span: clause.span,
            });
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: result_ty,
            kind: ThirExprKind::Handle {
                expr: handled_expr,
                value,
                ops,
            },
            span,
        })
    }

    fn check_handler_lambda(
        &mut self,
        lambda: HirExprId,
        param_ty: TypeId,
        body_ty: TypeId,
        op: &str,
        span: Span,
    ) -> ThirExprId {
        let lambda_expr = self.hir_expr(lambda);
        let HirExprKind::Lambda { params, body } = lambda_expr.kind.clone() else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::HandlerClauseArityMismatch {
                    op: op.to_string(),
                    expected: 1,
                    found: 0,
                },
                span,
            });
            return self.error_expr(lambda, span);
        };

        if params.len() != 1 {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::HandlerClauseArityMismatch {
                    op: op.to_string(),
                    expected: 1,
                    found: params.len(),
                },
                span,
            });
        }

        let mut scoped_bindings = Vec::new();
        let lowered_params = params
            .iter()
            .enumerate()
            .map(|(index, &pat)| {
                let expected = if index == 0 {
                    param_ty
                } else {
                    self.error_type
                };
                self.check_pattern(pat, expected, &mut scoped_bindings)
            })
            .collect();
        let body = self.check_expr(body, body_ty);
        self.clear_scoped_value_types(&scoped_bindings);

        let lambda_ty = self.alloc_type(Type {
            kind: TypeKind::Function {
                from: param_ty,
                to: body_ty,
            },
            span,
        });
        self.alloc_expr(ThirExpr {
            source: lambda,
            ty: lambda_ty,
            kind: ThirExprKind::Lambda {
                params: lowered_params,
                body,
            },
            span,
        })
    }

    fn infer_resume_expr(&mut self, id: HirExprId, value: HirExprId, span: Span) -> ThirExprId {
        let Some((expected, result_ty)) = self.resume_stack.last().copied() else {
            let value = self.infer_expr(value);
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.error_type,
                kind: ThirExprKind::Resume { value },
                span,
            });
        };

        let value = self.infer_expr(value);
        let found = self.expr(value).ty;
        if !self.type_matches(expected, found) {
            let expected = self.type_name(expected);
            let found = self.type_name(found);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ResumeTypeMismatch { expected, found },
                span,
            });
        }
        self.alloc_expr(ThirExpr {
            source: id,
            ty: result_ty,
            kind: ThirExprKind::Resume { value },
            span,
        })
    }

    fn lower_sequence_expr(
        &mut self,
        id: HirExprId,
        items: &[HirExprId],
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some((&last, prefix)) = items.split_last() else {
            let ty = self.unit_type(span);
            return self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::Sequence(Vec::new()),
                span,
            });
        };

        let mut lowered = Vec::with_capacity(items.len());
        for &item in prefix {
            lowered.push(self.infer_expr(item));
        }
        let last = match expected {
            Some(expected) => self.check_expr(last, expected),
            None => self.infer_expr(last),
        };
        lowered.push(last);
        let ty = self.expr(last).ty;
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Sequence(lowered),
            span,
        })
    }

    fn check_one_shot(&mut self, body: HirExprId, op: &str, span: Span) {
        if self.max_resumes(body) >= 2 {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::MultipleResume { op: op.to_string() },
                span,
            });
        }
    }

    fn max_resumes(&self, id: HirExprId) -> usize {
        match &self.hir_expr(id).kind {
            HirExprKind::Resume { value } => 1 + self.max_resumes(*value),
            HirExprKind::Sequence(items) => items.iter().map(|&e| self.max_resumes(e)).sum(),
            HirExprKind::Block { bindings, result } => {
                bindings
                    .iter()
                    .map(|b| self.max_resumes(b.value))
                    .sum::<usize>()
                    + self.max_resumes(*result)
            }
            HirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.max_resumes(*cond)
                    + self
                        .max_resumes(*then_branch)
                        .max(self.max_resumes(*else_branch))
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.max_resumes(*scrutinee)
                    + arms
                        .iter()
                        .map(|arm| self.max_resumes(arm.body))
                        .max()
                        .unwrap_or(0)
            }
            HirExprKind::Apply { func, arg } => self.max_resumes(*func) + self.max_resumes(*arg),
            HirExprKind::Binary { lhs, rhs, .. } => self.max_resumes(*lhs) + self.max_resumes(*rhs),
            HirExprKind::Record(fields) => fields.iter().map(|f| self.max_resumes(f.value)).sum(),
            HirExprKind::Tuple(items) => items
                .iter()
                .map(|item| match item {
                    HirTupleItem::Named { value, .. } | HirTupleItem::Positional(value) => {
                        self.max_resumes(*value)
                    }
                })
                .sum(),
            HirExprKind::List(items) => items.iter().map(|&e| self.max_resumes(e)).sum(),
            HirExprKind::TaggedValue { payload, .. } => self.max_resumes(*payload),
            HirExprKind::Select { receiver, .. }
            | HirExprKind::Access { receiver, .. }
            | HirExprKind::OptAccess { receiver, .. } => self.max_resumes(*receiver),
            HirExprKind::Perform { arg, .. } => self.max_resumes(*arg),
            HirExprKind::Handle { expr, clauses } => {
                self.max_resumes(*expr)
                    + clauses
                        .iter()
                        .filter_map(|clause| match clause.op {
                            HirHandleOp::Value => {
                                Some(self.max_resumes_handler_clause_body(clause.body))
                            }
                            HirHandleOp::Operation(_) => None,
                        })
                        .sum::<usize>()
            }
            HirExprKind::Lambda { .. }
            | HirExprKind::True
            | HirExprKind::False
            | HirExprKind::Integer(_)
            | HirExprKind::Float(_)
            | HirExprKind::String(_)
            | HirExprKind::Atom(_)
            | HirExprKind::BindingRef(_)
            | HirExprKind::UnresolvedIdent(_)
            | HirExprKind::Import(_)
            | HirExprKind::TypeForm(_) => 0,
        }
    }

    fn max_resumes_handler_clause_body(&self, id: HirExprId) -> usize {
        match &self.hir_expr(id).kind {
            HirExprKind::Lambda { body, .. } => self.max_resumes(*body),
            _ => self.max_resumes(id),
        }
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
            if !matches!(
                self.type_arena[self.resolve(func_ty).0 as usize].kind,
                TypeKind::Error
            ) {
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

        // If the function signature contains TypeVars (explicit polymorphism),
        // instantiate them with fresh InferVars so each call site is independent.
        let type_vars: Vec<_> = {
            let mut v = self.collect_type_vars(from);
            let mut from_to = self.collect_type_vars(to);
            from_to.retain(|b| !v.contains(b));
            v.extend(from_to);
            v.sort_by_key(|b| b.0);
            v.dedup();
            v
        };
        let (from, to, instantiation) = if type_vars.is_empty() {
            (from, to, Vec::new())
        } else {
            let mut subst = HashMap::new();
            let mut inst = Vec::new();
            for var in &type_vars {
                let fresh = self.fresh_infer_var(span);
                subst.insert(*var, fresh);
                inst.push(fresh);
            }
            let new_from = self.instantiate_type_vars(from, &subst);
            let new_to = self.instantiate_type_vars(to, &subst);
            (new_from, new_to, inst)
        };

        // Instantiate rigid row variables (`<Rest>`) with fresh flexible row
        // variables so each call site solves the row independently. The same
        // fresh variable is shared across `from` and `to`, preserving the tail.
        let row_params: Vec<_> = {
            let mut v = self.collect_row_params(from);
            let mut from_to = self.collect_row_params(to);
            from_to.retain(|b| !v.contains(b));
            v.extend(from_to);
            v.sort_by_key(|b| b.0);
            v.dedup();
            v
        };
        let (from, to) = if row_params.is_empty() {
            (from, to)
        } else {
            let mut row_subst = HashMap::new();
            for var in &row_params {
                row_subst.insert(*var, self.fresh_row_var());
            }
            let new_from = self.instantiate_row_params(from, &row_subst);
            let new_to = self.instantiate_row_params(to, &row_subst);
            (new_from, new_to)
        };

        let arg = self.check_expr(arg, from);
        // Resolve the return type: InferVars introduced for TypeVars may now be
        // solved after checking the argument. If the fully-applied call returns
        // an effectful computation, discharge that row into the current ambient
        // or handler layer and expose the pure base type to the caller.
        let result_ty = self.resolve(to);
        let effect_ty = self.resolve_alias(to, &mut HashSet::new(), span);
        let result_ty = match self.type_arena[effect_ty.0 as usize].kind.clone() {
            TypeKind::Effect { base, row } => {
                self.discharge_row(&row, span);
                base
            }
            _ => result_ty,
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty: result_ty,
            kind: ThirExprKind::Apply {
                func,
                arg,
                instantiation,
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
            Some(ty) => {
                let ty = match self.poly_schemes.get(&binding).cloned() {
                    Some(scheme) => {
                        let subst: HashMap<u32, TypeId> = scheme
                            .into_iter()
                            .map(|v| (v, self.fresh_infer_var(span)))
                            .collect();
                        self.instantiate_infer_vars(ty, &subst)
                    }
                    None => ty,
                };
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::BindingRef(binding),
                    span,
                })
            }
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
            kind: TypeKind::Record(type_fields, RowTail::Closed),
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
                let then_ty = self.expr(then_branch).ty;
                if self.is_never_type(then_ty) {
                    let else_branch = self.infer_expr(else_branch);
                    let ty = self.expr(else_branch).ty;
                    (then_branch, else_branch, ty)
                } else {
                    let else_branch = self.check_expr(else_branch, then_ty);
                    (then_branch, else_branch, then_ty)
                }
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
        let lhs_resolved = self.resolve(lhs_ty);
        if !self.is_ordered_scalar(lhs_ty) {
            if matches!(
                self.type_arena[lhs_resolved.0 as usize].kind,
                TypeKind::InferVar(_)
            ) {
                let int_ty = self.int_type(span);
                self.unify(lhs_ty, int_ty, span);
            } else if !self.hir_has_ordering_constraint(op) {
                let rhs_ty = self.expr(rhs).ty;
                self.invalid_binary_operands(op, lhs_ty, rhs_ty, span);
            }
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
        let lhs_resolved = self.resolve(lhs_ty);
        if !self.is_numeric_scalar(lhs_ty) {
            if matches!(
                self.type_arena[lhs_resolved.0 as usize].kind,
                TypeKind::InferVar(_)
            ) {
                // Default unresolved numeric context to Int.
                let int_ty = self.int_type(span);
                self.unify(lhs_ty, int_ty, span);
            } else {
                let rhs_ty = self.expr(rhs).ty;
                self.invalid_binary_operands(op, lhs_ty, rhs_ty, span);
            }
        }
        // After possible unification, use the resolved type for the result.
        let result_ty = self.resolve(lhs_ty);
        self.alloc_binary_expr(id, op, lhs, rhs, result_ty, span)
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

    /// Returns `true` if any HIR constraint declares an operator method whose
    /// name matches `bin_op_name(op)`. Used to allow non-scalar ordering
    /// expressions to type-check when a user-defined witness may cover them.
    fn hir_has_ordering_constraint(&self, op: ast::BinOp) -> bool {
        let op_name = bin_op_name(op);
        for &decl_id in &self.hir.decls {
            let decl = self.hir_decl(decl_id);
            if let HirDeclKind::Constraint { methods, .. } = &decl.kind {
                for m in methods {
                    if m.is_operator && m.name == op_name {
                        return true;
                    }
                }
            }
        }
        false
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
        let Some((expected_fields, expected_tail)) = self.record_row(expected, span) else {
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
        // Extra actual fields not named by `expected`: rejected for a closed or
        // rigid row, discarded for an anonymous open row, and captured by a
        // flexible row variable so a named tail preserves them.
        let mut captured_extras: Vec<TypeRecordField> = Vec::new();
        for field in fields {
            let Some(expected_field) = expected_by_name.get(field.name.as_str()) else {
                let value = self.infer_expr(field.value);
                match expected_tail {
                    RowTail::Open => {}
                    RowTail::Infer(_) => captured_extras.push(TypeRecordField {
                        name: field.name.clone(),
                        optional: false,
                        ty: self.expr(value).ty,
                        span: field.span,
                    }),
                    RowTail::Closed | RowTail::Param(_) => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::UnexpectedRecordField {
                                name: field.name.clone(),
                            },
                            span: field.span,
                        });
                    }
                }
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

        // Solve a flexible expected tail with whatever the literal supplied beyond
        // the named fields, so a row-polymorphic call preserves the extras.
        if let RowTail::Infer(r) = expected_tail {
            self.row_subst.insert(
                r,
                RowSolution::Record {
                    fields: captured_extras,
                    tail: RowTail::Closed,
                },
            );
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
            let resolved = self.resolve(receiver_ty);
            if matches!(self.ty(resolved).kind, TypeKind::InferVar(_)) {
                // Field access on an un-inferred value: row-polymorphic inference
                // is not principal here, so an explicit annotation is required.
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::RowAnnotationRequired,
                    span,
                });
            } else {
                let found = self.type_name(receiver_ty);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedRecord { found },
                    span,
                });
            }
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

    /// Type-check `select receiver { f1; f2; }` as a closed record built from the
    /// selected fields in requested order. Desugars to record construction over
    /// field accesses so downstream stages reuse existing record/access nodes.
    fn lower_select_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        fields: &[HirSelectField],
        span: Span,
    ) -> ThirExprId {
        let receiver = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver).ty;
        let Some(rec_fields) = self.record_fields(receiver_ty, span) else {
            let found = self.type_name(receiver_ty);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.error_expr(id, span);
        };
        let mut thir_fields = Vec::with_capacity(fields.len());
        let mut type_fields = Vec::with_capacity(fields.len());
        for sf in fields {
            let Some(rf) = rec_fields.iter().find(|f| f.name == sf.name) else {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::UnknownField {
                        name: sf.name.clone(),
                    },
                    span: sf.span,
                });
                continue;
            };
            let field_ty = if rf.optional {
                self.optional_type(rf.ty, rf.span)
            } else {
                rf.ty
            };
            let access = self.alloc_expr(ThirExpr {
                source: id,
                ty: field_ty,
                kind: ThirExprKind::Access {
                    receiver,
                    field: sf.name.clone(),
                },
                span: sf.span,
            });
            thir_fields.push(ThirRecordField {
                name: sf.name.clone(),
                value: access,
                span: sf.span,
            });
            type_fields.push(TypeRecordField {
                name: sf.name.clone(),
                optional: false,
                ty: field_ty,
                span: sf.span,
            });
        }
        let ty = self.alloc_type(Type {
            kind: TypeKind::Record(type_fields, RowTail::Closed),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    /// Infer the type of a lambda when no expected type is available.
    /// Generates fresh InferVars for each parameter; they are solved by checking
    /// the body, then zonked to concrete types at the end of lowering.
    fn infer_lambda_expr(
        &mut self,
        id: HirExprId,
        params: &[zutai_hir::HirPatId],
        body: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let param_vars: Vec<TypeId> = params.iter().map(|_| self.fresh_infer_var(span)).collect();

        let mut scoped_bindings = Vec::new();
        let lowered_params: Vec<_> = params
            .iter()
            .zip(&param_vars)
            .map(|(&pat_id, &param_ty)| self.check_pattern(pat_id, param_ty, &mut scoped_bindings))
            .collect();

        let body_thir = self.infer_expr(body);
        let body_ty = self.expr(body_thir).ty;
        self.clear_scoped_value_types(&scoped_bindings);

        // Build curried function type: p1 -> p2 -> ... -> body_ty
        let lambda_ty = param_vars.iter().rev().fold(body_ty, |to, &from| {
            let from = self.resolve(from);
            self.alloc_type(crate::ir::Type {
                kind: TypeKind::Function { from, to },
                span,
            })
        });

        self.alloc_expr(ThirExpr {
            source: id,
            ty: lambda_ty,
            kind: ThirExprKind::Lambda {
                params: lowered_params,
                body: body_thir,
            },
            span,
        })
    }

    fn check_lambda_expr(
        &mut self,
        id: HirExprId,
        params: &[zutai_hir::HirPatId],
        body: HirExprId,
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let (param_types, return_type) = self.function_parts(expected, span);

        if param_types.is_empty() {
            let found = self.type_name(expected);
            if !matches!(self.ty(expected).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedFunction { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        }

        if params.len() != param_types.len() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::FunctionClauseArityMismatch {
                    expected: param_types.len(),
                    found: params.len(),
                },
                span,
            });
        }

        let mut scoped_bindings = Vec::new();
        let lowered_params: Vec<_> = params
            .iter()
            .enumerate()
            .map(|(i, &pat_id)| {
                let expected_ty = param_types.get(i).copied().unwrap_or(self.error_type);
                self.check_pattern(pat_id, expected_ty, &mut scoped_bindings)
            })
            .collect();

        let (body_ty, saved_effect_ambient) = self.enter_effectful_result(return_type);
        let body = self.check_expr(body, body_ty);
        self.exit_effectful_result(saved_effect_ambient);
        self.clear_scoped_value_types(&scoped_bindings);

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Lambda {
                params: lowered_params,
                body,
            },
            span,
        })
    }

    fn lower_match_expr(
        &mut self,
        id: HirExprId,
        scrutinee: HirExprId,
        arms: &[zutai_hir::HirClause],
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let scrutinee_thir = self.infer_expr(scrutinee);
        let scrutinee_ty = self.expr(scrutinee_thir).ty;

        if arms.is_empty() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnsupportedFeature {
                    feature: "empty match expressions",
                },
                span,
            });
            return self.error_expr(id, span);
        }

        let mut body_ty = expected;
        let mut fallback_never_ty = None;
        let mut lowered_arms = Vec::with_capacity(arms.len());

        for arm in arms {
            if arm.patterns.len() != 1 {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::MatchArmPatternCountMismatch {
                        found: arm.patterns.len(),
                    },
                    span: arm.span,
                });
            }

            let mut scoped_bindings = Vec::new();
            let patterns: Vec<_> = arm
                .patterns
                .iter()
                .map(|&pat_id| self.check_pattern(pat_id, scrutinee_ty, &mut scoped_bindings))
                .collect();

            let guard = arm.guard.map(|guard_id| {
                let bool_ty = self.bool_type(arm.span);
                self.check_expr(guard_id, bool_ty)
            });

            let body = match body_ty {
                Some(ty) => self.check_expr(arm.body, ty),
                None => {
                    let b = self.infer_expr(arm.body);
                    let inferred = self.expr(b).ty;
                    if self.is_never_type(inferred) {
                        fallback_never_ty.get_or_insert(inferred);
                    } else {
                        body_ty = Some(inferred);
                    }
                    b
                }
            };

            self.clear_scoped_value_types(&scoped_bindings);

            lowered_arms.push(crate::ir::ThirClause {
                patterns,
                guard,
                body,
                span: arm.span,
            });
        }

        // Only check coverage when every arm has the expected single pattern;
        // an arm-count mismatch already produced a diagnostic and would yield a
        // malformed matrix.
        if lowered_arms.iter().all(|arm| arm.patterns.len() == 1) {
            self.check_match_exhaustiveness(&lowered_arms, &[scrutinee_ty], span);
        }

        let ty = body_ty.or(fallback_never_ty).unwrap_or(self.error_type);
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Match {
                scrutinee: scrutinee_thir,
                arms: lowered_arms,
            },
            span,
        })
    }

    fn lower_opt_access_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        field: &str,
        span: Span,
    ) -> ThirExprId {
        let receiver_thir = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver_thir).ty;

        let Some(inner) = self.optional_inner_type(receiver_ty, span) else {
            let found = self.type_name(receiver_ty);
            if !matches!(self.ty(receiver_ty).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedOptional { found },
                    span,
                });
            }
            return self.error_expr(id, span);
        };

        let Some(fields) = self.record_fields(inner, span) else {
            let found = self.type_name(inner);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.error_expr(id, span);
        };

        let Some(record_field) = fields.iter().find(|f| f.name == field).cloned() else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnknownField {
                    name: field.to_string(),
                },
                span,
            });
            return self.error_expr(id, span);
        };

        let ty = self.optional_type(record_field.ty, span);
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::OptionalAccess {
                receiver: receiver_thir,
                field: field.to_string(),
            },
            span,
        })
    }

    fn resolve_alias_for_expr(&mut self, ty: TypeId) -> TypeId {
        use std::collections::HashSet;

        self.resolve_alias(ty, &mut HashSet::new(), self.ty(ty).span)
    }

    /// Lower a `#tag { payload }` expression.
    ///
    /// In check mode (`expected == Some(union_ty)`), the variant's record payload
    /// type is threaded into the payload expression.  In infer mode, the payload
    /// is inferred and a singleton union type is synthesised.
    fn lower_tagged_value_expr(
        &mut self,
        id: HirExprId,
        tag: &str,
        payload: HirExprId,
        expected: Option<TypeId>,
        span: Span,
    ) -> ThirExprId {
        use std::collections::HashSet;

        if let Some(expected_ty) = expected {
            let resolved = self.resolve_alias(expected_ty, &mut HashSet::new(), span);
            let kind = self.ty(resolved).kind.clone();

            match kind {
                TypeKind::Union(variants, _) => {
                    let variant = variants.iter().find(|v| v.name == tag).cloned();
                    match variant {
                        Some(v) => {
                            let payload_expr = match v.payload {
                                Some(record_ty) => self.check_expr(payload, record_ty),
                                None => {
                                    // No payload expected — infer it anyway (will unify to unit)
                                    self.infer_expr(payload)
                                }
                            };
                            return self.alloc_expr(ThirExpr {
                                source: id,
                                ty: expected_ty,
                                kind: ThirExprKind::TaggedValue {
                                    tag: tag.to_string(),
                                    payload: payload_expr,
                                },
                                span,
                            });
                        }
                        None => {
                            // Unknown variant — fall through to infer+check below
                        }
                    }
                }
                TypeKind::Optional(inner) if tag == "some" => {
                    let record_ty = self.alloc_type(crate::ir::Type {
                        kind: TypeKind::Record(
                            vec![TypeRecordField {
                                name: "value".to_string(),
                                optional: false,
                                ty: inner,
                                span,
                            }],
                            RowTail::Closed,
                        ),
                        span,
                    });
                    let payload_expr = self.check_expr(payload, record_ty);
                    return self.alloc_expr(ThirExpr {
                        source: id,
                        ty: expected_ty,
                        kind: ThirExprKind::TaggedValue {
                            tag: tag.to_string(),
                            payload: payload_expr,
                        },
                        span,
                    });
                }
                _ => {}
            }
        }

        // Infer mode (or unknown variant): infer payload, synthesise a singleton union type.
        let payload_expr = self.infer_expr(payload);
        let payload_ty = self.expr(payload_expr).ty;
        let variant = crate::ir::UnionVariant {
            name: tag.to_string(),
            payload: Some(payload_ty),
            span,
        };
        let ty = self.alloc_type(crate::ir::Type {
            kind: TypeKind::Union(vec![variant], RowTail::Closed),
            span,
        });

        if let Some(expected_ty) = expected
            && !self.type_matches(expected_ty, ty)
        {
            self.type_mismatch(expected_ty, ty, span);
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::TaggedValue {
                tag: tag.to_string(),
                payload: payload_expr,
            },
            span,
        })
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
