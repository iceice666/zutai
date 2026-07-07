use crate::ir::{Row, TlcExpr, TlcExprId, TlcHandleClause, TlcTupleItem, TlcType};

use super::EffectElaborator;

impl<'module> EffectElaborator<'module> {
    pub(super) fn can_elaborate_handle(
        &self,
        expr: TlcExprId,
        finally: Option<TlcExprId>,
        ops: &[TlcHandleClause],
    ) -> bool {
        self.can_elaborate_handle_with_parent(expr, finally, ops, &[])
    }

    pub(super) fn can_elaborate_handle_with_parent(
        &self,
        expr: TlcExprId,
        finally: Option<TlcExprId>,
        ops: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
    ) -> bool {
        // A `finally` teardown is not handled by the lexical CPS pass. The native
        // compile path desugars it before re-running effect lowering, while the
        // shared lowering leaves it residual for the interpreter oracle.
        if finally.is_some() {
            return false;
        }
        let has_deferred_handled_perform = if self.preserve_deferred_performs {
            self.expr_has_deferred_handled_perform_thunk(expr, ops)
        } else {
            self.expr_has_deferred_handled_perform_arg(expr, ops)
        };
        if has_deferred_handled_perform {
            return false;
        }
        let mut saw_perform = false;
        ops.iter()
            .all(|clause| self.handler_clause_is_elaboratable(clause.body, parent_handlers))
            && self.expr_is_elaboratable_handle_body(expr, ops, parent_handlers, &mut saw_perform)
            && (saw_perform || self.expr_is_pure_for_first_cut(expr))
    }

