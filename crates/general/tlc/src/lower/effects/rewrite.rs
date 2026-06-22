use std::collections::HashMap;

use zutai_hir::BindingId;

use crate::ir::{TlcAlt, TlcExpr, TlcExprId, TlcHandleClause, TlcTupleItem, TlcTypeId};

use super::restore_subst;
use super::{EffectElaborator, Kont};

impl<'module> EffectElaborator<'module> {
    pub(super) fn rewrite_handler_cps(
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
    pub(super) fn rewrite_handler_cps_sequence(
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

    pub(super) fn handler_expr_contains_control(&self, id: TlcExprId) -> bool {
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

    pub(super) fn rewrite_handler_expr(
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

    pub(super) fn freshen_pat(
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

    pub(super) fn rewrite_handler_sequence(
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

    pub(super) fn elaborate_sequence_direct(
        &mut self,
        id: TlcExprId,
        items: Vec<TlcExprId>,
    ) -> TlcExprId {
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

    pub(super) fn alloc_like(
        &mut self,
        source: TlcExprId,
        expr: TlcExpr,
        ty: TlcTypeId,
    ) -> TlcExprId {
        let id = self.module.expr_arena.alloc(expr);
        self.module.expr_types.insert(id, ty);
        let span = self.module.spans.get(&source).copied().unwrap_or_default();
        self.module.spans.insert(id, span);
        id
    }

    pub(super) fn expr_ty(&self, id: TlcExprId) -> TlcTypeId {
        self.module.expr_types[&id]
    }

    pub(super) fn fresh_binding(&mut self) -> BindingId {
        loop {
            let binding = BindingId(self.next_fresh);
            self.next_fresh = self.next_fresh.saturating_sub(1);
            if self.used_bindings.insert(binding) {
                return binding;
            }
        }
    }
}
