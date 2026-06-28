//! Cross-function effect handling via inline-specialization.
//!
//! The lexical CPS elaborator (`super::cps`) only discharges a `perform` that is
//! syntactically enclosed by its `handle`. A `perform` reached only through a
//! call to a separate function therefore survives elaboration and is rejected by
//! the residual-effect gate, even when every call site is handled — a run-vs-
//! compile parity gap (`zutai-eval` resolves handlers *dynamically* at perform
//! time, so closures carry no handler stack and it accepts these programs).
//!
//! This pass closes the gap for the sound, common case: a fully-saturated direct
//! call to a monomorphic, non-recursive, effectful top-level function is
//! beta-reduced into its call site (`f a` -> `let p = a in body`, with every
//! introduced binder freshened to avoid capture across call sites and arguments
//! `let`-bound — never substituted — so their effects fire exactly once in
//! left-to-right order). The relocated `perform` is then lexically enclosed by
//! the call site's handler and the existing, trusted lexical CPS elaborator
//! discharges it identically to the interpreter.
//!
//! Recursive (self/mutual) effectful callees are left for the reify pass
//! (`super::reify`), which lowers them to a free-monad `Computation` driver
//! rather than inlining. Polymorphic, higher-order, and let-bound effectful
//! callees, and partial applications, are still left untouched and stay gated
//! (refused, never miscompiled) — the reachability-scoped residual-effect gate
//! is the safety net. Inlined-away decls are dead-code-eliminated so their now
//! orphaned bodies and effectful function types stop tripping the gate.

use rustc_hash::{FxHashMap, FxHashSet};
use zutai_hir::BindingId;

use crate::ir::{
    Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcModule, TlcPat, TlcPatItem, TlcType, TlcTypeId,
};
use crate::monomorphize::push_child_exprs;

impl TlcModule {
    /// Inline fully-saturated calls to monomorphic, non-recursive, effectful
    /// top-level functions into their call sites so the lexical CPS elaborator
    /// can discharge the relocated `perform`s, then drop the now-dead callees.
    ///
    pub fn inline_effectful_calls(&mut self) {
        let mut inliner = EffectInliner::new(self);
        if inliner.inlinable.is_empty() {
            return;
        }
        inliner.run();
    }
}

/// A peeled effectful callee: its value parameters (in application order) and
/// the innermost body reached after stripping the leading lambda chain.
struct InlineTarget {
    params: Vec<(BindingId, TlcTypeId)>,
    body: TlcExprId,
}

struct EffectInliner<'m> {
    module: &'m mut crate::ir::TlcModule,
    used_bindings: FxHashSet<BindingId>,
    next_fresh: u32,
    inlinable: FxHashMap<BindingId, InlineTarget>,
}

impl<'m> EffectInliner<'m> {
    fn new(module: &'m mut crate::ir::TlcModule) -> Self {
        let mut used_bindings = FxHashSet::default();
        for (_, decl) in module.decl_arena.iter() {
            match decl {
                TlcDecl::Value { binding, .. } | TlcDecl::TypeAlias { binding, .. } => {
                    used_bindings.insert(*binding);
                }
            }
        }
        for (_, expr) in module.expr_arena.iter() {
            super::collect_expr_bindings(expr, &mut used_bindings);
        }

        // Candidate effectful top-level functions: effectful arrow type, body is
        // a monomorphic value-lambda chain whose saturated result is no longer an
        // effectful arrow (so its `perform`s become lexically exposed when the
        // call site is fully applied).
        let decl_sigs: Vec<(BindingId, TlcTypeId, TlcExprId)> = module
            .decl_arena
            .iter()
            .filter_map(|(_, decl)| match decl {
                TlcDecl::Value { binding, ty, body } => Some((*binding, *ty, *body)),
                TlcDecl::TypeAlias { .. } => None,
            })
            .collect();
        let mut candidates: FxHashMap<BindingId, InlineTarget> = FxHashMap::default();
        for (binding, ty, body) in decl_sigs {
            if !fun_spine_has_effect(module, ty) {
                continue;
            }
            if let Some(target) = peel_value_lambdas(module, ty, body) {
                candidates.insert(binding, target);
            }
        }

        // Drop callees on any cycle of the effectful-call graph (self or mutual
        // recursion): inlining them would not terminate. Leaving them gated is
        // sound — a residual call to an effectful function keeps the enclosing
        // handle ineligible, so the gate refuses rather than miscompiles.
        let cyclic = cyclic_candidates(module, &candidates);
        candidates.retain(|binding, _| !cyclic.contains(binding));

        Self {
            module,
            used_bindings,
            next_fresh: u32::MAX,
            inlinable: candidates,
        }
    }

