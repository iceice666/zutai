use std::collections::HashMap;

use crate::ir::{Row, TlcExpr, TlcExprId, TlcHandleClause, TlcType, TlcTypeId};

use super::{EffectElaborator, Kont};

impl<'module> EffectElaborator<'module> {
    pub(super) fn elaborate_handle(
        &mut self,
        id: TlcExprId,
        expr: TlcExprId,
        value: Option<TlcExprId>,
        ops: Vec<TlcHandleClause>,
    ) -> TlcExprId {
        self.elaborate_handle_with_parent(id, expr, value, ops, &[])
    }

    pub(super) fn elaborate_handle_with_parent(
        &mut self,
        id: TlcExprId,
        expr: TlcExprId,
        value: Option<TlcExprId>,
        ops: Vec<TlcHandleClause>,
        parent_handlers: &[TlcHandleClause],
    ) -> TlcExprId {
        let result_ty = self.expr_ty(id);
        self.cps(
            expr,
            &ops,
            parent_handlers,
            result_ty,
            Box::new(move |this, value_id| {
                if let Some(value_clause) = value {
                    let value_clause = this.elaborate_expr(value_clause);
                    this.alloc_like(id, TlcExpr::App(value_clause, value_id), result_ty)
                } else {
                    value_id
                }
            }),
        )
    }

    pub(super) fn cps(
        &mut self,
        id: TlcExprId,
        current_handlers: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
        result_ty: TlcTypeId,
        k: Kont<'_>,
    ) -> TlcExprId {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Perform { op, arg } => self.cps(
                arg,
                current_handlers,
                parent_handlers,
                result_ty,
                Box::new(move |this, arg_id| {
                    this.apply_handler_or_forward(
                        id,
                        op,
                        arg_id,
                        current_handlers,
                        parent_handlers,
                        result_ty,
                        k,
                    )
                }),
            ),
            TlcExpr::Sequence(items) => {
                self.cps_sequence(id, items, current_handlers, parent_handlers, result_ty, k)
            }
            TlcExpr::Handle { expr, value, ops } => {
                let mut enclosing = parent_handlers.to_vec();
                enclosing.extend_from_slice(current_handlers);
                if self.can_elaborate_handle_with_parent(expr, &ops, &enclosing) {
                    let handled =
                        self.elaborate_handle_with_parent(id, expr, value, ops, &enclosing);
                    k(self, handled)
                } else {
                    let direct = self.elaborate_expr(id);
                    k(self, direct)
                }
            }
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => self.cps(
                value,
                current_handlers,
                parent_handlers,
                result_ty,
                Box::new(move |this, value_id| {
                    let body_id = this.cps(body, current_handlers, parent_handlers, result_ty, k);
                    this.alloc_like(
                        id,
                        TlcExpr::Let {
                            binding,
                            ty,
                            value: value_id,
                            body: body_id,
                        },
                        result_ty,
                    )
                }),
            ),
            TlcExpr::App(func, arg) => self.cps(
                func,
                current_handlers,
                parent_handlers,
                result_ty,
                Box::new(move |this, func_id| {
                    this.cps(
                        arg,
                        current_handlers,
                        parent_handlers,
                        result_ty,
                        Box::new(move |this, arg_id| {
                            let app = this.alloc_like(
                                id,
                                TlcExpr::App(func_id, arg_id),
                                this.expr_ty(id),
                            );
                            k(this, app)
                        }),
                    )
                }),
            ),
            TlcExpr::Builtin(op, lhs, rhs) => self.cps(
                lhs,
                current_handlers,
                parent_handlers,
                result_ty,
                Box::new(move |this, lhs_id| {
                    this.cps(
                        rhs,
                        current_handlers,
                        parent_handlers,
                        result_ty,
                        Box::new(move |this, rhs_id| {
                            let builtin = this.alloc_like(
                                id,
                                TlcExpr::Builtin(op, lhs_id, rhs_id),
                                this.expr_ty(id),
                            );
                            k(this, builtin)
                        }),
                    )
                }),
            ),
            _ => {
                let direct = self.elaborate_expr(id);
                k(self, direct)
            }
        }
    }

    pub(super) fn cps_sequence(
        &mut self,
        id: TlcExprId,
        items: Vec<TlcExprId>,
        current_handlers: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
        result_ty: TlcTypeId,
        k: Kont<'_>,
    ) -> TlcExprId {
        let mut iter = items.into_iter();
        let Some(first) = iter.next() else {
            let nothing = self.alloc_like(
                id,
                TlcExpr::Lit(crate::ir::Literal::Nothing),
                self.expr_ty(id),
            );
            return k(self, nothing);
        };
        let rest: Vec<_> = iter.collect();
        if rest.is_empty() {
            return self.cps(first, current_handlers, parent_handlers, result_ty, k);
        }
        self.cps(
            first,
            current_handlers,
            parent_handlers,
            result_ty,
            Box::new(move |this, _| {
                this.cps_sequence(id, rest, current_handlers, parent_handlers, result_ty, k)
            }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_handler_or_forward(
        &mut self,
        perform_id: TlcExprId,
        op: String,
        arg: TlcExprId,
        current_handlers: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
        result_ty: TlcTypeId,
        k: Kont<'_>,
    ) -> TlcExprId {
        if current_handlers.iter().any(|clause| clause.op == op) {
            return self.apply_handler(
                perform_id,
                op,
                arg,
                current_handlers,
                parent_handlers,
                result_ty,
                k,
            );
        }
        if let Some(clause) = parent_handlers.iter().find(|clause| clause.op == op)
            && self.handler_clause_contains_resume(clause.body)
        {
            return self.apply_handler(perform_id, op, arg, parent_handlers, &[], result_ty, k);
        }
        let perform = self.alloc_like(
            perform_id,
            TlcExpr::Perform { op, arg },
            self.expr_ty(perform_id),
        );
        k(self, perform)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_handler(
        &mut self,
        perform_id: TlcExprId,
        op: String,
        arg: TlcExprId,
        handlers: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
        result_ty: TlcTypeId,
        k: Kont<'_>,
    ) -> TlcExprId {
        let Some(clause) = handlers.iter().find(|clause| clause.op == op) else {
            let perform = self.alloc_like(
                perform_id,
                TlcExpr::Perform { op, arg },
                self.expr_ty(perform_id),
            );
            return k(self, perform);
        };

        let resume_arg_ty = self.expr_ty(perform_id);
        let resume_binding = self.fresh_binding();
        let resume_var = self.alloc_like(perform_id, TlcExpr::Var(resume_binding), resume_arg_ty);
        let resume_body = k(self, resume_var);
        let resume_ty =
            self.module
                .type_arena
                .alloc(TlcType::Fun(resume_arg_ty, result_ty, Row::REmpty));
        let resume_lam = self.alloc_like(
            perform_id,
            TlcExpr::Lam(resume_binding, resume_arg_ty, resume_body),
            resume_ty,
        );

        let mut subst = HashMap::new();
        let handler = self.rewrite_handler_expr(
            clause.body,
            resume_lam,
            result_ty,
            &mut subst,
            parent_handlers,
        );
        self.alloc_like(perform_id, TlcExpr::App(handler, arg), result_ty)
    }
}
