use std::collections::{HashMap, HashSet};

use zutai_hir::BindingId;

use crate::ir::{
    Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcHandleClause, TlcModule, TlcTupleItem, TlcType,
    TlcTypeId,
};

impl TlcModule {
    /// Elaborate handled TLC effect markers into pure TLC terms.
    ///
    /// This is intentionally conservative for the first CPS cut: matched handler
    /// operations are lowered to ordinary handler-function calls with an explicit
    /// resume lambda; unmatched operations remain as `Perform` so the existing
    /// Dataflow gate rejects them.
    pub fn elaborate_effects(&mut self) {
        let mut elaborator = EffectElaborator::new(self);
        let decls = elaborator.module.decls.clone();
        for decl_id in decls {
            let body = match elaborator.module.decl_arena[decl_id] {
                TlcDecl::Value { body, .. } => body,
                TlcDecl::TypeAlias { .. } => continue,
            };
            let rewritten = elaborator.elaborate_expr(body);
            if let TlcDecl::Value { body, .. } = &mut elaborator.module.decl_arena[decl_id] {
                *body = rewritten;
            }
        }
        if let Some(final_expr) = elaborator.module.final_expr {
            elaborator.module.final_expr = Some(elaborator.elaborate_expr(final_expr));
        }
    }
}
type Kont<'kont> = Box<dyn FnOnce(&mut EffectElaborator<'_>, TlcExprId) -> TlcExprId + 'kont>;

struct EffectElaborator<'module> {
    module: &'module mut TlcModule,
    used_bindings: HashSet<BindingId>,
    next_fresh: u32,
}

impl<'module> EffectElaborator<'module> {
    fn new(module: &'module mut TlcModule) -> Self {
        let mut used_bindings = HashSet::new();
        for (_, decl) in module.decl_arena.iter() {
            let binding = match decl {
                TlcDecl::Value { binding, .. } | TlcDecl::TypeAlias { binding, .. } => binding,
            };
            used_bindings.insert(*binding);
        }
        for (_, expr) in module.expr_arena.iter() {
            collect_expr_bindings(expr, &mut used_bindings);
        }
        Self {
            module,
            used_bindings,
            next_fresh: u32::MAX,
        }
    }

