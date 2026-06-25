use super::*;

impl Lowerer {
    /// Wrap `body` in a unit-ignoring thunk `\_. body` — the deferral that makes
    /// a codata `Stream` cell lazy (used by generator desugaring).
    fn thunk(&mut self, body: HirExprId, span: Span) -> HirExprId {
        let wildcard = self.alloc_pat(HirPat {
            kind: HirPatKind::Wildcard,
            span,
        });
        self.alloc_expr(HirExpr {
            kind: HirExprKind::Lambda {
                params: vec![wildcard],
                body,
            },
            span,
        })
    }

    /// Desugar `stream { … }` into demand-driven codata (V3-G1, with richer
    /// `yield` from V3-G3). A `Stream A` is `Unit -> StreamCell A`, so the block
    /// lowers by continuation-passing onto the `#nil`/`#cons` cell: `yield` conses
    /// one element, a conditional yields per branch, and `yield from` splices a
    /// sub-stream in tail position (the canonical recursive/loop generator). No
    /// second iterator abstraction is introduced; the result steps identically to
    /// the equivalent `unfold`.
    fn lower_generator(&mut self, body: &[ast::GenStmt], span: Span) -> HirExprId {
        self.lower_gen_stmts(body, None, span)
    }

    /// Lower a generator statement list against its continuation — the stream
    /// that follows this block. `None` is the terminal `\_. #nil`; `Some(b)` is a
    /// `Stream`-valued local that later statements continue onto.
    fn lower_gen_stmts(
        &mut self,
        stmts: &[ast::GenStmt],
        cont: Option<BindingId>,
        span: Span,
    ) -> HirExprId {
        let Some((stmt, rest)) = stmts.split_first() else {
            return self.cont_stream(cont, span);
        };
        match stmt {
            ast::GenStmt::Yield { value, span: ys } => {
                let head = self.lower_expr(value);
                let tail = self.lower_gen_stmts(rest, cont, span);
                self.gen_cons(head, tail, *ys)
            }
            ast::GenStmt::YieldFrom { stream, span: ys } => {
                // `yield from s` is "every element of s, then the continuation".
                // The codata cell has no shared append, so this is sound only in
                // tail position (nothing follows and the continuation is the
                // terminal `#nil`) — exactly the canonical recursive/loop
                // generator. A non-tail splice is reported, never miscompiled.
                if !rest.is_empty() || cont.is_some() {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::NonTailYieldFrom,
                        span: *ys,
                    });
                }
                self.lower_expr(stream)
            }
            ast::GenStmt::If {
                cond,
                then_body,
                else_body,
                span: ifs,
            } => {
                if rest.is_empty() {
                    // Tail conditional: both branches inherit the parent
                    // continuation directly, so a tail `yield from` inside a
                    // branch stays in tail position.
                    let cond_id = self.lower_expr(cond);
                    let then_id = self.lower_gen_stmts(then_body, cont, span);
                    let else_id = self.lower_gen_stmts(else_body, cont, span);
                    self.alloc_expr(HirExpr {
                        kind: HirExprKind::If {
                            cond: cond_id,
                            then_branch: then_id,
                            else_branch: else_id,
                        },
                        span: *ifs,
                    })
                } else {
                    // Non-tail conditional: bind the shared continuation to a
                    // fresh local so both branches reference it once (no aliased
                    // node) and it is built at most once per chosen branch.
                    let rest_id = self.lower_gen_stmts(rest, cont, span);
                    let bind = self.alloc_synthetic_local("gen-cont", span);
                    let cond_id = self.lower_expr(cond);
                    let then_id = self.lower_gen_stmts(then_body, Some(bind), span);
                    let else_id = self.lower_gen_stmts(else_body, Some(bind), span);
                    let if_id = self.alloc_expr(HirExpr {
                        kind: HirExprKind::If {
                            cond: cond_id,
                            then_branch: then_id,
                            else_branch: else_id,
                        },
                        span: *ifs,
                    });
                    self.alloc_expr(HirExpr {
                        kind: HirExprKind::Block {
                            bindings: vec![HirLocalBinding {
                                binding: bind,
                                annotation: None,
                                value: rest_id,
                                span,
                            }],
                            result: if_id,
                        },
                        span: *ifs,
                    })
                }
            }
        }
    }

    /// The continuation stream as an expression: a fresh reference to the bound
    /// local, or the terminal `\_. #nil` thunk when there is none.
    fn cont_stream(&mut self, cont: Option<BindingId>, span: Span) -> HirExprId {
        match cont {
            Some(b) => self.alloc_expr(HirExpr {
                kind: HirExprKind::BindingRef(b),
                span,
            }),
            None => {
                let nil = self.alloc_expr(HirExpr {
                    kind: HirExprKind::Atom("nil".to_string()),
                    span,
                });
                self.thunk(nil, span)
            }
        }
    }

    /// Build a codata cons cell `\_. #cons { head; tail }` — a `Stream` value.
    fn gen_cons(&mut self, head: HirExprId, tail: HirExprId, span: Span) -> HirExprId {
        let payload = self.alloc_expr(HirExpr {
            kind: HirExprKind::Record(vec![
                HirRecordField {
                    name: "head".to_string(),
                    value: head,
                    span,
                },
                HirRecordField {
                    name: "tail".to_string(),
                    value: tail,
                    span,
                },
            ]),
            span,
        });
        let cell = self.alloc_expr(HirExpr {
            kind: HirExprKind::TaggedValue {
                tag: "cons".to_string(),
                payload,
            },
            span,
        });
        self.thunk(cell, span)
    }

    /// Allocate a fresh, unscoped `Local` binding for a desugaring-internal value.
    /// It is referenced only by `BindingId` (never by name), so it cannot collide
    /// with or shadow user names.
    fn alloc_synthetic_local(&mut self, hint: &str, span: Span) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            name: format!("${hint}#{}", id.0),
            kind: BindingKind::Local,
            span,
        });
        id
    }

    pub(super) fn lower_expr(&mut self, expr: &ast::Expr) -> HirExprId {
        let span = expr.span();
        let kind = match expr {
            ast::Expr::True(_) => HirExprKind::True,
            ast::Expr::False(_) => HirExprKind::False,
            ast::Expr::Integer { value, postfix, .. } => HirExprKind::Integer(*value, *postfix),
            ast::Expr::Float { value, postfix, .. } => HirExprKind::Float(*value, *postfix),
            ast::Expr::Posit { literal, .. } => HirExprKind::Posit(*literal),
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
            ast::Expr::Generator { body, span } => {
                return self.lower_generator(body, *span);
            }
            ast::Expr::Block {
                bindings, result, ..
            } => {
                self.push_scope();
                let bindings = bindings
                    .iter()
                    .map(|binding| {
                        let annotation = binding.annotation.as_ref().map(|ty| self.lower_type(ty));
                        let value = self.lower_expr(&binding.value);
                        let binding_id = self.define_current(
                            binding.name.clone(),
                            BindingKind::Local,
                            binding.span,
                        );
                        HirLocalBinding {
                            binding: binding_id,
                            annotation,
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
            ast::Expr::TypeForm { ty, .. } => HirExprKind::TypeForm(self.lower_type(ty)),
            ast::Expr::WitnessReflect {
                constraint,
                target,
                span,
            } => {
                let constraint = match self.resolve(constraint) {
                    Some(binding) => Some(binding),
                    None => {
                        self.diagnostics.push(HirDiagnostic {
                            kind: HirDiagnosticKind::UnknownConstraint {
                                name: constraint.clone(),
                            },
                            span: *span,
                        });
                        None
                    }
                };
                HirExprKind::WitnessReflect {
                    constraint,
                    target: self.lower_type(target),
                }
            }
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