    pub(super) fn handler_clause_is_elaboratable(
        &self,
        id: TlcExprId,
        parent_handlers: &[TlcHandleClause],
    ) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Perform { op, arg } => {
                parent_handlers.iter().any(|clause| clause.op == *op)
                    && self.handler_clause_is_elaboratable(*arg, parent_handlers)
            }
            TlcExpr::Handle {
                expr, finally, ops, ..
            } => self.can_elaborate_handle_with_parent(*expr, *finally, ops, parent_handlers),
            TlcExpr::Resume { value } => {
                self.handler_clause_is_elaboratable(*value, parent_handlers)
            }
            TlcExpr::App(func, arg) => {
                !self.expr_has_nonempty_fun_row(*func)
                    && self.handler_clause_is_elaboratable(*func, parent_handlers)
                    && self.handler_clause_is_elaboratable(*arg, parent_handlers)
            }
            TlcExpr::Builtin(_, lhs, rhs) => {
                self.handler_clause_is_elaboratable(*lhs, parent_handlers)
                    && self.handler_clause_is_elaboratable(*rhs, parent_handlers)
            }
            TlcExpr::ListAppend(left, right) => {
                self.handler_clause_is_elaboratable(*left, parent_handlers)
                    && self.handler_clause_is_elaboratable(*right, parent_handlers)
            }
            TlcExpr::Sequence(items) | TlcExpr::List(items) => items
                .iter()
                .all(|item| self.handler_clause_is_elaboratable(*item, parent_handlers)),
            TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
                self.handler_clause_is_elaboratable(*body, parent_handlers)
            }
            TlcExpr::Let { value, body, .. } => {
                self.handler_clause_is_elaboratable(*value, parent_handlers)
                    && self.handler_clause_is_elaboratable(*body, parent_handlers)
            }
            TlcExpr::Letrec { bindings, body } => {
                bindings.iter().all(|(_, _, value)| {
                    self.handler_clause_is_elaboratable(*value, parent_handlers)
                }) && self.handler_clause_is_elaboratable(*body, parent_handlers)
            }
            TlcExpr::Case(scrutinee, alts) => {
                self.handler_clause_is_elaboratable(*scrutinee, parent_handlers)
                    && alts.iter().all(|alt| {
                        alt.guard.is_none_or(|guard| {
                            self.handler_clause_is_elaboratable(guard, parent_handlers)
                        }) && self.handler_clause_is_elaboratable(alt.body, parent_handlers)
                    })
            }
            TlcExpr::Record(fields) => fields
                .iter()
                .all(|(_, value)| self.handler_clause_is_elaboratable(*value, parent_handlers)),
            TlcExpr::RecordUpdate { receiver, fields } => {
                self.handler_clause_is_elaboratable(*receiver, parent_handlers)
                    && fields.iter().all(|(_, value)| {
                        self.handler_clause_is_elaboratable(*value, parent_handlers)
                    })
            }
            TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
                self.handler_clause_is_elaboratable(*expr, parent_handlers)
            }
            TlcExpr::Tuple(items) => items.iter().all(|item| match item {
                TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                    self.handler_clause_is_elaboratable(*value, parent_handlers)
                }
            }),
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => true,
        }
    }

    fn expr_has_deferred_handled_perform_arg(
        &self,
        id: TlcExprId,
        ops: &[TlcHandleClause],
    ) -> bool {
        let mut seen = rustc_hash::FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::App(func, arg) => {
                    if self.expr_has_deferred_handled_perform_thunk(*arg, ops) {
                        return true;
                    }
                    stack.push(*func);
                    stack.push(*arg);
                }
                other => {
                    let mut children = Vec::new();
                    crate::monomorphize::push_child_exprs(other, &mut children);
                    stack.extend(children);
                }
            }
        }
        false
    }

    fn expr_has_deferred_handled_perform_thunk(
        &self,
        id: TlcExprId,
        ops: &[TlcHandleClause],
    ) -> bool {
        let mut seen = rustc_hash::FxHashSet::default();
        let mut stack = vec![(id, false)];
        while let Some((cur, under_lam)) = stack.pop() {
            if !seen.insert((cur, under_lam)) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Perform { op, .. }
                    if under_lam
                        && ops.iter().any(|clause| clause.op == *op)
                        && super::deferred_perform_needs_reifier(op) =>
                {
                    return true;
                }
                TlcExpr::Lam(_, _, body) => {
                    stack.push((*body, true));
                }
                TlcExpr::Handle {
                    expr,
                    value,
                    finally,
                    ..
                } => {
                    stack.push((*expr, under_lam));
                    if let Some(value) = value {
                        stack.push((*value, under_lam));
                    }
                    if let Some(finally) = finally {
                        stack.push((*finally, under_lam));
                    }
                }
                other => {
                    let mut children = Vec::new();
                    crate::monomorphize::push_child_exprs(other, &mut children);
                    stack.extend(children.into_iter().map(|child| (child, under_lam)));
                }
            }
        }
        false
    }

    pub(super) fn handler_clause_contains_resume(&self, id: TlcExprId) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Resume { .. } => true,
            TlcExpr::Sequence(items) | TlcExpr::List(items) => items
                .iter()
                .any(|item| self.handler_clause_contains_resume(*item)),
            TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
                self.handler_clause_contains_resume(*body)
            }
            TlcExpr::App(func, arg)
            | TlcExpr::Builtin(_, func, arg)
            | TlcExpr::ListAppend(func, arg) => {
                self.handler_clause_contains_resume(*func)
                    || self.handler_clause_contains_resume(*arg)
            }
            TlcExpr::Let { value, body, .. } => {
                self.handler_clause_contains_resume(*value)
                    || self.handler_clause_contains_resume(*body)
            }
            TlcExpr::Letrec { bindings, body } => {
                bindings
                    .iter()
                    .any(|(_, _, value)| self.handler_clause_contains_resume(*value))
                    || self.handler_clause_contains_resume(*body)
            }
            TlcExpr::Case(scrutinee, alts) => {
                self.handler_clause_contains_resume(*scrutinee)
                    || alts.iter().any(|alt| {
                        alt.guard
                            .is_some_and(|guard| self.handler_clause_contains_resume(guard))
                            || self.handler_clause_contains_resume(alt.body)
                    })
            }
            TlcExpr::Record(fields) => fields
                .iter()
                .any(|(_, value)| self.handler_clause_contains_resume(*value)),
            TlcExpr::RecordUpdate { receiver, fields } => {
                self.handler_clause_contains_resume(*receiver)
                    || fields
                        .iter()
                        .any(|(_, value)| self.handler_clause_contains_resume(*value))
            }
            TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
                self.handler_clause_contains_resume(*expr)
            }
            TlcExpr::Tuple(items) => items.iter().any(|item| match item {
                TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                    self.handler_clause_contains_resume(*value)
                }
            }),
            TlcExpr::Perform { .. }
            | TlcExpr::Handle { .. }
            | TlcExpr::Var(_)
            | TlcExpr::Lit(_)
            | TlcExpr::Import(_) => false,
        }
    }

    pub(super) fn expr_is_elaboratable_handle_body(
        &self,
        id: TlcExprId,
        ops: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
        saw_perform: &mut bool,
    ) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Perform { op, arg } => {
                *saw_perform = true;
                let handled_here = ops.iter().any(|clause| clause.op == *op);
                let forwarded_to_resuming_parent = parent_handlers
                    .iter()
                    .find(|clause| clause.op == *op)
                    .is_some_and(|clause| self.handler_clause_contains_resume(clause.body));
                (handled_here || forwarded_to_resuming_parent)
                    && self.expr_is_elaboratable_handle_body(
                        *arg,
                        ops,
                        parent_handlers,
                        saw_perform,
                    )
            }
            TlcExpr::Handle {
                expr,
                value: _,
                finally,
                ops: nested_ops,
            } => {
                let mut enclosing = parent_handlers.to_vec();
                enclosing.extend_from_slice(ops);
                self.can_elaborate_handle_with_parent(*expr, *finally, nested_ops, &enclosing)
            }
            TlcExpr::Resume { .. } => false,
            TlcExpr::App(func, arg) => {
                !self.expr_has_nonempty_fun_row(*func)
                    && self.expr_is_elaboratable_handle_body(
                        *func,
                        ops,
                        parent_handlers,
                        saw_perform,
                    )
                    && self.expr_is_elaboratable_handle_body(
                        *arg,
                        ops,
                        parent_handlers,
                        saw_perform,
                    )
            }
            TlcExpr::Builtin(_, lhs, rhs) => {
                self.expr_is_elaboratable_handle_body(*lhs, ops, parent_handlers, saw_perform)
                    && self.expr_is_elaboratable_handle_body(
                        *rhs,
                        ops,
                        parent_handlers,
                        saw_perform,
                    )
            }
            TlcExpr::ListAppend(left, right) => {
                self.expr_is_elaboratable_handle_body(*left, ops, parent_handlers, saw_perform)
                    && self.expr_is_elaboratable_handle_body(
                        *right,
                        ops,
                        parent_handlers,
                        saw_perform,
                    )
            }
            TlcExpr::Sequence(items) | TlcExpr::List(items) => items.iter().all(|item| {
                self.expr_is_elaboratable_handle_body(*item, ops, parent_handlers, saw_perform)
            }),
            TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
                self.expr_is_elaboratable_handle_body(*body, ops, parent_handlers, saw_perform)
            }
            TlcExpr::Let { value, body, .. } => {
                self.expr_is_elaboratable_handle_body(*value, ops, parent_handlers, saw_perform)
                    && self.expr_is_elaboratable_handle_body(
                        *body,
                        ops,
                        parent_handlers,
                        saw_perform,
                    )
            }
            TlcExpr::Letrec { bindings, body } => {
                bindings.iter().all(|(_, _, value)| {
                    self.expr_is_elaboratable_handle_body(*value, ops, parent_handlers, saw_perform)
                }) && self.expr_is_elaboratable_handle_body(
                    *body,
                    ops,
                    parent_handlers,
                    saw_perform,
                )
            }
            TlcExpr::Case(scrutinee, alts) => {
                self.expr_is_elaboratable_handle_body(*scrutinee, ops, parent_handlers, saw_perform)
                    && alts.iter().all(|alt| {
                        alt.guard.is_none_or(|guard| {
                            self.expr_is_elaboratable_handle_body(
                                guard,
                                ops,
                                parent_handlers,
                                saw_perform,
                            )
                        }) && self.expr_is_elaboratable_handle_body(
                            alt.body,
                            ops,
                            parent_handlers,
                            saw_perform,
                        )
                    })
            }
            TlcExpr::Record(fields) => fields.iter().all(|(_, value)| {
                self.expr_is_elaboratable_handle_body(*value, ops, parent_handlers, saw_perform)
            }),
            TlcExpr::RecordUpdate { receiver, fields } => {
                self.expr_is_elaboratable_handle_body(*receiver, ops, parent_handlers, saw_perform)
                    && fields.iter().all(|(_, value)| {
                        self.expr_is_elaboratable_handle_body(
                            *value,
                            ops,
                            parent_handlers,
                            saw_perform,
                        )
                    })
            }
            TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
                self.expr_is_elaboratable_handle_body(*expr, ops, parent_handlers, saw_perform)
            }
            TlcExpr::Tuple(items) => items.iter().all(|item| match item {
                TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                    self.expr_is_elaboratable_handle_body(*value, ops, parent_handlers, saw_perform)
                }
            }),
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => true,
        }
    }

    pub(super) fn expr_is_pure_for_first_cut(&self, id: TlcExprId) -> bool {
        !self.expr_has_nonempty_fun_row(id)
    }

    pub(super) fn expr_is_direct_sequence_safe(&self, id: TlcExprId) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. } => false,
            TlcExpr::App(func, arg) => {
                !self.expr_has_nonempty_fun_row(*func)
                    && self.expr_is_direct_sequence_safe(*func)
                    && self.expr_is_direct_sequence_safe(*arg)
            }
            TlcExpr::Sequence(items) | TlcExpr::List(items) => items
                .iter()
                .all(|item| self.expr_is_direct_sequence_safe(*item)),
            TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
                self.expr_is_direct_sequence_safe(*body)
            }
            TlcExpr::Let { value, body, .. } => {
                self.expr_is_direct_sequence_safe(*value)
                    && self.expr_is_direct_sequence_safe(*body)
            }
            TlcExpr::Letrec { bindings, body } => {
                bindings
                    .iter()
                    .all(|(_, _, value)| self.expr_is_direct_sequence_safe(*value))
                    && self.expr_is_direct_sequence_safe(*body)
            }
            TlcExpr::Case(scrutinee, alts) => {
                self.expr_is_direct_sequence_safe(*scrutinee)
                    && alts.iter().all(|alt| {
                        alt.guard
                            .is_none_or(|guard| self.expr_is_direct_sequence_safe(guard))
                            && self.expr_is_direct_sequence_safe(alt.body)
                    })
            }
            TlcExpr::Record(fields) => fields
                .iter()
                .all(|(_, value)| self.expr_is_direct_sequence_safe(*value)),
            TlcExpr::RecordUpdate { receiver, fields } => {
                self.expr_is_direct_sequence_safe(*receiver)
                    && fields
                        .iter()
                        .all(|(_, value)| self.expr_is_direct_sequence_safe(*value))
            }
            TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
                self.expr_is_direct_sequence_safe(*expr)
            }
            TlcExpr::Tuple(items) => items.iter().all(|item| match item {
                TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                    self.expr_is_direct_sequence_safe(*value)
                }
            }),
            TlcExpr::Builtin(_, lhs, rhs) => {
                self.expr_is_direct_sequence_safe(*lhs) && self.expr_is_direct_sequence_safe(*rhs)
            }
            TlcExpr::ListAppend(left, right) => {
                self.expr_is_direct_sequence_safe(*left)
                    && self.expr_is_direct_sequence_safe(*right)
            }
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => true,
        }
    }

    pub(super) fn expr_has_nonempty_fun_row(&self, id: TlcExprId) -> bool {
        self.module.expr_types.get(&id).is_some_and(|ty| {
            matches!(
                &self.module.type_arena[*ty],
                TlcType::Fun(_, _, row) if !matches!(row, Row::REmpty)
            )
        })
    }
}