    fn elaborate_expr(&mut self, id: TlcExprId) -> TlcExprId {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Handle { expr, value, ops } => {
                if self.can_elaborate_handle(expr, &ops) {
                    self.elaborate_handle(id, expr, value, ops)
                } else {
                    id
                }
            }
            TlcExpr::Sequence(items) => {
                if items
                    .iter()
                    .all(|item| self.expr_is_direct_sequence_safe(*item))
                {
                    self.elaborate_sequence_direct(id, items)
                } else {
                    id
                }
            }
            TlcExpr::Perform { op, arg } => {
                let arg = self.elaborate_expr(arg);
                self.alloc_like(id, TlcExpr::Perform { op, arg }, self.expr_ty(id))
            }
            TlcExpr::Resume { value } => {
                let value = self.elaborate_expr(value);
                self.alloc_like(id, TlcExpr::Resume { value }, self.expr_ty(id))
            }
            TlcExpr::Lam(binding, ty, body) => {
                let body = self.elaborate_expr(body);
                self.alloc_like(id, TlcExpr::Lam(binding, ty, body), self.expr_ty(id))
            }
            TlcExpr::App(func, arg) => {
                let func = self.elaborate_expr(func);
                let arg = self.elaborate_expr(arg);
                self.alloc_like(id, TlcExpr::App(func, arg), self.expr_ty(id))
            }
            TlcExpr::TyLam(var, kind, body) => {
                let body = self.elaborate_expr(body);
                self.alloc_like(id, TlcExpr::TyLam(var, kind, body), self.expr_ty(id))
            }
            TlcExpr::TyApp(expr, ty) => {
                let expr = self.elaborate_expr(expr);
                self.alloc_like(id, TlcExpr::TyApp(expr, ty), self.expr_ty(id))
            }
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let value = self.elaborate_expr(value);
                let body = self.elaborate_expr(body);
                self.alloc_like(
                    id,
                    TlcExpr::Let {
                        binding,
                        ty,
                        value,
                        body,
                    },
                    self.expr_ty(id),
                )
            }
            TlcExpr::Letrec { bindings, body } => {
                let bindings = bindings
                    .into_iter()
                    .map(|(binding, ty, value)| (binding, ty, self.elaborate_expr(value)))
                    .collect();
                let body = self.elaborate_expr(body);
                self.alloc_like(id, TlcExpr::Letrec { bindings, body }, self.expr_ty(id))
            }
            TlcExpr::Case(scrutinee, alts) => {
                let scrutinee = self.elaborate_expr(scrutinee);
                let alts = alts
                    .into_iter()
                    .map(|alt| TlcAlt {
                        pat: alt.pat,
                        guard: alt.guard.map(|guard| self.elaborate_expr(guard)),
                        body: self.elaborate_expr(alt.body),
                    })
                    .collect();
                self.alloc_like(id, TlcExpr::Case(scrutinee, alts), self.expr_ty(id))
            }
            TlcExpr::Record(fields) => {
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| (name, self.elaborate_expr(value)))
                    .collect();
                self.alloc_like(id, TlcExpr::Record(fields), self.expr_ty(id))
            }
            TlcExpr::RecordUpdate { receiver, fields } => {
                let receiver = self.elaborate_expr(receiver);
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| (name, self.elaborate_expr(value)))
                    .collect();
                self.alloc_like(
                    id,
                    TlcExpr::RecordUpdate { receiver, fields },
                    self.expr_ty(id),
                )
            }
            TlcExpr::GetField(expr, field) => {
                let expr = self.elaborate_expr(expr);
                self.alloc_like(id, TlcExpr::GetField(expr, field), self.expr_ty(id))
            }
            TlcExpr::Tuple(items) => {
                let items = items
                    .into_iter()
                    .map(|item| match item {
                        TlcTupleItem::Named { name, value } => TlcTupleItem::Named {
                            name,
                            value: self.elaborate_expr(value),
                        },
                        TlcTupleItem::Positional(value) => {
                            TlcTupleItem::Positional(self.elaborate_expr(value))
                        }
                    })
                    .collect();
                self.alloc_like(id, TlcExpr::Tuple(items), self.expr_ty(id))
            }
            TlcExpr::List(items) => {
                let items = items
                    .into_iter()
                    .map(|item| self.elaborate_expr(item))
                    .collect();
                self.alloc_like(id, TlcExpr::List(items), self.expr_ty(id))
            }
            TlcExpr::Builtin(op, lhs, rhs) => {
                let lhs = self.elaborate_expr(lhs);
                let rhs = self.elaborate_expr(rhs);
                self.alloc_like(id, TlcExpr::Builtin(op, lhs, rhs), self.expr_ty(id))
            }
            TlcExpr::Variant(tag, payload) => {
                let payload = self.elaborate_expr(payload);
                self.alloc_like(id, TlcExpr::Variant(tag, payload), self.expr_ty(id))
            }
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => id,
        }
    }

    fn can_elaborate_handle(&self, expr: TlcExprId, ops: &[TlcHandleClause]) -> bool {
        self.can_elaborate_handle_with_parent(expr, ops, &[])
    }

    fn can_elaborate_handle_with_parent(
        &self,
        expr: TlcExprId,
        ops: &[TlcHandleClause],
        parent_handlers: &[TlcHandleClause],
    ) -> bool {
        let mut saw_perform = false;
        ops.iter()
            .all(|clause| self.handler_clause_is_elaboratable(clause.body, parent_handlers))
            && self.expr_is_elaboratable_handle_body(expr, ops, parent_handlers, &mut saw_perform)
            && (saw_perform || self.expr_is_pure_for_first_cut(expr))
    }

    fn handler_clause_is_elaboratable(
        &self,
        id: TlcExprId,
        parent_handlers: &[TlcHandleClause],
    ) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Perform { op, arg } => {
                parent_handlers.iter().any(|clause| clause.op == *op)
                    && self.handler_clause_is_elaboratable(*arg, parent_handlers)
            }
            TlcExpr::Handle { expr, ops, .. } => {
                self.can_elaborate_handle_with_parent(*expr, ops, parent_handlers)
            }
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

    fn handler_clause_contains_resume(&self, id: TlcExprId) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Resume { .. } => true,
            TlcExpr::Sequence(items) | TlcExpr::List(items) => items
                .iter()
                .any(|item| self.handler_clause_contains_resume(*item)),
            TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
                self.handler_clause_contains_resume(*body)
            }
            TlcExpr::App(func, arg) | TlcExpr::Builtin(_, func, arg) => {
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

    fn expr_is_elaboratable_handle_body(
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
                ops: nested_ops,
            } => {
                let mut enclosing = parent_handlers.to_vec();
                enclosing.extend_from_slice(ops);
                self.can_elaborate_handle_with_parent(*expr, nested_ops, &enclosing)
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

    fn expr_is_pure_for_first_cut(&self, id: TlcExprId) -> bool {
        !self.expr_has_nonempty_fun_row(id)
    }

    fn expr_is_direct_sequence_safe(&self, id: TlcExprId) -> bool {
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
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => true,
        }
    }

    fn expr_has_nonempty_fun_row(&self, id: TlcExprId) -> bool {
        self.module.expr_types.get(&id).is_some_and(|ty| {
            matches!(
                &self.module.type_arena[*ty],
                TlcType::Fun(_, _, row) if !matches!(row, Row::REmpty)
            )
        })
    }

    fn elaborate_handle(
        &mut self,
        id: TlcExprId,
        expr: TlcExprId,
        value: Option<TlcExprId>,
        ops: Vec<TlcHandleClause>,
    ) -> TlcExprId {
        self.elaborate_handle_with_parent(id, expr, value, ops, &[])
    }

    fn elaborate_handle_with_parent(
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

    fn cps(
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

    fn cps_sequence(
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
    fn apply_handler_or_forward(
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
    fn apply_handler(
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

    fn rewrite_handler_cps(
        &mut self,
        id: TlcExprId,
        resume_lam: TlcExprId,
        result_ty: TlcTypeId,
        subst: &mut HashMap<BindingId, BindingId>,
        parent_handlers: &[TlcHandleClause],
        k: Kont<'_>,
    ) -> TlcExprId {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Perform { op, arg } => {
                let arg =
                    self.rewrite_handler_expr(arg, resume_lam, result_ty, subst, parent_handlers);
                self.apply_handler_or_forward(id, op, arg, parent_handlers, &[], result_ty, k)
            }
            TlcExpr::Resume { value } => {
                let value =
                    self.rewrite_handler_expr(value, resume_lam, result_ty, subst, parent_handlers);
                let resumed = self.alloc_like(id, TlcExpr::App(resume_lam, value), result_ty);
                k(self, resumed)
            }
            TlcExpr::Sequence(items) => self.rewrite_handler_cps_sequence(
                id,
                items,
                resume_lam,
                result_ty,
                subst,
                parent_handlers,
                k,
            ),
            TlcExpr::Handle { expr, value, ops } => {
                let handled = if self.can_elaborate_handle_with_parent(expr, &ops, parent_handlers)
                {
                    self.elaborate_handle_with_parent(id, expr, value, ops, parent_handlers)
                } else {
                    self.elaborate_expr(id)
                };
                k(self, handled)
            }
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let mut body_subst = subst.clone();
                self.rewrite_handler_cps(
                    value,
                    resume_lam,
                    result_ty,
                    subst,
                    parent_handlers,
                    Box::new(move |this, value_id| {
                        let fresh = this.fresh_binding();
                        let old = body_subst.insert(binding, fresh);
                        let body_id = this.rewrite_handler_cps(
                            body,
                            resume_lam,
                            result_ty,
                            &mut body_subst,
                            parent_handlers,
                            k,
                        );
                        restore_subst(&mut body_subst, binding, old);
                        this.alloc_like(
                            id,
                            TlcExpr::Let {
                                binding: fresh,
                                ty,
                                value: value_id,
                                body: body_id,
                            },
                            result_ty,
                        )
                    }),
                )
            }
            TlcExpr::App(func, arg) => {
                let mut arg_subst = subst.clone();
                self.rewrite_handler_cps(
                    func,
                    resume_lam,
                    result_ty,
                    subst,
                    parent_handlers,
                    Box::new(move |this, func_id| {
                        this.rewrite_handler_cps(
                            arg,
                            resume_lam,
                            result_ty,
                            &mut arg_subst,
                            parent_handlers,
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
                )
            }
            TlcExpr::Builtin(op, lhs, rhs) => {
                let mut rhs_subst = subst.clone();
                self.rewrite_handler_cps(
                    lhs,
                    resume_lam,
                    result_ty,
                    subst,
                    parent_handlers,
                    Box::new(move |this, lhs_id| {
                        this.rewrite_handler_cps(
                            rhs,
                            resume_lam,
                            result_ty,
                            &mut rhs_subst,
                            parent_handlers,
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
                )
            }
            _ => {
                let direct =
                    self.rewrite_handler_expr(id, resume_lam, result_ty, subst, parent_handlers);
                k(self, direct)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn rewrite_handler_cps_sequence(
        &mut self,
        id: TlcExprId,
        items: Vec<TlcExprId>,
        resume_lam: TlcExprId,
        result_ty: TlcTypeId,
        subst: &mut HashMap<BindingId, BindingId>,
        parent_handlers: &[TlcHandleClause],
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
            return self.rewrite_handler_cps(
                first,
                resume_lam,
                result_ty,
                subst,
                parent_handlers,
                k,
            );
        }
        let mut rest_subst = subst.clone();
        self.rewrite_handler_cps(
            first,
            resume_lam,
            result_ty,
            subst,
            parent_handlers,
            Box::new(move |this, _| {
                this.rewrite_handler_cps_sequence(
                    id,
                    rest,
                    resume_lam,
                    result_ty,
                    &mut rest_subst,
                    parent_handlers,
                    k,
                )
            }),
        )
    }

    fn handler_expr_contains_control(&self, id: TlcExprId) -> bool {
        match &self.module.expr_arena[id] {
            TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. } => true,
            TlcExpr::Sequence(items) | TlcExpr::List(items) => items
                .iter()
                .any(|item| self.handler_expr_contains_control(*item)),
            TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
                self.handler_expr_contains_control(*body)
            }
            TlcExpr::App(func, arg) | TlcExpr::Builtin(_, func, arg) => {
                self.handler_expr_contains_control(*func)
                    || self.handler_expr_contains_control(*arg)
            }
            TlcExpr::Let { value, body, .. } => {
                self.handler_expr_contains_control(*value)
                    || self.handler_expr_contains_control(*body)
            }
            TlcExpr::Letrec { bindings, body } => {
                bindings
                    .iter()
                    .any(|(_, _, value)| self.handler_expr_contains_control(*value))
                    || self.handler_expr_contains_control(*body)
            }
            TlcExpr::Case(scrutinee, alts) => {
                self.handler_expr_contains_control(*scrutinee)
                    || alts.iter().any(|alt| {
                        alt.guard
                            .is_some_and(|guard| self.handler_expr_contains_control(guard))
                            || self.handler_expr_contains_control(alt.body)
                    })
            }
            TlcExpr::Record(fields) => fields
                .iter()
                .any(|(_, value)| self.handler_expr_contains_control(*value)),
            TlcExpr::RecordUpdate { receiver, fields } => {
                self.handler_expr_contains_control(*receiver)
                    || fields
                        .iter()
                        .any(|(_, value)| self.handler_expr_contains_control(*value))
            }
            TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
                self.handler_expr_contains_control(*expr)
            }
            TlcExpr::Tuple(items) => items.iter().any(|item| match item {
                TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                    self.handler_expr_contains_control(*value)
                }
            }),
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => false,
        }
    }

    fn rewrite_handler_expr(
        &mut self,
        id: TlcExprId,
        resume_lam: TlcExprId,
        result_ty: TlcTypeId,
        subst: &mut HashMap<BindingId, BindingId>,
        parent_handlers: &[TlcHandleClause],
    ) -> TlcExprId {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Var(binding) => {
                let binding = subst.get(&binding).copied().unwrap_or(binding);
                self.alloc_like(id, TlcExpr::Var(binding), self.expr_ty(id))
            }
            TlcExpr::Resume { value } => {
                let value =
                    self.rewrite_handler_expr(value, resume_lam, result_ty, subst, parent_handlers);
                self.alloc_like(id, TlcExpr::App(resume_lam, value), result_ty)
            }
            TlcExpr::Sequence(items) => self.rewrite_handler_sequence(
                id,
                items,
                resume_lam,
                result_ty,
                subst,
                parent_handlers,
            ),
            TlcExpr::Lam(binding, ty, body) => {
                let fresh = self.fresh_binding();
                let old = subst.insert(binding, fresh);
                let body = if self.handler_expr_contains_control(body) {
                    self.rewrite_handler_cps(
                        body,
                        resume_lam,
                        result_ty,
                        subst,
                        parent_handlers,
                        Box::new(|_, value| value),
                    )
                } else {
                    self.rewrite_handler_expr(body, resume_lam, result_ty, subst, parent_handlers)
                };
                restore_subst(subst, binding, old);
                self.alloc_like(id, TlcExpr::Lam(fresh, ty, body), self.expr_ty(id))
            }
            TlcExpr::App(func, arg) => {
                let func =
                    self.rewrite_handler_expr(func, resume_lam, result_ty, subst, parent_handlers);
                let arg =
                    self.rewrite_handler_expr(arg, resume_lam, result_ty, subst, parent_handlers);
                self.alloc_like(id, TlcExpr::App(func, arg), self.expr_ty(id))
            }
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let value =
                    self.rewrite_handler_expr(value, resume_lam, result_ty, subst, parent_handlers);
                let fresh = self.fresh_binding();
                let old = subst.insert(binding, fresh);
                let body =
                    self.rewrite_handler_expr(body, resume_lam, result_ty, subst, parent_handlers);
                restore_subst(subst, binding, old);
                self.alloc_like(
                    id,
                    TlcExpr::Let {
                        binding: fresh,
                        ty,
                        value,
                        body,
                    },
                    self.expr_ty(id),
                )
            }
            TlcExpr::Builtin(op, lhs, rhs) => {
                let lhs =
                    self.rewrite_handler_expr(lhs, resume_lam, result_ty, subst, parent_handlers);
                let rhs =
                    self.rewrite_handler_expr(rhs, resume_lam, result_ty, subst, parent_handlers);
                self.alloc_like(id, TlcExpr::Builtin(op, lhs, rhs), self.expr_ty(id))
            }
            TlcExpr::TyLam(var, kind, body) => {
                let body =
                    self.rewrite_handler_expr(body, resume_lam, result_ty, subst, parent_handlers);
                self.alloc_like(id, TlcExpr::TyLam(var, kind, body), self.expr_ty(id))
            }
            TlcExpr::TyApp(expr, ty) => {
                let expr =
                    self.rewrite_handler_expr(expr, resume_lam, result_ty, subst, parent_handlers);
                self.alloc_like(id, TlcExpr::TyApp(expr, ty), self.expr_ty(id))
            }
            TlcExpr::Letrec { bindings, body } => {
                let mut old_bindings = Vec::with_capacity(bindings.len());
                let mut fresh_bindings = Vec::with_capacity(bindings.len());
                for (binding, ty, value) in &bindings {
                    let fresh = self.fresh_binding();
                    old_bindings.push((*binding, subst.insert(*binding, fresh)));
                    fresh_bindings.push((fresh, *ty, *value));
                }
                let bindings = fresh_bindings
                    .into_iter()
                    .map(|(binding, ty, value)| {
                        (
                            binding,
                            ty,
                            self.rewrite_handler_expr(
                                value,
                                resume_lam,
                                result_ty,
                                subst,
                                parent_handlers,
                            ),
                        )
                    })
                    .collect();
                let body =
                    self.rewrite_handler_expr(body, resume_lam, result_ty, subst, parent_handlers);
                for (binding, old) in old_bindings.into_iter().rev() {
                    restore_subst(subst, binding, old);
                }
                self.alloc_like(id, TlcExpr::Letrec { bindings, body }, self.expr_ty(id))
            }
            TlcExpr::Case(scrutinee, alts) => {
                let scrutinee = self.rewrite_handler_expr(
                    scrutinee,
                    resume_lam,
                    result_ty,
                    subst,
                    parent_handlers,
                );
                let alts = alts
                    .into_iter()
                    .map(|alt| {
                        let mut old_bindings = Vec::new();
                        let pat = self.freshen_pat(alt.pat, subst, &mut old_bindings);
                        let guard = alt.guard.map(|guard| {
                            self.rewrite_handler_expr(
                                guard,
                                resume_lam,
                                result_ty,
                                subst,
                                parent_handlers,
                            )
                        });
                        let body = self.rewrite_handler_expr(
                            alt.body,
                            resume_lam,
                            result_ty,
                            subst,
                            parent_handlers,
                        );
                        for (binding, old) in old_bindings.into_iter().rev() {
                            restore_subst(subst, binding, old);
                        }
                        TlcAlt { pat, guard, body }
                    })
                    .collect();
                self.alloc_like(id, TlcExpr::Case(scrutinee, alts), self.expr_ty(id))
            }
            TlcExpr::Record(fields) => {
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| {
                        (
                            name,
                            self.rewrite_handler_expr(
                                value,
                                resume_lam,
                                result_ty,
                                subst,
                                parent_handlers,
                            ),
                        )
                    })
                    .collect();
                self.alloc_like(id, TlcExpr::Record(fields), self.expr_ty(id))
            }
            TlcExpr::RecordUpdate { receiver, fields } => {
                let receiver = self.rewrite_handler_expr(
                    receiver,
                    resume_lam,
                    result_ty,
                    subst,
                    parent_handlers,
                );
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| {
                        (
                            name,
                            self.rewrite_handler_expr(
                                value,
                                resume_lam,
                                result_ty,
                                subst,
                                parent_handlers,
                            ),
                        )
                    })
                    .collect();
                self.alloc_like(
                    id,
                    TlcExpr::RecordUpdate { receiver, fields },
                    self.expr_ty(id),
                )
            }
            TlcExpr::GetField(expr, field) => {
                let expr =
                    self.rewrite_handler_expr(expr, resume_lam, result_ty, subst, parent_handlers);
                self.alloc_like(id, TlcExpr::GetField(expr, field), self.expr_ty(id))
            }
            TlcExpr::Tuple(items) => {
                let items = items
                    .into_iter()
                    .map(|item| match item {
                        TlcTupleItem::Named { name, value } => TlcTupleItem::Named {
                            name,
                            value: self.rewrite_handler_expr(
                                value,
                                resume_lam,
                                result_ty,
                                subst,
                                parent_handlers,
                            ),
                        },
                        TlcTupleItem::Positional(value) => {
                            TlcTupleItem::Positional(self.rewrite_handler_expr(
                                value,
                                resume_lam,
                                result_ty,
                                subst,
                                parent_handlers,
                            ))
                        }
                    })
                    .collect();
                self.alloc_like(id, TlcExpr::Tuple(items), self.expr_ty(id))
            }
            TlcExpr::List(items) => {
                let items = items
                    .into_iter()
                    .map(|item| {
                        self.rewrite_handler_expr(
                            item,
                            resume_lam,
                            result_ty,
                            subst,
                            parent_handlers,
                        )
                    })
                    .collect();
                self.alloc_like(id, TlcExpr::List(items), self.expr_ty(id))
            }
            TlcExpr::Variant(tag, payload) => {
                let payload = self.rewrite_handler_expr(
                    payload,
                    resume_lam,
                    result_ty,
                    subst,
                    parent_handlers,
                );
                self.alloc_like(id, TlcExpr::Variant(tag, payload), self.expr_ty(id))
            }
            TlcExpr::Perform { .. } | TlcExpr::Handle { .. } => self.rewrite_handler_cps(
                id,
                resume_lam,
                result_ty,
                subst,
                parent_handlers,
                Box::new(|_, value| value),
            ),
            TlcExpr::Lit(_) | TlcExpr::Import(_) => id,
        }
    }

    fn freshen_pat(
        &mut self,
        pat: crate::ir::TlcPat,
        subst: &mut HashMap<BindingId, BindingId>,
        old_bindings: &mut Vec<(BindingId, Option<BindingId>)>,
    ) -> crate::ir::TlcPat {
        match pat {
            crate::ir::TlcPat::Bind(binding) => {
                let fresh = self.fresh_binding();
                old_bindings.push((binding, subst.insert(binding, fresh)));
                crate::ir::TlcPat::Bind(fresh)
            }
            crate::ir::TlcPat::Tuple(items) => crate::ir::TlcPat::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        crate::ir::TlcPatItem::Named { name, pat } => {
                            crate::ir::TlcPatItem::Named {
                                name,
                                pat: self.freshen_pat(pat, subst, old_bindings),
                            }
                        }
                        crate::ir::TlcPatItem::Positional(pat) => {
                            crate::ir::TlcPatItem::Positional(self.freshen_pat(
                                pat,
                                subst,
                                old_bindings,
                            ))
                        }
                    })
                    .collect(),
            ),
            crate::ir::TlcPat::Record(fields) => crate::ir::TlcPat::Record(
                fields
                    .into_iter()
                    .map(|(name, pat)| (name, self.freshen_pat(pat, subst, old_bindings)))
                    .collect(),
            ),
            crate::ir::TlcPat::Variant(tag, inner) => crate::ir::TlcPat::Variant(
                tag,
                Box::new(self.freshen_pat(*inner, subst, old_bindings)),
            ),
            crate::ir::TlcPat::Wildcard
            | crate::ir::TlcPat::Lit(_)
            | crate::ir::TlcPat::Atom(_) => pat,
        }
    }

    fn rewrite_handler_sequence(
        &mut self,
        id: TlcExprId,
        items: Vec<TlcExprId>,
        resume_lam: TlcExprId,
        result_ty: TlcTypeId,
        subst: &mut HashMap<BindingId, BindingId>,
        parent_handlers: &[TlcHandleClause],
    ) -> TlcExprId {
        if items
            .iter()
            .any(|item| self.handler_expr_contains_control(*item))
        {
            return self.rewrite_handler_cps_sequence(
                id,
                items,
                resume_lam,
                result_ty,
                subst,
                parent_handlers,
                Box::new(|_, value| value),
            );
        }
        let mut iter = items.into_iter().rev();
        let Some(last) = iter.next() else {
            return self.alloc_like(
                id,
                TlcExpr::Lit(crate::ir::Literal::Nothing),
                self.expr_ty(id),
            );
        };
        let mut acc =
            self.rewrite_handler_expr(last, resume_lam, result_ty, subst, parent_handlers);
        for item in iter {
            let value =
                self.rewrite_handler_expr(item, resume_lam, result_ty, subst, parent_handlers);
            let binding = self.fresh_binding();
            acc = self.alloc_like(
                id,
                TlcExpr::Let {
                    binding,
                    ty: self.expr_ty(item),
                    value,
                    body: acc,
                },
                self.expr_ty(id),
            );
        }
        acc
    }

    fn elaborate_sequence_direct(&mut self, id: TlcExprId, items: Vec<TlcExprId>) -> TlcExprId {
        let mut iter = items.into_iter().rev();
        let Some(last) = iter.next() else {
            return self.alloc_like(
                id,
                TlcExpr::Lit(crate::ir::Literal::Nothing),
                self.expr_ty(id),
            );
        };
        let mut acc = self.elaborate_expr(last);
        for item in iter {
            let value = self.elaborate_expr(item);
            let binding = self.fresh_binding();
            acc = self.alloc_like(
                id,
                TlcExpr::Let {
                    binding,
                    ty: self.expr_ty(item),
                    value,
                    body: acc,
                },
                self.expr_ty(id),
            );
        }
        acc
    }

    fn alloc_like(&mut self, source: TlcExprId, expr: TlcExpr, ty: TlcTypeId) -> TlcExprId {
        let id = self.module.expr_arena.alloc(expr);
        self.module.expr_types.insert(id, ty);
        let span = self.module.spans.get(&source).copied().unwrap_or_default();
        self.module.spans.insert(id, span);
        id
    }

    fn expr_ty(&self, id: TlcExprId) -> TlcTypeId {
        self.module.expr_types[&id]
    }

    fn fresh_binding(&mut self) -> BindingId {
        loop {
            let binding = BindingId(self.next_fresh);
            self.next_fresh = self.next_fresh.saturating_sub(1);
            if self.used_bindings.insert(binding) {
                return binding;
            }
        }
    }
}

