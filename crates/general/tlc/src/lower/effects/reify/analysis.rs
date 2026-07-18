// ── Analysis & validation ──────────────────────────────────────────────────────

use super::*;

impl<'m> Reifier<'m> {
    /// Look up a top-level `Value` decl by binding, returning `(ty, body)`.
    pub(super) fn decl_of(&self, binding: BindingId) -> Option<(TlcTypeId, TlcExprId)> {
        self.module
            .decl_arena
            .iter()
            .find_map(|(_, decl)| match decl {
                TlcDecl::Value {
                    binding: b,
                    ty,
                    body,
                } if *b == binding => Some((*ty, *body)),
                _ => None,
            })
    }

    /// Overwrite a `Value` decl's type and body in place.
    pub(super) fn set_decl(&mut self, binding: BindingId, ty: TlcTypeId, body: TlcExprId) {
        let ids: Vec<_> = self.module.decl_arena.iter().map(|(id, _)| id).collect();
        for id in ids {
            if let TlcDecl::Value {
                binding: b,
                ty: t,
                body: bd,
            } = &mut self.module.decl_arena[id]
                && *b == binding
            {
                *t = ty;
                *bd = body;
                return;
            }
        }
    }

    /// Every `Var` binding referenced in the subtree rooted at `root`.
    pub(super) fn var_refs(&self, root: TlcExprId) -> FxHashSet<BindingId> {
        let mut out = FxHashSet::default();
        let mut seen = FxHashSet::default();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let TlcExpr::Var(b) = &self.module.expr_arena[id] {
                out.insert(*b);
            }
            push_child_exprs(&self.module.expr_arena[id], &mut stack);
        }
        out
    }

    /// Whether `ty`'s curried spine carries a non-empty effect row.
    pub(super) fn fun_spine_has_effect(&self, ty: TlcTypeId) -> bool {
        match &self.module.type_arena[ty] {
            TlcType::Fun(_, ret, row) => {
                !matches!(row, Row::REmpty) || self.fun_spine_has_effect(*ret)
            }
            _ => false,
        }
    }

    /// The operation names appearing in `ty`'s curried-spine effect rows.
    pub(super) fn effect_ops_of(&self, ty: TlcTypeId, out: &mut FxHashSet<String>) {
        if let TlcType::Fun(_, ret, row) = &self.module.type_arena[ty] {
            let mut r = row;
            while let Row::RExtend { label, tail, .. } = r {
                out.insert(label.clone());
                r = tail;
            }
            self.effect_ops_of(*ret, out);
        }
    }

    /// Transitive set of effectful top-level functions reachable from `root`
    /// whose effect operations are all handled by `handle_ops` (closed-row).
    pub(super) fn effectful_fn_set(
        &self,
        root: TlcExprId,
        handle_ops: &FxHashSet<String>,
    ) -> FxHashSet<BindingId> {
        let mut set = FxHashSet::default();
        let mut visited = FxHashSet::default();
        let mut work: Vec<BindingId> = self.var_refs(root).into_iter().collect();
        while let Some(b) = work.pop() {
            if !visited.insert(b) {
                continue;
            }
            let Some((ty, body)) = self.decl_of(b) else {
                continue;
            };
            if self.fun_spine_has_effect(ty) {
                // An effectful callee whose ops are all handled is reified; one with
                // an unhandled op cannot be — don't descend past it either.
                let mut ops = FxHashSet::default();
                self.effect_ops_of(ty, &mut ops);
                if !ops.is_subset(handle_ops) {
                    continue;
                }
                set.insert(b);
            }
            // Descend into both effectful-handled callees *and* pure wrapper bindings
            // (e.g. a record `{ f = g }` holding an effectful function), so a callee
            // reached only through a wrapper is still discovered.
            work.extend(self.var_refs(body));
        }
        set
    }

    /// Top-level `Value` bindings that are not themselves reified but hold an
    /// `fn_set` member as a value reached *outside* the handle subtree — e.g. a
    /// record `box = { f = g }` whose field is later projected and called. Their
    /// declared types are rewritten to monadic form alongside the reified callees.
    pub(super) fn wrapper_set(
        &self,
        handle_id: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
    ) -> FxHashSet<BindingId> {
        let mut wrappers = FxHashSet::default();
        for (_, decl) in self.module.decl_arena.iter() {
            if let TlcDecl::Value { binding, body, .. } = decl
                && !fn_set.contains(binding)
                && self.refs_fn_outside(*body, handle_id, fn_set)
            {
                wrappers.insert(*binding);
            }
        }
        wrappers
    }

    /// Whether any reified function *or wrapper* is referenced from outside this
    /// handle's scope (anything but the handle subtree and the reified/wrapper
    /// bodies themselves). Rewriting a reified callee's or a wrapper's type would
    /// break any use that observes it unrewritten, so such an escape forces refusal.
    pub(super) fn fn_escapes_scope(
        &self,
        handle_id: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
        wrapper_set: &FxHashSet<BindingId>,
    ) -> bool {
        // Both reified callees and wrappers have their declared types rewritten, so
        // a reference to either from out-of-scope code escapes.
        let mut sensitive = fn_set.clone();
        sensitive.extend(wrapper_set.iter().copied());
        // Scan the final expr and every decl body that is neither a reified callee
        // nor a wrapper (those bodies are in-scope and may reference sensitive
        // bindings); a sensitive reference reached without passing through the
        // handle subtree escapes.
        let mut roots: Vec<TlcExprId> = Vec::new();
        if let Some(final_expr) = self.module.final_expr {
            roots.push(final_expr);
        }
        for (_, decl) in self.module.decl_arena.iter() {
            if let TlcDecl::Value { binding, body, .. } = decl
                && !fn_set.contains(binding)
                && !wrapper_set.contains(binding)
            {
                roots.push(*body);
            }
        }
        for root in roots {
            if self.refs_fn_outside(root, handle_id, &sensitive) {
                return true;
            }
        }
        false
    }

    /// Whether `root` references a member of `fn_set` without passing through the
    /// `handle` node (whose subtree is in-scope).
    pub(super) fn refs_fn_outside(
        &self,
        root: TlcExprId,
        handle_id: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
    ) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if id == handle_id || !seen.insert(id) {
                continue;
            }
            if let TlcExpr::Var(b) = &self.module.expr_arena[id]
                && fn_set.contains(b)
            {
                return true;
            }
            push_child_exprs(&self.module.expr_arena[id], &mut stack);
        }
        false
    }

    /// Whether the subtree contains an effectful-callee reference or a
    /// `Perform`/`Handle`/`Resume` control node.
    pub(super) fn is_effectful(&self, id: TlcExprId) -> bool {
        if self.subtree_has_comp_binder(id) {
            return true;
        }
        if self.node_is_eff_cell_case(id) {
            return true;
        }
        let fn_set = &self.ctx().fn_set;
        let handle_ops = &self.ctx().handle_ops;
        self.subtree_is_effectful(id, fn_set, handle_ops)
    }

    /// Whether the subtree contains no `Perform`/`Handle`/`Resume` node (a pure
    /// computation safe to reuse directly as a value).
    pub(super) fn no_residual_control(&self, id: TlcExprId) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if matches!(
                self.module.expr_arena[cur],
                TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. }
            ) {
                return false;
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        true
    }

    /// A handler clause body is reifiable if it contains no `Perform`/`Handle`,
    /// no call into `fn_set`, and every `Resume` value is pure.
    pub(super) fn handler_body_ok(&self, id: TlcExprId, fn_set: &FxHashSet<BindingId>) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Perform { .. } | TlcExpr::Handle { .. } => return false,
                TlcExpr::Var(b) if fn_set.contains(b) => return false,
                TlcExpr::Resume { value } if !self.no_residual_control(*value) => return false,
                _ => {}
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        true
    }

    /// Peel the leading value-lambda chain, returning `[(param, paramTy)]` and the
    /// innermost computation body.
    pub(super) fn peel_lams(&self, body: TlcExprId) -> (Vec<(BindingId, TlcTypeId)>, TlcExprId) {
        let mut params = Vec::new();
        let mut cur = body;
        while let TlcExpr::Lam(p, ty, b) = self.module.expr_arena[cur] {
            params.push((p, ty));
            cur = b;
        }
        (params, cur)
    }

    /// The `(argTy, resultTy)` of each value parameter in `ty`'s curried spine, up
    /// to and including the arrow that carries the effect row (the last value
    /// parameter of an effectful function).
    pub(super) fn arrow_value_spine(&self, ty: TlcTypeId) -> Vec<(TlcTypeId, TlcTypeId)> {
        let mut out = Vec::new();
        let mut cur = ty;
        while let TlcType::Fun(a, b, row) = &self.module.type_arena[cur] {
            out.push((*a, *b));
            let (b, empty) = (*b, matches!(row, Row::REmpty));
            if !empty {
                break;
            }
            cur = b;
        }
        out
    }

    /// Like `peel_lams`, but eta-expands a partially-applied body up to the full
    /// value arity of `decl_ty` so the result is a saturated computation core.
    pub(super) fn peel_or_eta(
        &mut self,
        decl_ty: TlcTypeId,
        body: TlcExprId,
    ) -> (Vec<(BindingId, TlcTypeId)>, TlcExprId) {
        let spine = self.arrow_value_spine(decl_ty);
        let (mut params, mut core) = self.peel_lams(body);
        let mut idx = params.len();
        while idx < spine.len() {
            let (arg_ty, res_ty) = spine[idx];
            let p = self.fresh_binding();
            let pv = self.var(p, arg_ty);
            core = self.mk(TlcExpr::App(core, pv), res_ty);
            params.push((p, arg_ty));
            idx += 1;
        }
        (params, core)
    }

    /// Rewrite every inline under-saturated effectful-arrow application reachable
    /// from `roots` (a partial application of an effectful function whose own type
    /// is still an effectful arrow — a closure *value*, not a saturated computation)
    /// into an eta-expanded lambda value, recorded in `eta_fn_args`. After this,
    /// `subtree_is_effectful` classifies the argument as pure (a `Lam`), so the
    /// enclosing higher-order call becomes reifiable and the lambda body is reified
    /// at the call site by `maybe_reify_eta_fn_arg`.
    pub(super) fn normalize_undersaturated_eff_args(
        &mut self,
        roots: &[TlcExprId],
        fn_set: &FxHashSet<BindingId>,
        handle_ops: &FxHashSet<String>,
    ) {
        let mut seen = FxHashSet::default();
        let mut stack: Vec<TlcExprId> = roots.to_vec();
        let mut apps: Vec<TlcExprId> = Vec::new();
        // Nodes appearing as the *function* side of an application — they belong to a
        // larger spine that will saturate them, so they must not be eta-expanded
        // (only a maximal partial application used as a value is a candidate).
        let mut func_positions: FxHashSet<TlcExprId> = FxHashSet::default();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if let TlcExpr::App(func, _) = &self.module.expr_arena[cur] {
                func_positions.insert(*func);
                apps.push(cur);
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        for n in apps {
            if !func_positions.contains(&n)
                && self.is_undersaturated_eff_value(n, fn_set, handle_ops)
            {
                self.eta_expand_in_place(n);
            }
        }
    }

    /// Whether `id` is an inline partial application of an effectful callee that
    /// yields a *value* of effectful-arrow type (still curried), with every
    /// already-applied argument pure. Such a node is a closure, not an immediate
    /// effect, and is safe to eta-expand into a lambda.
    pub(super) fn is_undersaturated_eff_value(
        &self,
        id: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        if !self.fun_spine_has_effect(self.expr_ty(id)) {
            return false;
        }
        let mut cur = id;
        while let TlcExpr::App(func, arg) = &self.module.expr_arena[cur] {
            // An effectful already-applied argument would make the saturated core
            // itself non-reifiable; leave such a node residual (gated).
            if self.subtree_is_effectful(*arg, fn_set, handle_ops) {
                return false;
            }
            cur = *func;
        }
        self.node_is_eff_fn_ref(cur, fn_set, handle_ops)
            || self.node_is_eff_field_ref(cur, handle_ops)
    }

    /// Replace the under-saturated effectful application in arena slot `n` with an
    /// eta-expanded lambda `\p1 .. pk. (n') p1 .. pk` (where `n'` is a copy of the
    /// original partial application), so existing parents transparently see the
    /// lambda value. The lambda is recorded in `eta_fn_args`.
    pub(super) fn eta_expand_in_place(&mut self, n: TlcExprId) {
        let nty = self.expr_ty(n);
        let spine = self.arrow_value_spine(nty);
        if spine.is_empty() {
            return;
        }
        let span = self.module.spans.get(&n).cloned().unwrap_or_default();
        let copy = self
            .module
            .expr_arena
            .alloc(self.module.expr_arena[n].clone());
        self.module.expr_types.insert(copy, nty);
        self.module.spans.insert(copy, span);
        let mut params: Vec<(BindingId, TlcTypeId)> = Vec::new();
        let mut core = copy;
        for (arg_ty, res_ty) in &spine {
            let p = self.fresh_binding();
            let pv = self.var(p, *arg_ty);
            core = self.mk(TlcExpr::App(core, pv), *res_ty);
            params.push((p, *arg_ty));
        }
        // Inner lambdas (params[1..], innermost first); the outermost is written into
        // slot `n` so the parent reference stays valid.
        let mut body = core;
        let mut body_ty = self.expr_ty(core);
        for (p, pty) in params[1..].iter().rev() {
            let t = self.fun_ty(*pty, body_ty);
            body = self.mk(TlcExpr::Lam(*p, *pty, body), t);
            body_ty = t;
        }
        let (p0, pty0) = params[0];
        let t0 = self.fun_ty(pty0, body_ty);
        self.module.expr_arena[n] = TlcExpr::Lam(p0, pty0, body);
        self.module.expr_types.insert(n, t0);
        self.eta_fn_args.insert(n);
    }

    /// Structural reifiability of a computation expression.
    pub(super) fn reifiable(
        &self,
        id: TlcExprId,
        handle_ops: &FxHashSet<String>,
        fn_set: &FxHashSet<BindingId>,
    ) -> bool {
        if !self.subtree_is_effectful(id, fn_set, handle_ops) {
            return self.no_residual_control(id);
        }
        match &self.module.expr_arena[id] {
            TlcExpr::Perform { op, arg } => {
                handle_ops.contains(op) && self.no_residual_control(*arg)
            }
            TlcExpr::Sequence(items) => {
                items.iter().all(|i| self.reifiable(*i, handle_ops, fn_set))
            }
            TlcExpr::Let { value, body, .. } => {
                self.reifiable(*value, handle_ops, fn_set)
                    && self.reifiable(*body, handle_ops, fn_set)
            }
            TlcExpr::Case(scrut, alts) => {
                self.no_residual_control(*scrut)
                    && alts.iter().all(|a| {
                        a.guard.is_none_or(|g| self.no_residual_control(g))
                            && self.reifiable(a.body, handle_ops, fn_set)
                    })
            }
            TlcExpr::App(..) => self.effectful_call(id, fn_set, handle_ops).is_some(),
            TlcExpr::Builtin(_, l, r) => {
                self.reifiable(*l, handle_ops, fn_set) && self.reifiable(*r, handle_ops, fn_set)
            }
            _ => false,
        }
    }

    /// Whether the subtree contains an effectful-callee reference or control node
    /// (explicit `fn_set`/`handle_ops`, usable before `ctx` during validation).
    pub(super) fn subtree_is_effectful(
        &self,
        id: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. } => {
                    return true;
                }
                TlcExpr::Case(..) if self.node_is_eff_cell_case(cur) => return true,
                // A *call* to an effectful callee is a computation; a bare
                // effectful-function value reference (bound to a local, passed as
                // an argument) is just a closure and stays pure.
                TlcExpr::App(..) if self.effectful_call(cur, fn_set, handle_ops).is_some() => {
                    return true;
                }
                // A `perform` under a `Lam` is *deferred* (it fires only when the
                // closure is called), so a thunk value carrying an effect — e.g. a
                // generator producer `\_. #cons { head = perform … }` passed as an
                // argument — is not an *immediate* effect of this expression. Do
                // not descend into lambda bodies.
                TlcExpr::Lam(..) => continue,
                _ => {}
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        false
    }

    /// If `id` is a saturated `head a1 .. aN` spine whose head is an effectful
    /// callee — a top-level reified fn / higher-order effectful `Var` parameter
    /// (`Some(binding)`) or a record-field projection `box.f` (`None`, no binding) —
    /// and whose arguments are all pure, return `(head_binding, head_node, [args])`.
    pub(super) fn effectful_call(
        &self,
        id: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
        handle_ops: &FxHashSet<String>,
    ) -> Option<(Option<BindingId>, TlcExprId, Vec<TlcExprId>)> {
        let mut args = Vec::new();
        let mut cur = id;
        loop {
            match &self.module.expr_arena[cur] {
                TlcExpr::App(func, arg) => {
                    // Effectful arguments are out of scope (an effectful arg would
                    // need its own bind); leave such calls residual (gated).
                    if self.subtree_is_effectful(*arg, fn_set, handle_ops) {
                        return None;
                    }
                    args.push(*arg);
                    cur = *func;
                }
                TlcExpr::Var(b) if self.node_is_eff_fn_ref(cur, fn_set, handle_ops) => {
                    let b = *b;
                    args.reverse();
                    return Some((Some(b), cur, args));
                }
                TlcExpr::GetField(..) if self.node_is_eff_field_ref(cur, handle_ops) => {
                    args.reverse();
                    return Some((None, cur, args));
                }
                _ => return None,
            }
        }
    }

    /// Whether `id` is a `case`/`match` over a codata cell whose fields were
    /// rewritten to `Computation` values for this reification scope.
    pub(super) fn node_is_eff_cell_case(&self, id: TlcExprId) -> bool {
        let TlcExpr::Case(scrut, _) = self.module.expr_arena[id] else {
            return false;
        };
        self.cell_identity(self.expr_ty(scrut))
            .is_some_and(|cell_id| self.eff_fields.contains_key(&cell_id))
    }

    /// Collect each handled op's argument/resume types from its perform sites.
    pub(super) fn collect_op_meta(
        &self,
        roots: &[TlcExprId],
        handle_ops: &FxHashSet<String>,
    ) -> Option<FxHashMap<String, (TlcTypeId, TlcTypeId)>> {
        let mut meta: FxHashMap<String, (TlcTypeId, TlcTypeId)> = FxHashMap::default();
        for &root in roots {
            let mut seen = FxHashSet::default();
            let mut stack = vec![root];
            while let Some(cur) = stack.pop() {
                if !seen.insert(cur) {
                    continue;
                }
                if let TlcExpr::Perform { op, arg } = &self.module.expr_arena[cur] {
                    if !handle_ops.contains(op) {
                        return None;
                    }
                    let arg_ty = self.expr_ty(*arg);
                    let resume_ty = self.expr_ty(cur);
                    meta.entry(op.clone()).or_insert((arg_ty, resume_ty));
                }
                push_child_exprs(&self.module.expr_arena[cur], &mut stack);
            }
        }
        // Every handled op must have at least one perform site.
        if handle_ops.iter().all(|op| meta.contains_key(op)) {
            Some(meta)
        } else {
            None
        }
    }

    /// Canonical identity of a codata cell type: a named alias reference
    /// `TyVar(Named(id))` *or* a resolved type that is some `TypeAlias`'s body (the
    /// type checker stamps producer cells with the resolved body, but annotations
    /// keep the alias reference — both must map to the same id).
    pub(super) fn cell_identity(&self, ty: TlcTypeId) -> Option<u32> {
        if let TlcType::TyVar(TlcTypeVar::Named(id), _) = &self.module.type_arena[ty] {
            return Some(*id);
        }
        self.module
            .decl_arena
            .iter()
            .find_map(|(_, decl)| match decl {
                TlcDecl::TypeAlias { binding, body, .. } if *body == ty => Some(binding.0),
                _ => None,
            })
    }

    /// Whether the subtree reaches a handled `perform` *without crossing a `Lam`*
    /// (a `perform` under a lambda is deferred, not an immediate field effect).
    pub(super) fn reaches_handled_perform(
        &self,
        id: TlcExprId,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Perform { op, .. } if handle_ops.contains(op) => return true,
                TlcExpr::Lam(..) => continue,
                _ => {}
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        false
    }

    pub(super) fn reaches_non_print_host_perform(
        &self,
        id: TlcExprId,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Perform { op, .. }
                    if handle_ops.contains(op)
                        && crate::HostOp::from_name(op).is_some()
                        && op != "io.print" =>
                {
                    return true;
                }
                TlcExpr::Lam(..) => continue,
                _ => {}
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        false
    }

    /// Detect effectful-codata producers (V3-G4): a `Variant(tag, Record{…})` cell
    /// whose type is a named alias and one of whose strict fields holds a deferred
    /// handled `perform`. Records each `(cell_id, (tag, field))` in `eff_fields`.
    /// Returns `false` on an unsupported shape (a deferred-effect field on an
    /// anonymous cell type, or a non-`io.print` host-resource operation in a cell)
    /// so the handle stays residual and the native backend rejects it precisely.
    pub(super) fn detect_eff_codata(
        &mut self,
        roots: &[TlcExprId],
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        let mut seen = FxHashSet::default();
        let mut stack: Vec<TlcExprId> = roots.to_vec();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if let TlcExpr::Variant(tag, payload) = self.module.expr_arena[cur].clone()
                && let TlcExpr::Record(fields) = self.module.expr_arena[payload].clone()
            {
                let effectful: Vec<String> = fields
                    .iter()
                    .filter(|(_, v)| self.reaches_handled_perform(*v, handle_ops))
                    .map(|(name, _)| name.clone())
                    .collect();
                if !effectful.is_empty() {
                    if fields
                        .iter()
                        .any(|(_, v)| self.reaches_non_print_host_perform(*v, handle_ops))
                    {
                        return false;
                    }
                    let Some(cell_id) = self.cell_identity(self.expr_ty(cur)) else {
                        return false; // deferred effect on an anonymous cell type
                    };
                    let set = self.eff_fields.entry(cell_id).or_default();
                    for field in effectful {
                        set.insert((tag.clone(), field));
                    }
                }
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        true
    }
}