    fn run(&mut self) {
        if let Some(final_expr) = self.module.final_expr {
            let rewritten = self.inline_expr(final_expr);
            self.module.final_expr = Some(rewritten);
        }
        let decl_ids: Vec<_> = self.module.decls.clone();
        for decl_id in decl_ids {
            let body = match self.module.decl_arena[decl_id] {
                TlcDecl::Value { body, .. } => body,
                TlcDecl::TypeAlias { .. } => continue,
            };
            let rewritten = self.inline_expr(body);
            if let TlcDecl::Value { body, .. } = &mut self.module.decl_arena[decl_id] {
                *body = rewritten;
            }
        }
        self.drop_dead_candidates();
    }

    /// Rewrite `id`, inlining any fully-saturated call to an inlinable callee and
    /// otherwise recursing structurally into children.
    fn inline_expr(&mut self, id: TlcExprId) -> TlcExprId {
        if let Some((callee, args)) = self.match_inlinable_call(id) {
            return self.inline_call(id, callee, args);
        }
        match self.module.expr_arena[id].clone() {
            TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => id,
            TlcExpr::Lam(b, ty, body) => {
                let body = self.inline_expr(body);
                self.alloc_from(id, TlcExpr::Lam(b, ty, body))
            }
            TlcExpr::App(func, arg) => {
                let func = self.inline_expr(func);
                let arg = self.inline_expr(arg);
                self.alloc_from(id, TlcExpr::App(func, arg))
            }
            TlcExpr::TyLam(var, kind, body) => {
                let body = self.inline_expr(body);
                self.alloc_from(id, TlcExpr::TyLam(var, kind, body))
            }
            TlcExpr::TyApp(body, ty) => {
                let body = self.inline_expr(body);
                self.alloc_from(id, TlcExpr::TyApp(body, ty))
            }
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let value = self.inline_expr(value);
                let body = self.inline_expr(body);
                self.alloc_from(
                    id,
                    TlcExpr::Let {
                        binding,
                        ty,
                        value,
                        body,
                    },
                )
            }
            TlcExpr::Letrec { bindings, body } => {
                let bindings = bindings
                    .into_iter()
                    .map(|(b, ty, value)| (b, ty, self.inline_expr(value)))
                    .collect();
                let body = self.inline_expr(body);
                self.alloc_from(id, TlcExpr::Letrec { bindings, body })
            }
            TlcExpr::Case(scrutinee, alts) => {
                let scrutinee = self.inline_expr(scrutinee);
                let alts = alts
                    .into_iter()
                    .map(|alt| TlcAlt {
                        pat: alt.pat,
                        guard: alt.guard.map(|guard| self.inline_expr(guard)),
                        body: self.inline_expr(alt.body),
                    })
                    .collect();
                self.alloc_from(id, TlcExpr::Case(scrutinee, alts))
            }
            TlcExpr::Record(fields) => {
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| (name, self.inline_expr(value)))
                    .collect();
                self.alloc_from(id, TlcExpr::Record(fields))
            }
            TlcExpr::RecordUpdate { receiver, fields } => {
                let receiver = self.inline_expr(receiver);
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| (name, self.inline_expr(value)))
                    .collect();
                self.alloc_from(id, TlcExpr::RecordUpdate { receiver, fields })
            }
            TlcExpr::GetField(base, field) => {
                let base = self.inline_expr(base);
                self.alloc_from(id, TlcExpr::GetField(base, field))
            }
            TlcExpr::Tuple(items) => {
                let items = items
                    .into_iter()
                    .map(|item| match item {
                        crate::ir::TlcTupleItem::Named { name, value } => {
                            crate::ir::TlcTupleItem::Named {
                                name,
                                value: self.inline_expr(value),
                            }
                        }
                        crate::ir::TlcTupleItem::Positional(value) => {
                            crate::ir::TlcTupleItem::Positional(self.inline_expr(value))
                        }
                    })
                    .collect();
                self.alloc_from(id, TlcExpr::Tuple(items))
            }
            TlcExpr::List(items) => {
                let items = items
                    .into_iter()
                    .map(|item| self.inline_expr(item))
                    .collect();
                self.alloc_from(id, TlcExpr::List(items))
            }
            TlcExpr::Builtin(op, lhs, rhs) => {
                let lhs = self.inline_expr(lhs);
                let rhs = self.inline_expr(rhs);
                self.alloc_from(id, TlcExpr::Builtin(op, lhs, rhs))
            }
            TlcExpr::Variant(tag, payload) => {
                let payload = self.inline_expr(payload);
                self.alloc_from(id, TlcExpr::Variant(tag, payload))
            }
            TlcExpr::Perform { op, arg } => {
                let arg = self.inline_expr(arg);
                self.alloc_from(id, TlcExpr::Perform { op, arg })
            }
            TlcExpr::Handle {
                expr,
                value,
                finally,
                ops,
            } => {
                let expr = self.inline_expr(expr);
                let value = value.map(|value| self.inline_expr(value));
                let finally = finally.map(|finally| self.inline_expr(finally));
                let ops = ops
                    .into_iter()
                    .map(|clause| crate::ir::TlcHandleClause {
                        op: clause.op,
                        body: self.inline_expr(clause.body),
                    })
                    .collect();
                self.alloc_from(
                    id,
                    TlcExpr::Handle {
                        expr,
                        value,
                        finally,
                        ops,
                    },
                )
            }
            TlcExpr::Resume { value } => {
                let value = self.inline_expr(value);
                self.alloc_from(id, TlcExpr::Resume { value })
            }
            TlcExpr::Sequence(items) => {
                let items = items
                    .into_iter()
                    .map(|item| self.inline_expr(item))
                    .collect();
                self.alloc_from(id, TlcExpr::Sequence(items))
            }
        }
    }

    /// If `id` is a `Var(f) a1 .. aN` application spine whose head is an
    /// inlinable callee and whose argument count exactly saturates it, return
    /// `(f, [a1, .., aN])` in application order. Partial and over-application
    /// (where it descends to a saturating sub-spine) and non-`Var` heads return
    /// `None` so they recurse or stay gated.
    fn match_inlinable_call(&self, id: TlcExprId) -> Option<(BindingId, Vec<TlcExprId>)> {
        let mut args = Vec::new();
        let mut cur = id;
        loop {
            match &self.module.expr_arena[cur] {
                TlcExpr::App(func, arg) => {
                    args.push(*arg);
                    cur = *func;
                }
                TlcExpr::Var(binding) => {
                    let target = self.inlinable.get(binding)?;
                    if args.len() != target.params.len() {
                        return None;
                    }
                    args.reverse();
                    return Some((*binding, args));
                }
                _ => return None,
            }
        }
    }

    fn inline_call(
        &mut self,
        call_id: TlcExprId,
        callee: BindingId,
        args: Vec<TlcExprId>,
    ) -> TlcExprId {
        let (params, body) = {
            let target = &self.inlinable[&callee];
            (target.params.clone(), target.body)
        };
        // Inline the arguments first; they share the call site's handler scope.
        let args: Vec<TlcExprId> = args.into_iter().map(|arg| self.inline_expr(arg)).collect();

        // Freshen the callee body, renaming each value parameter to a fresh
        // `let` binder, then inline any further effectful calls it exposes (the
        // candidate graph is acyclic, so this terminates).
        let mut subst: FxHashMap<BindingId, BindingId> = FxHashMap::default();
        let fresh_params: Vec<BindingId> = params
            .iter()
            .map(|(param, _)| {
                let fresh = self.fresh_binding();
                subst.insert(*param, fresh);
                fresh
            })
            .collect();
        let mut result = self.freshen_expr(body, &mut subst);
        // A clause body lowers to a `Case` whose recorded type is the function's
        // type, not its result; once saturated the inlined body has the call's
        // result type. Re-stamp it so the result type does not leak an effectful
        // arrow into the gate or the CPS join lambda.
        if let Some(call_ty) = self.module.expr_types.get(&call_id).copied() {
            self.module.expr_types.insert(result, call_ty);
        }
        result = self.inline_expr(result);

        // Wrap in `let`s in argument order: first argument outermost so its
        // effects sequence before later arguments and the body. The whole
        // expression has the saturated call's result type.
        let call_ty = self.module.expr_types.get(&call_id).copied();
        for index in (0..params.len()).rev() {
            let binding = fresh_params[index];
            let param_ty = params[index].1;
            let value = args[index];
            let result_ty = call_ty
                .or_else(|| self.module.expr_types.get(&result).copied())
                .unwrap_or(param_ty);
            let let_id = self.module.expr_arena.alloc(TlcExpr::Let {
                binding,
                ty: param_ty,
                value,
                body: result,
            });
            self.module.expr_types.insert(let_id, result_ty);
            let span = self.module.spans.get(&call_id).copied().unwrap_or_default();
            self.module.spans.insert(let_id, span);
            result = let_id;
        }
        result
    }

    /// Capture-avoiding deep clone: freshen every introduced binder
    /// (Lam/Let/Letrec/Case-pattern) through `subst`, leave free globals
    /// untouched, and copy the type/span/dict side tables of every node.
    fn freshen_expr(
        &mut self,
        id: TlcExprId,
        subst: &mut FxHashMap<BindingId, BindingId>,
    ) -> TlcExprId {
        let new = match self.module.expr_arena[id].clone() {
            TlcExpr::Var(binding) => TlcExpr::Var(subst.get(&binding).copied().unwrap_or(binding)),
            TlcExpr::Lit(lit) => TlcExpr::Lit(lit),
            TlcExpr::Import(src) => TlcExpr::Import(src),
            TlcExpr::Lam(binding, ty, body) => {
                let fresh = self.fresh_binding();
                let old = subst.insert(binding, fresh);
                let body = self.freshen_expr(body, subst);
                super::restore_subst(subst, binding, old);
                TlcExpr::Lam(fresh, ty, body)
            }
            TlcExpr::App(func, arg) => {
                let func = self.freshen_expr(func, subst);
                let arg = self.freshen_expr(arg, subst);
                TlcExpr::App(func, arg)
            }
            TlcExpr::TyLam(var, kind, body) => {
                let body = self.freshen_expr(body, subst);
                TlcExpr::TyLam(var, kind, body)
            }
            TlcExpr::TyApp(body, ty) => {
                let body = self.freshen_expr(body, subst);
                TlcExpr::TyApp(body, ty)
            }
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let value = self.freshen_expr(value, subst);
                let fresh = self.fresh_binding();
                let old = subst.insert(binding, fresh);
                let body = self.freshen_expr(body, subst);
                super::restore_subst(subst, binding, old);
                TlcExpr::Let {
                    binding: fresh,
                    ty,
                    value,
                    body,
                }
            }
            TlcExpr::Letrec { bindings, body } => {
                let mut restores = Vec::with_capacity(bindings.len());
                let fresh_binders: Vec<BindingId> = bindings
                    .iter()
                    .map(|(binding, _, _)| {
                        let fresh = self.fresh_binding();
                        restores.push((*binding, subst.insert(*binding, fresh)));
                        fresh
                    })
                    .collect();
                let bindings = bindings
                    .into_iter()
                    .enumerate()
                    .map(|(index, (_, ty, value))| {
                        (fresh_binders[index], ty, self.freshen_expr(value, subst))
                    })
                    .collect();
                let body = self.freshen_expr(body, subst);
                for (binding, old) in restores {
                    super::restore_subst(subst, binding, old);
                }
                TlcExpr::Letrec { bindings, body }
            }
            TlcExpr::Case(scrutinee, alts) => {
                let scrutinee = self.freshen_expr(scrutinee, subst);
                let alts = alts
                    .into_iter()
                    .map(|alt| {
                        let mut restores = Vec::new();
                        let pat = self.freshen_pat(alt.pat, subst, &mut restores);
                        let guard = alt.guard.map(|guard| self.freshen_expr(guard, subst));
                        let body = self.freshen_expr(alt.body, subst);
                        for (binding, old) in restores {
                            super::restore_subst(subst, binding, old);
                        }
                        TlcAlt { pat, guard, body }
                    })
                    .collect();
                TlcExpr::Case(scrutinee, alts)
            }
            TlcExpr::Record(fields) => TlcExpr::Record(
                fields
                    .into_iter()
                    .map(|(name, value)| (name, self.freshen_expr(value, subst)))
                    .collect(),
            ),
            TlcExpr::RecordUpdate { receiver, fields } => {
                let receiver = self.freshen_expr(receiver, subst);
                let fields = fields
                    .into_iter()
                    .map(|(name, value)| (name, self.freshen_expr(value, subst)))
                    .collect();
                TlcExpr::RecordUpdate { receiver, fields }
            }
            TlcExpr::GetField(base, field) => {
                TlcExpr::GetField(self.freshen_expr(base, subst), field)
            }
            TlcExpr::Tuple(items) => TlcExpr::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        crate::ir::TlcTupleItem::Named { name, value } => {
                            crate::ir::TlcTupleItem::Named {
                                name,
                                value: self.freshen_expr(value, subst),
                            }
                        }
                        crate::ir::TlcTupleItem::Positional(value) => {
                            crate::ir::TlcTupleItem::Positional(self.freshen_expr(value, subst))
                        }
                    })
                    .collect(),
            ),
            TlcExpr::List(items) => TlcExpr::List(
                items
                    .into_iter()
                    .map(|item| self.freshen_expr(item, subst))
                    .collect(),
            ),
            TlcExpr::Builtin(op, lhs, rhs) => {
                let lhs = self.freshen_expr(lhs, subst);
                let rhs = self.freshen_expr(rhs, subst);
                TlcExpr::Builtin(op, lhs, rhs)
            }
            TlcExpr::Variant(tag, payload) => {
                TlcExpr::Variant(tag, self.freshen_expr(payload, subst))
            }
            TlcExpr::Perform { op, arg } => TlcExpr::Perform {
                op,
                arg: self.freshen_expr(arg, subst),
            },
            TlcExpr::Handle {
                expr,
                value,
                finally,
                ops,
            } => {
                let expr = self.freshen_expr(expr, subst);
                let value = value.map(|value| self.freshen_expr(value, subst));
                let finally = finally.map(|finally| self.freshen_expr(finally, subst));
                let ops = ops
                    .into_iter()
                    .map(|clause| crate::ir::TlcHandleClause {
                        op: clause.op,
                        body: self.freshen_expr(clause.body, subst),
                    })
                    .collect();
                TlcExpr::Handle {
                    expr,
                    value,
                    finally,
                    ops,
                }
            }
            TlcExpr::Resume { value } => TlcExpr::Resume {
                value: self.freshen_expr(value, subst),
            },
            TlcExpr::Sequence(items) => TlcExpr::Sequence(
                items
                    .into_iter()
                    .map(|item| self.freshen_expr(item, subst))
                    .collect(),
            ),
        };
        self.alloc_from(id, new)
    }

    fn freshen_pat(
        &mut self,
        pat: TlcPat,
        subst: &mut FxHashMap<BindingId, BindingId>,
        restores: &mut Vec<(BindingId, Option<BindingId>)>,
    ) -> TlcPat {
        match pat {
            TlcPat::Wildcard => TlcPat::Wildcard,
            TlcPat::Lit(lit) => TlcPat::Lit(lit),
            TlcPat::Atom(name) => TlcPat::Atom(name),
            TlcPat::Bind(binding) => {
                let fresh = self.fresh_binding();
                restores.push((binding, subst.insert(binding, fresh)));
                TlcPat::Bind(fresh)
            }
            TlcPat::Tuple(items) => TlcPat::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        TlcPatItem::Named { name, pat } => TlcPatItem::Named {
                            name,
                            pat: self.freshen_pat(pat, subst, restores),
                        },
                        TlcPatItem::Positional(pat) => {
                            TlcPatItem::Positional(self.freshen_pat(pat, subst, restores))
                        }
                    })
                    .collect(),
            ),
            TlcPat::ListNil => TlcPat::ListNil,
            TlcPat::ListCons(head, tail) => TlcPat::ListCons(
                Box::new(self.freshen_pat(*head, subst, restores)),
                Box::new(self.freshen_pat(*tail, subst, restores)),
            ),
            TlcPat::Record(fields) => TlcPat::Record(
                fields
                    .into_iter()
                    .map(|(name, pat)| (name, self.freshen_pat(pat, subst, restores)))
                    .collect(),
            ),
            TlcPat::Variant(tag, inner) => {
                TlcPat::Variant(tag, Box::new(self.freshen_pat(*inner, subst, restores)))
            }
        }
    }

    /// Drop inlined-away effectful candidate decls that no live code references,
    /// so their orphaned bodies (and effectful function types) stop tripping the
    /// residual-effect gate. Iterated to a fixpoint: a candidate referenced only
    /// by other dead candidates is itself dead.
    fn drop_dead_candidates(&mut self) {
        let candidates: FxHashSet<BindingId> = self.inlinable.keys().copied().collect();

        // Var references contained in each candidate's own decl body.
        let mut candidate_refs: FxHashMap<BindingId, FxHashSet<BindingId>> = FxHashMap::default();
        // Seed the live worklist from every root: the final expression and every
        // non-candidate decl body (those are never dropped).
        let mut worklist: Vec<BindingId> = Vec::new();
        for (_, decl) in self.module.decl_arena.iter() {
            let TlcDecl::Value { binding, body, .. } = decl else {
                continue;
            };
            let mut refs = FxHashSet::default();
            collect_var_refs(self.module, *body, &mut refs);
            if candidates.contains(binding) {
                candidate_refs.insert(*binding, refs);
            } else {
                worklist.extend(refs.into_iter().filter(|r| candidates.contains(r)));
            }
        }
        if let Some(final_expr) = self.module.final_expr {
            let mut refs = FxHashSet::default();
            collect_var_refs(self.module, final_expr, &mut refs);
            worklist.extend(refs.into_iter().filter(|r| candidates.contains(r)));
        }

        let mut live: FxHashSet<BindingId> = FxHashSet::default();
        while let Some(binding) = worklist.pop() {
            if !live.insert(binding) {
                continue;
            }
            if let Some(refs) = candidate_refs.get(&binding) {
                worklist.extend(refs.iter().copied().filter(|r| candidates.contains(r)));
            }
        }

        let kept: Vec<_> = self
            .module
            .decls
            .iter()
            .copied()
            .filter(|decl_id| match &self.module.decl_arena[*decl_id] {
                TlcDecl::Value { binding, .. } => {
                    !candidates.contains(binding) || live.contains(binding)
                }
                TlcDecl::TypeAlias { .. } => true,
            })
            .collect();
        self.module.decls = kept;
    }

    fn alloc_from(&mut self, source: TlcExprId, expr: TlcExpr) -> TlcExprId {
        let ty = self.module.expr_types.get(&source).copied();
        let span = self.module.spans.get(&source).copied();
        let slot = self.module.dict_field_slots.get(&source).copied();
        let key = self.module.dict_dispatch_keys.get(&source).cloned();
        let id = self.module.expr_arena.alloc(expr);
        if let Some(ty) = ty {
            self.module.expr_types.insert(id, ty);
        }
        if let Some(span) = span {
            self.module.spans.insert(id, span);
        }
        if let Some(slot) = slot {
            self.module.dict_field_slots.insert(id, slot);
        }
        if let Some(key) = key {
            self.module.dict_dispatch_keys.insert(id, key);
        }
        id
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

/// Whether any arrow in the curried spine of `ty` carries a non-empty effect row.
fn fun_spine_has_effect(module: &crate::ir::TlcModule, ty: TlcTypeId) -> bool {
    match &module.type_arena[ty] {
        TlcType::Fun(_, ret, row) => {
            !matches!(row, Row::REmpty) || fun_spine_has_effect(module, *ret)
        }
        _ => false,
    }
}

/// Strip the leading value-lambda chain from `body`, returning each parameter
/// (binding + type, in application order) and the innermost body. The declared
/// function type `ty` is consumed in lockstep to find the saturated result type.
/// Rejects (returns `None`) anything that is not a monomorphic value function
/// whose saturated result is no longer an effectful arrow (an escaping effectful
/// closure) — those stay gated rather than inlined.
///
/// Pattern-parameter lambdas lower to `Lam(scrut, Case(scrut, [Bind(p) => …]))`,
/// so the peeled body is typically a `Case`; binding the `Lam` scrutinee to the
/// argument and freshening that `Case` reproduces the parameter match exactly.
fn peel_value_lambdas(
    module: &crate::ir::TlcModule,
    ty: TlcTypeId,
    body: TlcExprId,
) -> Option<InlineTarget> {
    let mut params = Vec::new();
    let mut cur = body;
    while let TlcExpr::Lam(lam_param, lam_ty, lam_body) = &module.expr_arena[cur] {
        let (lam_param, lam_ty, lam_body) = (*lam_param, *lam_ty, *lam_body);
        // A pattern-parameter layer lowers to `Lam(scrut, Case(Var(scrut),
        // [Bind(p) => rest]))`; bind `p` directly to the argument so the peeled
        // body is the user's body (which CPS handles), not a `Case` wrapper
        // (which it does not). A plain `Lam(p, body)` binds `p` directly.
        if let TlcExpr::Case(scrut, alts) = &module.expr_arena[lam_body]
            && alts.len() == 1
            && alts[0].guard.is_none()
            && let TlcPat::Bind(bound) = alts[0].pat
            && matches!(&module.expr_arena[*scrut], TlcExpr::Var(b) if *b == lam_param)
        {
            params.push((bound, lam_ty));
            cur = alts[0].body;
        } else {
            params.push((lam_param, lam_ty));
            cur = lam_body;
        }
    }
    if params.is_empty() {
        return None;
    }
    // Monomorphic only: a residual type-lambda means a polymorphic body whose
    // erased type arguments would have to be substituted — out of scope.
    if matches!(&module.expr_arena[cur], TlcExpr::TyLam(..)) {
        return None;
    }
    // Consume one arrow of the declared type per peeled lambda; the result must
    // not itself be an effectful arrow. (The inner expr's recorded type is
    // unreliable for pattern lambdas, so the declared type drives this check.)
    let mut result_ty = ty;
    for _ in 0..params.len() {
        let TlcType::Fun(_, ret, _) = &module.type_arena[result_ty] else {
            return None;
        };
        result_ty = *ret;
    }
    if fun_spine_has_effect(module, result_ty) {
        return None;
    }
    Some(InlineTarget { params, body: cur })
}

/// Candidates that lie on a cycle of the effectful-call graph (self or mutual
/// recursion); inlining them would not terminate, so they stay gated.
fn cyclic_candidates(
    module: &crate::ir::TlcModule,
    candidates: &FxHashMap<BindingId, InlineTarget>,
) -> FxHashSet<BindingId> {
    let mut edges: FxHashMap<BindingId, Vec<BindingId>> = FxHashMap::default();
    for (binding, target) in candidates {
        let mut refs = FxHashSet::default();
        collect_var_refs(module, target.body, &mut refs);
        let callees: Vec<BindingId> = refs
            .into_iter()
            .filter(|r| candidates.contains_key(r))
            .collect();
        edges.insert(*binding, callees);
    }
    let mut cyclic = FxHashSet::default();
    for &start in candidates.keys() {
        if reaches(start, start, &edges) {
            cyclic.insert(start);
        }
    }
    cyclic
}

/// Whether `target` is reachable from `start`'s callees in the call graph.
fn reaches(
    start: BindingId,
    target: BindingId,
    edges: &FxHashMap<BindingId, Vec<BindingId>>,
) -> bool {
    let mut seen = FxHashSet::default();
    let mut stack: Vec<BindingId> = edges.get(&start).cloned().unwrap_or_default();
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if !seen.insert(node) {
            continue;
        }
        if let Some(next) = edges.get(&node) {
            stack.extend(next.iter().copied());
        }
    }
    false
}

/// Collect every `Var` binding referenced in the subtree rooted at `root`.
fn collect_var_refs(
    module: &crate::ir::TlcModule,
    root: TlcExprId,
    out: &mut FxHashSet<BindingId>,
) {
    let mut seen = FxHashSet::default();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let TlcExpr::Var(binding) = &module.expr_arena[id] {
            out.insert(*binding);
        }
        push_child_exprs(&module.expr_arena[id], &mut stack);
    }
}
