use rustc_hash::{FxHashMap, FxHashSet};

mod cps;
mod eligible;
mod finally;
mod inline;
mod reify;
mod rewrite;

use zutai_hir::BindingId;

use crate::ir::{HostOp, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcModule, TlcTupleItem};

fn deferred_perform_needs_reifier(op: &str) -> bool {
    !matches!(
        HostOp::from_name(op),
        Some(host_op) if host_op != HostOp::IoPrint
    )
}

impl TlcModule {
    /// Elaborate handled TLC effect markers into pure TLC terms.
    ///
    /// This is intentionally conservative for the first CPS cut: matched handler
    /// operations are lowered to ordinary handler-function calls with an explicit
    /// resume lambda; unmatched operations remain as `Perform` so the existing
    /// Dataflow gate rejects them.
    pub fn elaborate_effects(&mut self) {
        let mut elaborator = EffectElaborator::new(self, false);
        elaborator.run();
    }

    /// Backend variant of [`elaborate_effects`]: leave handles residual when a
    /// handled operation is deferred behind a lambda/thunk, so the residual
    /// reifier can preserve dynamic handler scope for effectful generator cells.
    pub fn elaborate_effects_preserving_deferred_performs(&mut self) {
        let mut elaborator = EffectElaborator::new(self, true);
        elaborator.run();
    }
}
type Kont<'kont> = Box<dyn FnOnce(&mut EffectElaborator<'_>, TlcExprId) -> TlcExprId + 'kont>;

struct EffectElaborator<'module> {
    module: &'module mut TlcModule,
    used_bindings: FxHashSet<BindingId>,
    next_fresh: u32,
    preserve_deferred_performs: bool,
}

impl<'module> EffectElaborator<'module> {
    fn new(module: &'module mut TlcModule, preserve_deferred_performs: bool) -> Self {
        let mut used_bindings = FxHashSet::default();
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
            preserve_deferred_performs,
        }
    }

    fn run(&mut self) {
        let decls = self.module.decls.clone();
        for decl_id in decls {
            let body = match self.module.decl_arena[decl_id] {
                TlcDecl::Value { body, .. } => body,
                TlcDecl::TypeAlias { .. } => continue,
            };
            let rewritten = self.elaborate_expr(body);
            if let TlcDecl::Value { body, .. } = &mut self.module.decl_arena[decl_id] {
                *body = rewritten;
            }
        }
        if let Some(final_expr) = self.module.final_expr {
            self.module.final_expr = Some(self.elaborate_expr(final_expr));
        }
    }

    pub(super) fn elaborate_expr(&mut self, id: TlcExprId) -> TlcExprId {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Handle {
                expr,
                value,
                finally,
                ops,
            } => {
                if self.can_elaborate_handle(expr, finally, &ops) {
                    self.elaborate_handle(id, expr, value, ops)
                } else {
                    // A non-elaboratable handle (residual effects, or a
                    // `finally` teardown) is left intact for the interpreter and
                    // refused by the native residual-effect gate.
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
            TlcExpr::ListAppend(left, right) => {
                let left = self.elaborate_expr(left);
                let right = self.elaborate_expr(right);
                self.alloc_like(id, TlcExpr::ListAppend(left, right), self.expr_ty(id))
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
}

fn collect_expr_bindings(expr: &TlcExpr, used: &mut FxHashSet<BindingId>) {
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

fn collect_pat_bindings(pat: &crate::ir::TlcPat, used: &mut FxHashSet<BindingId>) {
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
        crate::ir::TlcPat::ListCons(head, tail) => {
            collect_pat_bindings(head, used);
            collect_pat_bindings(tail, used);
        }
        crate::ir::TlcPat::Record(fields) => {
            for (_, pat) in fields {
                collect_pat_bindings(pat, used);
            }
        }
        crate::ir::TlcPat::Variant(_, inner) => collect_pat_bindings(inner, used),
        crate::ir::TlcPat::Wildcard
        | crate::ir::TlcPat::Lit(_)
        | crate::ir::TlcPat::Atom(_)
        | crate::ir::TlcPat::ListNil => {}
    }
}

fn restore_subst(
    subst: &mut FxHashMap<BindingId, BindingId>,
    binding: BindingId,
    old: Option<BindingId>,
) {
    if let Some(old) = old {
        subst.insert(binding, old);
    } else {
        subst.remove(&binding);
    }
}
