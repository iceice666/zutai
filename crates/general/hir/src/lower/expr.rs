use super::types::clone_import_source;
use super::*;

impl Lowerer {
    pub(super) fn lower_expr(&mut self, expr: &ast::Expr) -> HirExprId {
        let span = expr.span();
        let kind = match expr {
            ast::Expr::True(_) => HirExprKind::True,
            ast::Expr::False(_) => HirExprKind::False,
            ast::Expr::Integer { value, postfix, .. } => HirExprKind::Integer(*value, *postfix),
            ast::Expr::Float { value, postfix, .. } => HirExprKind::Float(*value, *postfix),
            ast::Expr::String { value, .. } => HirExprKind::String(value.clone()),
            ast::Expr::Atom { name, .. } => HirExprKind::Atom(name.clone()),
            ast::Expr::TaggedValue { tag, payload, .. } => HirExprKind::TaggedValue {
                tag: tag.clone(),
                payload: self.lower_expr(payload),
            },
            ast::Expr::Ident { name, span } => self.lower_ident(name, *span),
            ast::Expr::Record { fields, .. } => HirExprKind::Record(
                fields
                    .iter()
                    .map(|field| HirRecordField {
                        name: field.name.clone(),
                        value: self.lower_expr(&field.value),
                        span: field.span,
                    })
                    .collect(),
            ),
            ast::Expr::RecordUpdate {
                receiver, fields, ..
            } => {
                let receiver = self.lower_expr(receiver);
                let fields = fields
                    .iter()
                    .map(|field| HirRecordField {
                        name: field.name.clone(),
                        value: self.lower_expr(&field.value),
                        span: field.span,
                    })
                    .collect();
                HirExprKind::RecordUpdate { receiver, fields }
            }
            ast::Expr::Tuple { items, .. } => HirExprKind::Tuple(
                items
                    .iter()
                    .map(|item| match item {
                        ast::TupleItem::Named { name, value, span } => HirTupleItem::Named {
                            name: name.clone(),
                            value: self.lower_expr(value),
                            span: *span,
                        },
                        ast::TupleItem::Positional(value) => {
                            HirTupleItem::Positional(self.lower_expr(value))
                        }
                    })
                    .collect(),
            ),
            ast::Expr::List { items, .. } => {
                HirExprKind::List(items.iter().map(|item| self.lower_expr(item)).collect())
            }
            ast::Expr::Block {
                bindings, result, ..
            } => {
                self.push_scope();
                let bindings = bindings
                    .iter()
                    .map(|binding| {
                        let value = self.lower_expr(&binding.value);
                        let binding_id = self.define_current(
                            binding.name.clone(),
                            BindingKind::Local,
                            binding.span,
                        );
                        HirLocalBinding {
                            binding: binding_id,
                            value,
                            span: binding.span,
                        }
                    })
                    .collect();
                let result = self.lower_expr(result);
                self.pop_scope();
                HirExprKind::Block { bindings, result }
            }
            ast::Expr::Lambda { params, body, .. } => {
                self.push_scope();
                let params = params
                    .iter()
                    .map(|param| self.lower_pattern(param))
                    .collect();
                let body = self.lower_expr(body);
                self.pop_scope();
                HirExprKind::Lambda { params, body }
            }
            ast::Expr::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => HirExprKind::If {
                cond: self.lower_expr(cond),
                then_branch: self.lower_expr(then_branch),
                else_branch: self.lower_expr(else_branch),
            },
            ast::Expr::Match {
                scrutinee, arms, ..
            } => HirExprKind::Match {
                scrutinee: self.lower_expr(scrutinee),
                arms: arms.iter().map(|arm| self.lower_clause(arm)).collect(),
            },
            ast::Expr::Import { source, .. } => HirExprKind::Import(clone_import_source(source)),
            ast::Expr::TypeForm { ty, .. } => HirExprKind::TypeForm(self.lower_type(ty)),
            ast::Expr::Select {
                receiver, fields, ..
            } => {
                let receiver = self.lower_expr(receiver);
                let fields = self.lower_select_fields(fields);
                HirExprKind::Select { receiver, fields }
            }
            ast::Expr::Perform { op, arg, .. } => HirExprKind::Perform {
                op: op.clone(),
                arg: self.lower_expr(arg),
            },
            ast::Expr::Handle { expr, clauses, .. } => {
                let expr = self.lower_expr(expr);
                let clauses = clauses
                    .iter()
                    .map(|clause| self.lower_handle_clause(clause))
                    .collect();
                HirExprKind::Handle { expr, clauses }
            }
            ast::Expr::Resume { value, .. } => {
                if self.handler_clause != Some(HandlerClauseKind::Operation) {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::ResumeOutsideHandler,
                        span,
                    });
                }
                HirExprKind::Resume {
                    value: self.lower_expr(value),
                }
            }
            ast::Expr::Sequence { items, .. } => {
                HirExprKind::Sequence(items.iter().map(|item| self.lower_expr(item)).collect())
            }
            ast::Expr::Apply { func, arg, .. } => HirExprKind::Apply {
                func: self.lower_expr(func),
                arg: self.lower_expr(arg),
            },
            ast::Expr::Access {
                receiver, field, ..
            } => HirExprKind::Access {
                receiver: self.lower_expr(receiver),
                field: field.clone(),
            },
            ast::Expr::OptAccess {
                receiver, field, ..
            } => HirExprKind::OptAccess {
                receiver: self.lower_expr(receiver),
                field: field.clone(),
            },
            ast::Expr::Binary { op, lhs, rhs, .. } => HirExprKind::Binary {
                op: *op,
                lhs: self.lower_expr(lhs),
                rhs: self.lower_expr(rhs),
            },
            ast::Expr::Pipeline { dir, lhs, rhs, .. } => {
                let lhs = self.lower_expr(lhs);
                let rhs = self.lower_expr(rhs);
                match dir {
                    ast::PipelineDir::Forward => HirExprKind::Apply {
                        func: rhs,
                        arg: lhs,
                    },
                    ast::PipelineDir::Backward => HirExprKind::Apply {
                        func: lhs,
                        arg: rhs,
                    },
                }
            }
        };
        self.alloc_expr(HirExpr { kind, span })
    }

    pub(super) fn lower_ident(&mut self, name: &str, span: Span) -> HirExprKind {
        match self.resolve(name) {
            Some(binding) => HirExprKind::BindingRef(binding),
            None => {
                self.diagnostics.push(HirDiagnostic {
                    kind: HirDiagnosticKind::UnknownIdentifier {
                        name: name.to_string(),
                    },
                    span,
                });
                HirExprKind::UnresolvedIdent(name.to_string())
            }
        }
    }

    pub(super) fn lower_select_fields(
        &mut self,
        fields: &[ast::SelectField],
    ) -> Vec<HirSelectField> {
        fields
            .iter()
            .map(|field| HirSelectField {
                name: field.name.clone(),
                span: field.span,
            })
            .collect()
    }

    pub(super) fn lower_handle_clause(&mut self, clause: &ast::HandleClause) -> HirHandleClause {
        // `value` is the special final-value handler clause; every other path
        // names a performed operation. Only operation clauses license `resume`.
        let op = if clause.op.len() == 1 && clause.op[0] == "value" {
            HirHandleOp::Value
        } else {
            HirHandleOp::Operation(clause.op.clone())
        };
        let clause_kind = match &op {
            HirHandleOp::Value => HandlerClauseKind::Value,
            HirHandleOp::Operation(_) => HandlerClauseKind::Operation,
        };
        let saved = self.handler_clause;
        self.handler_clause = Some(clause_kind);
        let body = self.lower_expr(&clause.body);
        self.handler_clause = saved;
        HirHandleClause {
            op,
            body,
            span: clause.span,
        }
    }
}
