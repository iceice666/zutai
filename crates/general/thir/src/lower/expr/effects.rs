use super::*;

impl<'hir> Lowerer<'hir> {
    pub(super) fn infer_perform_expr(
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

    pub(super) fn infer_handle_expr(
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
        let mut layer = FxHashMap::default();
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

    pub(super) fn check_handler_lambda(
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

    pub(super) fn infer_resume_expr(
        &mut self,
        id: HirExprId,
        value: HirExprId,
        span: Span,
    ) -> ThirExprId {
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

    pub(super) fn check_one_shot(&mut self, body: HirExprId, op: &str, span: Span) {
        if self.max_resumes(body) >= 2 {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::MultipleResume { op: op.to_string() },
                span,
            });
        }
    }

    pub(super) fn max_resumes(&self, id: HirExprId) -> usize {
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
            HirExprKind::RecordUpdate { receiver, fields } => {
                self.max_resumes(*receiver)
                    + fields
                        .iter()
                        .map(|field| self.max_resumes(field.value))
                        .sum::<usize>()
            }
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
            | HirExprKind::Integer(..)
            | HirExprKind::Float(..)
            | HirExprKind::Posit(_)
            | HirExprKind::String(_)
            | HirExprKind::Atom(_)
            | HirExprKind::BindingRef(_)
            | HirExprKind::UnresolvedIdent(_)
            | HirExprKind::Import(_)
            | HirExprKind::TypeForm(_)
            | HirExprKind::WitnessReflect { .. } => 0,
        }
    }

    pub(super) fn max_resumes_handler_clause_body(&self, id: HirExprId) -> usize {
        match &self.hir_expr(id).kind {
            HirExprKind::Lambda { body, .. } => self.max_resumes(*body),
            _ => self.max_resumes(id),
        }
    }
}