fn collect_expr_bindings(expr: &TlcExpr, used: &mut HashSet<BindingId>) {
    match expr {
        TlcExpr::Var(binding) | TlcExpr::Lam(binding, _, _) => {
            used.insert(*binding);
        }
        TlcExpr::Let { binding, .. } => {
            used.insert(*binding);
        }
        TlcExpr::Letrec { bindings, .. } => {
            for (binding, _, _) in bindings {
                used.insert(*binding);
            }
        }
        TlcExpr::Case(_, alts) => {
            for alt in alts {
                collect_pat_bindings(&alt.pat, used);
            }
        }
        _ => {}
    }
}

fn collect_pat_bindings(pat: &crate::ir::TlcPat, used: &mut HashSet<BindingId>) {
    match pat {
        crate::ir::TlcPat::Bind(binding) => {
            used.insert(*binding);
        }
        crate::ir::TlcPat::Tuple(items) => {
            for item in items {
                match item {
                    crate::ir::TlcPatItem::Named { pat, .. }
                    | crate::ir::TlcPatItem::Positional(pat) => collect_pat_bindings(pat, used),
                }
            }
        }
        crate::ir::TlcPat::Record(fields) => {
            for (_, pat) in fields {
                collect_pat_bindings(pat, used);
            }
        }
        crate::ir::TlcPat::Variant(_, inner) => collect_pat_bindings(inner, used),
        crate::ir::TlcPat::Wildcard | crate::ir::TlcPat::Lit(_) | crate::ir::TlcPat::Atom(_) => {}
    }
}

fn restore_subst(
    subst: &mut HashMap<BindingId, BindingId>,
    binding: BindingId,
    old: Option<BindingId>,
) {
    if let Some(old) = old {
        subst.insert(binding, old);
    } else {
        subst.remove(&binding);
    }
}
