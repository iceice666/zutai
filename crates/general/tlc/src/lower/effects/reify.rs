//! Reified delimited-continuation lowering — the fallback for handled effects
//! the lexical CPS path (`super::cps`) cannot discharge.
//!
//! The lexical elaborator only handles a `perform` it can statically enclose in
//! its `handle` (after `super::inline` relocates fully-saturated calls). A
//! *recursive* (self/mutual) effectful callee cannot be inlined, so its `perform`
//! survives and the handle stays residual — refused by the residual-effect gate.
//! The interpreter runs these because it reifies the continuation at runtime
//! (`eval_tlc::effects::handle_control`): `perform` suspends into a first-class
//! `EvalControl::Perform { op, arg, cont }`, and the handler drives it.
//!
//! This pass compiles that exact model. For an eligible residual `handle` it:
//!   1. builds a per-scope recursive `Computation` union (a `TypeAlias`, so the
//!      DC equirecursive machinery ties the `resume` back-edge — the same shape a
//!      hand-written free monad lowers through today): `zt__pure { value : R }`
//!      and `zt__op_<op> { payload : ArgTy; resume : ResumeTy -> Computation }`;
//!   2. rewrites every effectful function reachable from the handled expression
//!      into `… -> Computation` monadic form (`perform` → a `zt__op_*` node
//!      whose `resume` field is the continuation; effectful calls compose through
//!      a generated `bind`);
//!   3. generates `bind`/`run` driver decls mirroring `handle_control`
//!      (`zt__pure` → value clause; `zt__op_*` → handler body with `resume X`
//!      rewritten to `run (r X)`);
//!   4. replaces the `handle` with `run (<reified expr>)`.
//!
//! It is conservative: `reify_target` first checks the whole scope is reifiable
//! (monomorphic, closed-row, pure operation arguments, no `finally`), and only
//! commits when it is. Anything else is left intact and refused by the gate —
//! never miscompiled.

use rustc_hash::{FxHashMap, FxHashSet};
use zutai_hir::BindingId;

use crate::ir::{
    Kind, Literal, Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcHandleClause, TlcModule, TlcPat,
    TlcTupleField, TlcType, TlcTypeId, TlcTypeVar,
};
use crate::monomorphize::{push_child_exprs, reachable_exprs};

const PURE_TAG: &str = "zt__pure";

/// A distinct, LLVM-identifier-safe variant tag per operation. Variant tags are
/// stored bare (no `#`) and leak into generated SSA names, so the op name's `.`
/// (e.g. `io.print`) and any other non-alphanumeric byte are mapped to `_`.
fn op_tag(op: &str) -> String {
    let safe: String = op
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("zt__op_{safe}")
}

/// One handled operation's reified shape.
struct OpInfo {
    /// `perform`-argument type (`payload` field).
    arg_ty: TlcTypeId,
    /// `perform`-result / resume-input type (`resume`'s domain).
    resume_ty: TlcTypeId,
    /// Record type of this arm's payload: `{ payload : ArgTy; resume : ResumeTy -> Computation }`.
    payload_ty: TlcTypeId,
    /// Handler clause body for this op (a lambda `\arg. …` possibly using `resume`).
    handler_body: TlcExprId,
}

/// Per-target reify context, valid while transforming one handle.
struct ReifyCtx {
    /// `TyVar(Named(comp_binding))` — the recursive reference to `Computation`.
    comp_ref_ty: TlcTypeId,
    /// Pure carrier type `R` (the `#__zt_pure` `value` field; the common result
    /// of every reified function and of the handled expression).
    carrier_ty: TlcTypeId,
    /// Record type `{ value : R }` of the `#__zt_pure` arm payload.
    pure_payload_ty: TlcTypeId,
    /// Handled operations, by op name.
    ops: FxHashMap<String, OpInfo>,
    /// Effectful functions being rewritten to `Computation` form.
    fn_set: FxHashSet<BindingId>,
    /// Operation names handled by this scope (for recognizing effectful-arrow
    /// variables — e.g. a higher-order effectful function parameter).
    handle_ops: FxHashSet<String>,
    /// `bind` decl binding and its type (`Computation -> (R -> Computation) -> Computation`).
    bind_binding: BindingId,
    bind_ty: TlcTypeId,
    /// `R -> Computation` (the `bind`/join continuation type).
    cont_ty: TlcTypeId,
    /// New `… -> Computation` type of each reified function.
    fn_new_ty: FxHashMap<BindingId, TlcTypeId>,
}

type ReifyK<'k> = Box<dyn FnOnce(&mut Reifier<'_>, TlcExprId) -> TlcExprId + 'k>;

/// A reified function: its binding, peeled value parameters, and computation core.
type ReifyCore = (BindingId, Vec<(BindingId, TlcTypeId)>, TlcExprId);

struct Reifier<'m> {
    module: &'m mut TlcModule,
    used_bindings: FxHashSet<BindingId>,
    next_fresh: u32,
    ctx: Option<ReifyCtx>,
    /// Residual handles we attempted but could not reify; left intact so the
    /// loop terminates and the gate refuses them.
    skipped: FxHashSet<TlcExprId>,
    /// Effectful codata (V3-G4): for each cell alias `TyVar(Named(id))` whose
    /// producer stores a deferred handled `perform` in a strict field, the set of
    /// `(variant_tag, field_name)` carrying that effect. The field's type is
    /// rewritten to `Computation`-data and the consumer `bind`s it. Empty when the
    /// scope has no effectful generator.
    eff_fields: FxHashMap<u32, FxHashSet<(String, String)>>,
    /// Scope-local rewritten cell type `Cell'` per effectful cell alias id (built
    /// in `build_ctx`): `head : Int` → `head : Computation`, `tail : Unit -> Cell`
    /// → `tail : Unit -> Cell'`.
    cell_prime: FxHashMap<u32, TlcTypeId>,
    /// Binders currently bound to a `Computation` value — the head-field binders of
    /// an active `Case` arm over an effectful cell. A use of such a binder is an
    /// effectful value to `bind`. Scoped push/pop per arm.
    comp_binders: FxHashSet<BindingId>,
    /// Lambdas synthesized by `normalize_undersaturated_eff_args` to eta-expand an
    /// inline partially-applied effectful function used as a higher-order argument
    /// (e.g. `applyTo (addP 5)` → `applyTo (\p. addP 5 p)`). Reified specially at the
    /// call site by `maybe_reify_eta_fn_arg`, since `reify` does not descend into
    /// ordinary lambda bodies. Cleared per target.
    eta_fn_args: FxHashSet<TlcExprId>,
}

impl TlcModule {
    /// Reify residual handled effects the lexical CPS path could not discharge.
    pub fn reify_residual_effects(&mut self) {
        let mut reifier = Reifier::new(self);
        // Each top-level residual handle in reachable code is an independent
        // target. Collect them first (reachability is recomputed per attempt
        // because a successful transform rewrites the arena).
        while let Some(handle_id) = reifier.next_residual_handle() {
            if !reifier.reify_target(handle_id) {
                // Mark this handle as visited-but-skipped so the loop terminates;
                // it stays residual and the gate refuses it.
                reifier.skipped.insert(handle_id);
            }
        }
    }
}

impl<'m> Reifier<'m> {
    fn new(module: &'m mut TlcModule) -> Self {
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
        Self {
            module,
            used_bindings,
            next_fresh: u32::MAX,
            ctx: None,
            skipped: FxHashSet::default(),
            eff_fields: FxHashMap::default(),
            cell_prime: FxHashMap::default(),
            comp_binders: FxHashSet::default(),
            eta_fn_args: FxHashSet::default(),
        }
    }

    fn ctx(&self) -> &ReifyCtx {
        self.ctx.as_ref().expect("reify ctx set during a target")
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

    fn alloc_type(&mut self, ty: TlcType) -> TlcTypeId {
        self.module.type_arena.alloc(ty)
    }

    fn fun_ty(&mut self, arg: TlcTypeId, ret: TlcTypeId) -> TlcTypeId {
        self.alloc_type(TlcType::Fun(arg, ret, Row::REmpty))
    }

    /// Allocate an expression node carrying type `ty` (and a default span).
    fn mk(&mut self, expr: TlcExpr, ty: TlcTypeId) -> TlcExprId {
        let id = self.module.expr_arena.alloc(expr);
        self.module.expr_types.insert(id, ty);
        self.module.spans.insert(id, zutai_syntax::Span::default());
        id
    }

    fn expr_ty(&self, id: TlcExprId) -> TlcTypeId {
        self.module.expr_types[&id]
    }

    fn var(&mut self, binding: BindingId, ty: TlcTypeId) -> TlcExprId {
        self.mk(TlcExpr::Var(binding), ty)
    }

    /// Find the next reachable residual `handle` (no `finally`) not yet skipped.
    fn next_residual_handle(&self) -> Option<TlcExprId> {
        reachable_exprs(self.module).into_iter().find(|id| {
            !self.skipped.contains(id)
                && matches!(
                    self.module.expr_arena[*id],
                    TlcExpr::Handle { finally: None, .. }
                )
        })
    }

    /// Attempt to reify one residual handle. Returns `true` if it was committed.
    fn reify_target(&mut self, handle_id: TlcExprId) -> bool {
        let TlcExpr::Handle {
            expr,
            value,
            finally: None,
            ops,
        } = self.module.expr_arena[handle_id].clone()
        else {
            return false;
        };
        let handle_op_names: FxHashSet<String> = ops.iter().map(|c| c.op.clone()).collect();
        if handle_op_names.is_empty() {
            return false;
        }

        // Effectful functions reachable from the handled expression.
        let fn_set = self.effectful_fn_set(expr, &handle_op_names);
        if fn_set.is_empty() {
            // No cross-function effect to reify; the lexical path already had its
            // chance. Leave residual.
            return false;
        }

        // Wrapper bindings (e.g. a record holding an effectful function as a field)
        // that must have their declared types rewritten alongside the callees.
        let wrapper_set = self.wrapper_set(handle_id, &fn_set);

        // Every reified function and wrapper must be referenced only from within
        // this scope (the handle subtree or another reified/wrapper body), else
        // rewriting its type would break an unrelated use.
        if self.fn_escapes_scope(handle_id, &fn_set, &wrapper_set) {
            return false;
        }

        // The value clause and handler bodies must be pure apart from `resume`.
        if let Some(vc) = value
            && !self.no_residual_control(vc)
        {
            return false;
        }
        for clause in &ops {
            if !self.handler_body_ok(clause.body, &fn_set) {
                return false;
            }
        }

        // Eta-normalize inline partial applications of effectful functions used as
        // higher-order arguments (e.g. `applyTo (addP 5)`) into lambda *values*, so
        // the existing higher-order-value path reifies them. The named form
        // (`applyTo addP`) already works; this brings the inline form to parity.
        self.eta_fn_args.clear();
        let mut norm_roots = vec![expr];
        for &f in &fn_set {
            if let Some((_, body)) = self.decl_of(f) {
                norm_roots.push(body);
            }
        }
        self.normalize_undersaturated_eff_args(&norm_roots, &fn_set, &handle_op_names);

        // Effectful generators (V3-G4): find cell producers that store a deferred
        // `perform` in a strict field. Populates `self.eff_fields`; bails on an
        // unsupported shape (e.g. an anonymous cell type).
        self.eff_fields.clear();
        self.cell_prime.clear();
        self.comp_binders.clear();
        let mut gen_roots = vec![expr];
        for &f in &fn_set {
            if let Some((_, body)) = self.decl_of(f) {
                gen_roots.push(body);
            }
        }
        if !self.detect_eff_codata(&gen_roots, &handle_op_names) {
            return false;
        }

        // The handled expression and every reified function body must be reifiable.
        let mut cores: Vec<ReifyCore> = Vec::new();
        for &f in &fn_set {
            let Some((decl_ty, body)) = self.decl_of(f) else {
                return false;
            };
            // Eta-expand a partially-applied effectful body (e.g. `addP = add 10`,
            // which has no leading lambdas) up to its full value arity, so peeling
            // yields the parameters and a saturated computation core.
            let (params, core) = self.peel_or_eta(decl_ty, body);
            if params.is_empty() || !self.reifiable(core, &handle_op_names, &fn_set) {
                return false;
            }
            cores.push((f, params, core));
        }
        if !self.reifiable(expr, &handle_op_names, &fn_set) {
            return false;
        }

        // Operation metadata from perform sites (closed, monomorphic).
        let mut roots = vec![expr];
        roots.extend(cores.iter().map(|(_, _, core)| *core));
        let Some(op_meta) = self.collect_op_meta(&roots, &handle_op_names) else {
            return false;
        };

        // ── Commit ──────────────────────────────────────────────────────────
        let carrier_ty = self.expr_ty(expr);
        let handle_result_ty = self.expr_ty(handle_id);
        self.build_ctx(
            &ops,
            op_meta,
            fn_set.clone(),
            handle_op_names.clone(),
            carrier_ty,
        );

        // New `… -> Computation` types for every reified function (effectful
        // parameter types are rewritten to their monadic form too).
        let decl_tys: Vec<(BindingId, TlcTypeId)> = cores
            .iter()
            .filter_map(|(f, _, _)| self.decl_of(*f).map(|(ty, _)| (*f, ty)))
            .collect();
        let mut fn_new_ty: FxHashMap<BindingId, TlcTypeId> = FxHashMap::default();
        for (f, ty) in decl_tys {
            let new_ty = self.monadic_ty(ty);
            fn_new_ty.insert(f, new_ty);
        }
        self.ctx.as_mut().unwrap().fn_new_ty = fn_new_ty;

        // Rewrite each reified function body to Computation-returning form,
        // rewriting every value parameter's type to its monadic form.
        for (f, params, core) in cores {
            let core_c = self.reify(core, carrier_ty, Box::new(|this, v| this.make_pure(v)));
            let mut acc = core_c;
            let comp = self.ctx().comp_ref_ty;
            let mut acc_ty = comp;
            for (param, pty) in params.iter().rev() {
                let pty2 = self.monadic_ty(*pty);
                let lam_ty = self.fun_ty(pty2, acc_ty);
                acc = self.mk(TlcExpr::Lam(*param, pty2, acc), lam_ty);
                acc_ty = lam_ty;
            }
            let new_ty = self.ctx().fn_new_ty[&f];
            self.set_decl(f, new_ty, acc);
        }

        // Rewrite each wrapper binding's declared type to monadic form (e.g. a record
        // field `{ f : Int -> Int ! {op} }` → `{ f : Int -> Computation }`) so the
        // projected field's type matches the reified callee and the gate sees a pure
        // record. The body is unchanged — its nodes are restamped below.
        for &w in &wrapper_set {
            if let Some((wty, wbody)) = self.decl_of(w) {
                let new_wty = self.monadic_ty(wty);
                if new_wty != wty {
                    self.set_decl(w, new_wty, wbody);
                }
            }
        }

        // Generate the bind and run drivers, then splice the handle.
        self.emit_bind_decl();
        let run_binding = self.emit_run_decl(value, &ops, handle_result_ty);
        let comp_expr = self.reify(expr, carrier_ty, Box::new(|this, v| this.make_pure(v)));
        let run_ty = self.fun_ty(self.ctx().comp_ref_ty, handle_result_ty);
        let run_var = self.var(run_binding, run_ty);
        self.module.expr_arena[handle_id] = TlcExpr::App(run_var, comp_expr);
        self.module.expr_types.insert(handle_id, handle_result_ty);

        // Reused value nodes (an effectful function bound to a local or passed as
        // an argument) keep their stale effectful-arrow type; restamp every such
        // node in this scope to its monadic form so the gate sees only pure types.
        let mut roots = vec![handle_id];
        for &f in &fn_set {
            if let Some((_, body)) = self.decl_of(f) {
                roots.push(body);
            }
        }
        for &w in &wrapper_set {
            if let Some((_, body)) = self.decl_of(w) {
                roots.push(body);
            }
        }
        self.restamp_effectful_types(&roots);

        self.ctx = None;
        true
    }

    /// Restamp every node reachable from `roots` whose recorded type mentions a
    /// handled effectful arrow (including nested in tuples/records/unions) to its
    /// monadic (`Computation`-returning) form, so the residual-effect gate sees
    /// only pure types in this scope.
    fn restamp_effectful_types(&mut self, roots: &[TlcExprId]) {
        let mut seen = FxHashSet::default();
        let mut stack: Vec<TlcExprId> = roots.to_vec();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if let Some(&ty) = self.module.expr_types.get(&cur) {
                let new_ty = self.monadic_ty(ty);
                if new_ty != ty {
                    self.module.expr_types.insert(cur, new_ty);
                }
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
    }
}

// ── Analysis & validation ──────────────────────────────────────────────────────

impl<'m> Reifier<'m> {
    /// Look up a top-level `Value` decl by binding, returning `(ty, body)`.
    fn decl_of(&self, binding: BindingId) -> Option<(TlcTypeId, TlcExprId)> {
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
    fn set_decl(&mut self, binding: BindingId, ty: TlcTypeId, body: TlcExprId) {
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
    fn var_refs(&self, root: TlcExprId) -> FxHashSet<BindingId> {
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
    fn fun_spine_has_effect(&self, ty: TlcTypeId) -> bool {
        match &self.module.type_arena[ty] {
            TlcType::Fun(_, ret, row) => {
                !matches!(row, Row::REmpty) || self.fun_spine_has_effect(*ret)
            }
            _ => false,
        }
    }

    /// The operation names appearing in `ty`'s curried-spine effect rows.
    fn effect_ops_of(&self, ty: TlcTypeId, out: &mut FxHashSet<String>) {
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
    fn effectful_fn_set(
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
    fn wrapper_set(
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
    fn fn_escapes_scope(
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
    fn refs_fn_outside(
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
    fn is_effectful(&self, id: TlcExprId) -> bool {
        if self.subtree_has_comp_binder(id) {
            return true;
        }
        let fn_set = &self.ctx().fn_set;
        let handle_ops = &self.ctx().handle_ops;
        self.subtree_is_effectful(id, fn_set, handle_ops)
    }

    /// Whether the subtree contains no `Perform`/`Handle`/`Resume` node (a pure
    /// computation safe to reuse directly as a value).
    fn no_residual_control(&self, id: TlcExprId) -> bool {
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
    fn handler_body_ok(&self, id: TlcExprId, fn_set: &FxHashSet<BindingId>) -> bool {
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
    fn peel_lams(&self, body: TlcExprId) -> (Vec<(BindingId, TlcTypeId)>, TlcExprId) {
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
    fn arrow_value_spine(&self, ty: TlcTypeId) -> Vec<(TlcTypeId, TlcTypeId)> {
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
    fn peel_or_eta(
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
    fn normalize_undersaturated_eff_args(
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
    fn is_undersaturated_eff_value(
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
    fn eta_expand_in_place(&mut self, n: TlcExprId) {
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
    fn reifiable(
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
    fn subtree_is_effectful(
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
    fn effectful_call(
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

    /// Collect each handled op's argument/resume types from its perform sites.
    fn collect_op_meta(
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
    fn cell_identity(&self, ty: TlcTypeId) -> Option<u32> {
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
    fn reaches_handled_perform(&self, id: TlcExprId, handle_ops: &FxHashSet<String>) -> bool {
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

    /// Detect effectful-codata producers (V3-G4): a `Variant(tag, Record{…})` cell
    /// whose type is a named alias and one of whose strict fields holds a deferred
    /// handled `perform`. Records each `(cell_id, (tag, field))` in `eff_fields`.
    /// Returns `false` on an unsupported shape (a deferred-effect field on an
    /// anonymous cell type) so the handle stays residual.
    fn detect_eff_codata(&mut self, roots: &[TlcExprId], handle_ops: &FxHashSet<String>) -> bool {
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

// ── Construction: Computation type, drivers, and the reify transform ───────────

impl<'m> Reifier<'m> {
    /// Build the recursive `Computation` type alias and the per-target context.
    fn build_ctx(
        &mut self,
        ops_clauses: &[TlcHandleClause],
        op_meta: FxHashMap<String, (TlcTypeId, TlcTypeId)>,
        fn_set: FxHashSet<BindingId>,
        handle_ops: FxHashSet<String>,
        carrier_ty: TlcTypeId,
    ) {
        let comp_binding = self.fresh_binding();
        let comp_ref_ty = self.alloc_type(TlcType::TyVar(
            TlcTypeVar::Named(comp_binding.0),
            Kind::ground(),
        ));
        let pure_payload_ty = self.alloc_type(TlcType::Record(Row::from_record_fields([(
            "value".to_string(),
            carrier_ty,
            false,
        )])));

        let mut ops_map: FxHashMap<String, OpInfo> = FxHashMap::default();
        let mut variant_fields: Vec<(String, TlcTypeId)> =
            vec![(PURE_TAG.to_string(), pure_payload_ty)];
        for clause in ops_clauses {
            let (arg_ty, resume_ty) = op_meta[&clause.op];
            let resume_fn = self.fun_ty(resume_ty, comp_ref_ty);
            let payload_ty = self.alloc_type(TlcType::Record(Row::from_record_fields([
                ("payload".to_string(), arg_ty, false),
                ("resume".to_string(), resume_fn, false),
            ])));
            variant_fields.push((op_tag(&clause.op), payload_ty));
            ops_map.insert(
                clause.op.clone(),
                OpInfo {
                    arg_ty,
                    resume_ty,
                    payload_ty,
                    handler_body: clause.body,
                },
            );
        }
        let variant_ty = self.alloc_type(TlcType::VariantT(Row::from_fields(variant_fields)));
        let alias = self.module.decl_arena.alloc(TlcDecl::TypeAlias {
            binding: comp_binding,
            params: vec![],
            body: variant_ty,
        });
        self.module.decls.push(alias);

        // Effectful-codata (V3-G4): build a scope-local `Cell'` per effectful cell,
        // rewriting each effectful field to `Computation`-data and the recursive
        // `tail` to `Unit -> Cell'`.
        self.build_cell_primes(comp_ref_ty);

        let cont_ty = self.fun_ty(carrier_ty, comp_ref_ty);
        let bind_inner = self.fun_ty(cont_ty, comp_ref_ty);
        let bind_ty = self.fun_ty(comp_ref_ty, bind_inner);
        let bind_binding = self.fresh_binding();

        self.ctx = Some(ReifyCtx {
            comp_ref_ty,
            carrier_ty,
            pure_payload_ty,
            ops: ops_map,
            fn_set,
            handle_ops,
            bind_binding,
            bind_ty,
            cont_ty,
            fn_new_ty: FxHashMap::default(),
        });
    }

    /// The body type of the `TypeAlias` whose binding id is `cell_id`.
    fn alias_body(&self, cell_id: u32) -> Option<TlcTypeId> {
        self.module
            .decl_arena
            .iter()
            .find_map(|(_, decl)| match decl {
                TlcDecl::TypeAlias { binding, body, .. } if binding.0 == cell_id => Some(*body),
                _ => None,
            })
    }

    /// `(name, ty, optional)` of each field of a record row.
    fn record_fields_of(&self, row: &Row) -> Vec<(String, TlcTypeId, bool)> {
        let mut out = Vec::new();
        let mut r = row;
        while let Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } = r
        {
            out.push((label.clone(), *ty, *optional));
            r = tail;
        }
        out
    }

    /// Whether `ty` is a demand thunk `_ -> Cell` for the given effectful cell.
    fn is_demand_thunk_of_cell(&self, ty: TlcTypeId, cell_id: u32) -> bool {
        matches!(&self.module.type_arena[ty], TlcType::Fun(_, b, _)
            if self.cell_identity(*b) == Some(cell_id))
    }

    /// Build a scope-local `Cell'` alias per effectful cell: effectful fields →
    /// `Computation`-data, recursive `tail` → `Unit -> Cell'`. The fresh alias
    /// reference is registered in `cell_prime` *before* its body is built so the
    /// recursive `tail` back-edge ties the knot.
    fn build_cell_primes(&mut self, comp_ref: TlcTypeId) {
        let cells: Vec<(u32, FxHashSet<(String, String)>)> = self
            .eff_fields
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let mut new_binding: FxHashMap<u32, BindingId> = FxHashMap::default();
        for (cell_id, _) in &cells {
            let b = self.fresh_binding();
            let r = self.alloc_type(TlcType::TyVar(TlcTypeVar::Named(b.0), Kind::ground()));
            new_binding.insert(*cell_id, b);
            self.cell_prime.insert(*cell_id, r);
        }
        for (cell_id, eff_set) in &cells {
            let Some(body_ty) = self.alias_body(*cell_id) else {
                continue;
            };
            let TlcType::VariantT(row) = self.module.type_arena[body_ty].clone() else {
                continue;
            };
            let cell_prime_ref = self.cell_prime[cell_id];
            let arms: Vec<(String, TlcTypeId)> =
                row.fields().map(|(t, ty)| (t.to_string(), ty)).collect();
            let mut new_arms: Vec<(String, TlcTypeId)> = Vec::new();
            for (tag, payload_ty) in arms {
                let new_payload = match self.module.type_arena[payload_ty].clone() {
                    TlcType::Record(prow) => {
                        let fields = self.record_fields_of(&prow);
                        let new_fields: Vec<(String, TlcTypeId, bool)> = fields
                            .into_iter()
                            .map(|(name, fty, opt)| {
                                let nty = if eff_set.contains(&(tag.clone(), name.clone())) {
                                    comp_ref
                                } else if self.is_demand_thunk_of_cell(fty, *cell_id) {
                                    match self.module.type_arena[fty].clone() {
                                        TlcType::Fun(a, _, _) => self.fun_ty(a, cell_prime_ref),
                                        _ => fty,
                                    }
                                } else {
                                    fty
                                };
                                (name, nty, opt)
                            })
                            .collect();
                        self.alloc_type(TlcType::Record(Row::from_record_fields(new_fields)))
                    }
                    _ => payload_ty,
                };
                new_arms.push((tag, new_payload));
            }
            let new_variant = self.alloc_type(TlcType::VariantT(Row::from_fields(new_arms)));
            let alias = self.module.decl_arena.alloc(TlcDecl::TypeAlias {
                binding: new_binding[cell_id],
                params: vec![],
                body: new_variant,
            });
            self.module.decls.push(alias);
        }
    }

    /// Whether `ty` is an arrow whose curried spine carries only handled effects
    /// (so the function it types is reified to `Computation` form).
    fn arrow_is_handled(&self, ty: TlcTypeId) -> bool {
        if !self.fun_spine_has_effect(ty) {
            return false;
        }
        let mut ops = FxHashSet::default();
        self.effect_ops_of(ty, &mut ops);
        ops.is_subset(&self.ctx().handle_ops)
    }

    /// Rewrite a type to its `Computation`-returning monadic form: any curried
    /// arrow whose spine carries a handled effect has that effectful result
    /// replaced by the (single, closed) `Computation` type and its rows cleared.
    /// Recurses into composite types (tuples, records, lists, unions) so an
    /// effectful arrow nested in, e.g., a tupled multi-parameter scrutinee is
    /// rewritten too. Returns `ty` unchanged when nothing needs rewriting.
    fn monadic_ty(&mut self, ty: TlcTypeId) -> TlcTypeId {
        let comp = self.ctx().comp_ref_ty;
        // An effectful codata type `Cell` (alias ref or resolved body) → `Cell'`.
        if let Some(cell_id) = self.cell_identity(ty)
            && let Some(&cprime) = self.cell_prime.get(&cell_id)
        {
            return cprime;
        }
        match self.module.type_arena[ty].clone() {
            TlcType::Fun(a, b, row) => {
                let a2 = self.monadic_ty(a);
                if self.arrow_is_handled(ty) {
                    if !matches!(row, Row::REmpty) {
                        // This arrow carries the handled effect; result → Computation.
                        self.fun_ty(a2, comp)
                    } else {
                        let b2 = self.monadic_ty(b);
                        self.fun_ty(a2, b2)
                    }
                } else {
                    let b2 = self.monadic_ty(b);
                    if a2 == a && b2 == b {
                        ty
                    } else {
                        self.alloc_type(TlcType::Fun(a2, b2, row))
                    }
                }
            }
            TlcType::Tuple(fields) => {
                let mut changed = false;
                let new_fields: Vec<TlcTupleField> = fields
                    .into_iter()
                    .map(|f| match f {
                        TlcTupleField::Named { name, ty } => {
                            let ty2 = self.monadic_ty(ty);
                            changed |= ty2 != ty;
                            TlcTupleField::Named { name, ty: ty2 }
                        }
                        TlcTupleField::Positional(ty) => {
                            let ty2 = self.monadic_ty(ty);
                            changed |= ty2 != ty;
                            TlcTupleField::Positional(ty2)
                        }
                    })
                    .collect();
                if changed {
                    self.alloc_type(TlcType::Tuple(new_fields))
                } else {
                    ty
                }
            }
            TlcType::List(inner) => {
                let i2 = self.monadic_ty(inner);
                if i2 == inner {
                    ty
                } else {
                    self.alloc_type(TlcType::List(i2))
                }
            }
            TlcType::Optional(inner) => {
                let i2 = self.monadic_ty(inner);
                if i2 == inner {
                    ty
                } else {
                    self.alloc_type(TlcType::Optional(i2))
                }
            }
            TlcType::Maybe(inner) => {
                let i2 = self.monadic_ty(inner);
                if i2 == inner {
                    ty
                } else {
                    self.alloc_type(TlcType::Maybe(i2))
                }
            }
            TlcType::Record(row) => {
                let (row2, changed) = self.monadic_row(&row);
                if changed {
                    self.alloc_type(TlcType::Record(row2))
                } else {
                    ty
                }
            }
            TlcType::VariantT(row) => {
                let (row2, changed) = self.monadic_row(&row);
                if changed {
                    self.alloc_type(TlcType::VariantT(row2))
                } else {
                    ty
                }
            }
            _ => ty,
        }
    }

    /// Rewrite each field type of a row via `monadic_ty`; report whether changed.
    fn monadic_row(&mut self, row: &Row) -> (Row, bool) {
        match row {
            Row::REmpty | Row::RVar(_) => (row.clone(), false),
            Row::RExtend {
                label,
                ty,
                optional,
                tail,
            } => {
                let ty2 = self.monadic_ty(*ty);
                let (tail2, tail_changed) = self.monadic_row(tail);
                let changed = ty2 != *ty || tail_changed;
                (
                    Row::RExtend {
                        label: label.clone(),
                        ty: ty2,
                        optional: *optional,
                        tail: Box::new(tail2),
                    },
                    changed,
                )
            }
        }
    }

    /// Whether `node` references an effectful callee to reify: a `Var` that is
    /// either a top-level reified function (`fn_set`) or a value whose recorded
    /// type is a function arrow carrying only handled effects (a higher-order
    /// effectful parameter, e.g. `f` in `apply f = f 1`).
    fn node_is_eff_fn_ref(
        &self,
        node: TlcExprId,
        fn_set: &FxHashSet<BindingId>,
        handle_ops: &FxHashSet<String>,
    ) -> bool {
        let TlcExpr::Var(b) = &self.module.expr_arena[node] else {
            return false;
        };
        if fn_set.contains(b) {
            return true;
        }
        let Some(&ty) = self.module.expr_types.get(&node) else {
            return false;
        };
        if !self.fun_spine_has_effect(ty) {
            return false;
        }
        let mut ops = FxHashSet::default();
        self.effect_ops_of(ty, &mut ops);
        ops.is_subset(handle_ops)
    }

    /// Whether `node` is a record-field projection `box.f` yielding an effectful
    /// function value whose operations are all handled — the projection analogue of
    /// `node_is_eff_fn_ref` (which only recognizes `Var` heads). The underlying
    /// field value is reified to `Computation` form via the wrapper-binding rewrite,
    /// so the call site treats `box.f` as an opaque monadic callee.
    fn node_is_eff_field_ref(&self, node: TlcExprId, handle_ops: &FxHashSet<String>) -> bool {
        if !matches!(self.module.expr_arena[node], TlcExpr::GetField(..)) {
            return false;
        }
        let Some(&ty) = self.module.expr_types.get(&node) else {
            return false;
        };
        if !self.fun_spine_has_effect(ty) {
            return false;
        }
        let mut ops = FxHashSet::default();
        self.effect_ops_of(ty, &mut ops);
        ops.is_subset(handle_ops)
    }

    /// `#__zt_pure { value = v }`.
    fn make_pure(&mut self, v: TlcExprId) -> TlcExprId {
        let pure_payload_ty = self.ctx().pure_payload_ty;
        let comp = self.ctx().comp_ref_ty;
        let rec = self.mk(
            TlcExpr::Record(vec![("value".to_string(), v)]),
            pure_payload_ty,
        );
        self.mk(TlcExpr::Variant(PURE_TAG.to_string(), rec), comp)
    }

    /// Transform a computation expression to a `Computation` value, threading the
    /// pure continuation `k`. `val_ty` is the pure type of the value this
    /// computation yields (recorded `expr_ty` is unreliable — a multi-clause
    /// function body lowers to a `Case` typed as the whole *function* type).
    /// `bind m (\jp. k jp)` — compose a `Computation` value `m` with continuation
    /// `k`, both at the scope carrier.
    fn bind_m(&mut self, m: TlcExprId, k: ReifyK) -> TlcExprId {
        let comp = self.ctx().comp_ref_ty;
        let carrier = self.ctx().carrier_ty;
        let cont_ty = self.ctx().cont_ty;
        let bind_binding = self.ctx().bind_binding;
        let bind_ty = self.ctx().bind_ty;
        let jp = self.fresh_binding();
        let jp_var = self.var(jp, carrier);
        let join_body = k(self, jp_var);
        let join_lam = self.mk(TlcExpr::Lam(jp, carrier, join_body), cont_ty);
        let bind_var = self.var(bind_binding, bind_ty);
        let bind_inner_ty = self.fun_ty(cont_ty, comp);
        let app1 = self.mk(TlcExpr::App(bind_var, m), bind_inner_ty);
        self.mk(TlcExpr::App(app1, join_lam), comp)
    }

    /// For a `Case` arm pattern over an effectful cell, add each effectful-field
    /// binder to `comp_binders` and return them (to remove after the arm). The
    /// pattern is `Variant(tag, Record[(field, Bind(b))…])`.
    fn mark_arm_comp_binders(&mut self, pat: &TlcPat, cell_id: u32) -> Vec<BindingId> {
        let TlcPat::Variant(tag, inner) = pat else {
            return Vec::new();
        };
        let TlcPat::Record(field_pats) = inner.as_ref() else {
            return Vec::new();
        };
        let eff = self.eff_fields.get(&cell_id).cloned().unwrap_or_default();
        let mut marked = Vec::new();
        for (field, fpat) in field_pats {
            if let TlcPat::Bind(b) = fpat
                && eff.contains(&(tag.clone(), field.clone()))
            {
                self.comp_binders.insert(*b);
                marked.push(*b);
            }
        }
        marked
    }

    /// Whether `id` is a use of a binder bound to a `Computation` value (a
    /// head-field binder of an effectful cell), already in `bind`-able form.
    fn node_is_comp_value(&self, id: TlcExprId) -> bool {
        matches!(&self.module.expr_arena[id], TlcExpr::Var(b) if self.comp_binders.contains(b))
    }

    /// Whether the subtree (not descending into lambdas) uses a `Computation`-valued
    /// binder.
    fn subtree_has_comp_binder(&self, id: TlcExprId) -> bool {
        if self.comp_binders.is_empty() {
            return false;
        }
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            match &self.module.expr_arena[cur] {
                TlcExpr::Var(b) if self.comp_binders.contains(b) => return true,
                TlcExpr::Lam(..) => continue,
                _ => {}
            }
            push_child_exprs(&self.module.expr_arena[cur], &mut stack);
        }
        false
    }

    /// If `ty` is a demand thunk `_ -> Cell` for some effectful cell, that cell's id.
    fn demand_thunk_cell(&self, ty: TlcTypeId) -> Option<u32> {
        if let TlcType::Fun(_, b, _) = &self.module.type_arena[ty]
            && let Some(id) = self.cell_identity(*b)
            && self.eff_fields.contains_key(&id)
        {
            return Some(id);
        }
        None
    }

    /// Reify a demand-thunk value `\_. <cell>` into `\_. <Cell'-form cell>`.
    fn reify_thunk(&mut self, thunk_id: TlcExprId) -> TlcExprId {
        let TlcExpr::Lam(p, ty, body) = self.module.expr_arena[thunk_id].clone() else {
            return thunk_id;
        };
        let new_body = self.reify_cell_body(body);
        let p_ty = self.monadic_ty(ty);
        let body_ty = self.expr_ty(new_body);
        let lam_ty = self.fun_ty(p_ty, body_ty);
        self.mk(TlcExpr::Lam(p, p_ty, new_body), lam_ty)
    }

    /// Reify a cell-constructing expression `#cons { head = perform …; tail = … }`
    /// into `Cell'` form: an effectful field's `perform` becomes `Computation`-data,
    /// a recursive `tail` thunk is reified, pure fields pass through.
    fn reify_cell_body(&mut self, id: TlcExprId) -> TlcExprId {
        let TlcExpr::Variant(tag, payload) = self.module.expr_arena[id].clone() else {
            return id;
        };
        let Some(cell_id) = self.cell_identity(self.expr_ty(id)) else {
            return id;
        };
        let Some(&cprime) = self.cell_prime.get(&cell_id) else {
            return id;
        };
        let TlcExpr::Record(fields) = self.module.expr_arena[payload].clone() else {
            // Payload-less arm (e.g. `#nil`): just retype to `Cell'`.
            return self.mk(TlcExpr::Variant(tag, payload), cprime);
        };
        let eff = self.eff_fields[&cell_id].clone();
        let mut new_fields: Vec<(String, TlcExprId)> = Vec::new();
        let mut field_tys: Vec<(String, TlcTypeId, bool)> = Vec::new();
        for (name, val) in fields {
            let nv = if eff.contains(&(tag.clone(), name.clone())) {
                let vty = self.expr_ty(val);
                self.reify(val, vty, Box::new(|this, v| this.make_pure(v)))
            } else if self.demand_thunk_cell(self.expr_ty(val)).is_some() {
                self.reify_thunk(val)
            } else {
                val
            };
            field_tys.push((name.clone(), self.expr_ty(nv), false));
            new_fields.push((name, nv));
        }
        let payload_ty = self.alloc_type(TlcType::Record(Row::from_record_fields(field_tys)));
        let new_payload = self.mk(TlcExpr::Record(new_fields), payload_ty);
        self.mk(TlcExpr::Variant(tag, new_payload), cprime)
    }

    /// If `arg` is a demand-thunk value for an effectful cell, reify its body;
    /// otherwise leave it.
    fn maybe_reify_thunk_arg(&mut self, arg: TlcExprId) -> TlcExprId {
        if self.demand_thunk_cell(self.expr_ty(arg)).is_some() {
            self.reify_thunk(arg)
        } else {
            arg
        }
    }

    /// If `arg` is an eta-expanded partial-application lambda synthesized by
    /// `normalize_undersaturated_eff_args`, reify its (saturated effectful) body to
    /// `Computation` form and rebuild the lambda with monadic parameter/result
    /// types — `reify` itself never descends into ordinary lambda bodies. Mirrors
    /// the reified-function-body commit loop.
    fn maybe_reify_eta_fn_arg(&mut self, arg: TlcExprId) -> TlcExprId {
        if !self.eta_fn_args.contains(&arg) {
            return arg;
        }
        let (params, core) = self.peel_lams(arg);
        let core_val_ty = self.expr_ty(core);
        let mut acc = self.reify(core, core_val_ty, Box::new(|this, v| this.make_pure(v)));
        let mut acc_ty = self.ctx().comp_ref_ty;
        for (param, pty) in params.iter().rev() {
            let pty2 = self.monadic_ty(*pty);
            let lam_ty = self.fun_ty(pty2, acc_ty);
            acc = self.mk(TlcExpr::Lam(*param, pty2, acc), lam_ty);
            acc_ty = lam_ty;
        }
        acc
    }

    fn reify(&mut self, id: TlcExprId, val_ty: TlcTypeId, k: ReifyK) -> TlcExprId {
        if !self.is_effectful(id) {
            return k(self, id);
        }
        // A `Computation`-valued binder (an effectful cell's head field) is already
        // in monadic form; `bind` it.
        if self.node_is_comp_value(id) {
            return self.bind_m(id, k);
        }
        match self.module.expr_arena[id].clone() {
            TlcExpr::Perform { op, arg } => {
                let (resume_ty, payload_ty) = {
                    let oi = &self.ctx().ops[&op];
                    (oi.resume_ty, oi.payload_ty)
                };
                let comp = self.ctx().comp_ref_ty;
                let r = self.fresh_binding();
                let r_var = self.var(r, resume_ty);
                let resume_body = k(self, r_var);
                let resume_fn_ty = self.fun_ty(resume_ty, comp);
                let resume_lam = self.mk(TlcExpr::Lam(r, resume_ty, resume_body), resume_fn_ty);
                let rec = self.mk(
                    TlcExpr::Record(vec![
                        ("payload".to_string(), arg),
                        ("resume".to_string(), resume_lam),
                    ]),
                    payload_ty,
                );
                self.mk(TlcExpr::Variant(op_tag(&op), rec), comp)
            }
            TlcExpr::Sequence(items) => self.reify_sequence(items, val_ty, k),
            TlcExpr::Let {
                binding,
                ty,
                value,
                body,
            } => {
                let comp = self.ctx().comp_ref_ty;
                if self.is_effectful(value) {
                    self.reify(
                        value,
                        ty,
                        Box::new(move |this, vv| {
                            let body_c = this.reify(body, val_ty, k);
                            this.mk(
                                TlcExpr::Let {
                                    binding,
                                    ty,
                                    value: vv,
                                    body: body_c,
                                },
                                comp,
                            )
                        }),
                    )
                } else {
                    // A pure value may be an effectful-function *value* (e.g. a
                    // local bound to a reified callee); rewrite the binder type to
                    // its monadic form so later calls to it see `… -> Computation`.
                    let ty2 = self.monadic_ty(ty);
                    let body_c = self.reify(body, val_ty, k);
                    self.mk(
                        TlcExpr::Let {
                            binding,
                            ty: ty2,
                            value,
                            body: body_c,
                        },
                        comp,
                    )
                }
            }
            TlcExpr::Case(scrut, alts) => {
                let comp = self.ctx().comp_ref_ty;
                let case_val_ty = val_ty;
                let jp = self.fresh_binding();
                let jp_var = self.var(jp, case_val_ty);
                let join_body = k(self, jp_var);
                let join_ty = self.fun_ty(case_val_ty, comp);
                let join_lam = self.mk(TlcExpr::Lam(jp, case_val_ty, join_body), join_ty);
                let join_binding = self.fresh_binding();
                // If matching an effectful cell, each arm's head-field binders hold
                // `Computation` values (V3-G4); mark them while reifying that arm.
                let scrut_cell = self
                    .cell_identity(self.expr_ty(scrut))
                    .filter(|id| self.eff_fields.contains_key(id));
                let new_alts: Vec<TlcAlt> = alts
                    .into_iter()
                    .map(|alt| {
                        let marked = match scrut_cell {
                            Some(cell_id) => self.mark_arm_comp_binders(&alt.pat, cell_id),
                            None => Vec::new(),
                        };
                        let body = self.reify(
                            alt.body,
                            val_ty,
                            Box::new(move |this, av| {
                                let jv = this.var(join_binding, join_ty);
                                this.mk(TlcExpr::App(jv, av), comp)
                            }),
                        );
                        for b in marked {
                            self.comp_binders.remove(&b);
                        }
                        TlcAlt {
                            pat: alt.pat,
                            guard: alt.guard,
                            body,
                        }
                    })
                    .collect();
                let new_case = self.mk(TlcExpr::Case(scrut, new_alts), comp);
                self.mk(
                    TlcExpr::Let {
                        binding: join_binding,
                        ty: join_ty,
                        value: join_lam,
                        body: new_case,
                    },
                    comp,
                )
            }
            TlcExpr::Builtin(op, l, r) => {
                // Left-to-right: reify the left operand, then the right, then apply
                // the builtin in the continuation. Pure operands pass straight
                // through `reify`; an effectful operand (e.g. `n + f (n - 1)`)
                // composes through `bind`.
                let lty = self.expr_ty(l);
                let rty = self.expr_ty(r);
                let bty = self.expr_ty(id);
                self.reify(
                    l,
                    lty,
                    Box::new(move |this, lv| {
                        this.reify(
                            r,
                            rty,
                            Box::new(move |this, rv| {
                                let b = this.mk(TlcExpr::Builtin(op, lv, rv), bty);
                                k(this, b)
                            }),
                        )
                    }),
                )
            }
            TlcExpr::App(..) => {
                let fn_set = self.ctx().fn_set.clone();
                let handle_ops = self.ctx().handle_ops.clone();
                let (f_opt, head_node, args) = self
                    .effectful_call(id, &fn_set, &handle_ops)
                    .expect("validated reifiable");
                // Top-level reified fns have a precomputed monadic type; a
                // higher-order effectful parameter or a record-field projection
                // (`box.f`) derives its monadic type from the head node's recorded
                // (effectful-arrow) type.
                let new_f_ty = match f_opt.and_then(|b| self.ctx().fn_new_ty.get(&b).copied()) {
                    Some(ty) => ty,
                    None => {
                        let head_ty = self.expr_ty(head_node);
                        self.monadic_ty(head_ty)
                    }
                };
                // A `Var` head (reified fn or higher-order param) rebuilds as a typed
                // var; a `GetField` head (`box.f`) rebuilds the projection retyped to
                // its monadic form.
                let mut cur = match f_opt {
                    Some(b) => self.var(b, new_f_ty),
                    None => {
                        let node = self.module.expr_arena[head_node].clone();
                        self.mk(node, new_f_ty)
                    }
                };
                let mut cur_ty = new_f_ty;
                for arg in args {
                    // A demand-thunk argument carrying an effectful generator (e.g.
                    // `stream {…}` passed to a consumer) is reified into `Cell'` form.
                    let arg = self.maybe_reify_thunk_arg(arg);
                    // An eta-expanded partial-application lambda argument has its
                    // saturated effectful body reified to monadic form here.
                    let arg = self.maybe_reify_eta_fn_arg(arg);
                    let ret_ty = match &self.module.type_arena[cur_ty] {
                        TlcType::Fun(_, ret, _) => *ret,
                        _ => self.ctx().comp_ref_ty,
                    };
                    cur = self.mk(TlcExpr::App(cur, arg), ret_ty);
                    cur_ty = ret_ty;
                }
                let m = cur;
                self.bind_m(m, k)
            }
            _ => unreachable!("reifiable validated the computation shape"),
        }
    }

    fn reify_sequence(&mut self, items: Vec<TlcExprId>, val_ty: TlcTypeId, k: ReifyK) -> TlcExprId {
        let mut iter = items.into_iter();
        let Some(first) = iter.next() else {
            let carrier = self.ctx().carrier_ty;
            let nothing = self.mk(TlcExpr::Lit(Literal::Nothing), carrier);
            return k(self, nothing);
        };
        let rest: Vec<_> = iter.collect();
        if rest.is_empty() {
            return self.reify(first, val_ty, k);
        }
        // A non-last item's value is discarded; reify it with its own recorded
        // type and ignore the result.
        let first_ty = self.expr_ty(first);
        self.reify(
            first,
            first_ty,
            Box::new(move |this, _| this.reify_sequence(rest, val_ty, k)),
        )
    }

    /// Generate the recursive `bind : Computation -> (R -> Computation) -> Computation`.
    fn emit_bind_decl(&mut self) {
        let comp = self.ctx().comp_ref_ty;
        let carrier = self.ctx().carrier_ty;
        let cont_ty = self.ctx().cont_ty;
        let bind_binding = self.ctx().bind_binding;
        let bind_ty = self.ctx().bind_ty;

        let m = self.fresh_binding();
        let kb = self.fresh_binding();

        let mut arms: Vec<TlcAlt> = Vec::new();
        // pure arm: #__zt_pure { value = v } => k v
        let v = self.fresh_binding();
        let v_var = self.var(v, carrier);
        let kb_var = self.var(kb, cont_ty);
        let pure_body = self.mk(TlcExpr::App(kb_var, v_var), comp);
        arms.push(TlcAlt {
            pat: pure_pat(v),
            guard: None,
            body: pure_body,
        });

        let op_list: Vec<(String, TlcTypeId, TlcTypeId, TlcTypeId)> = self
            .ctx()
            .ops
            .iter()
            .map(|(name, oi)| (name.clone(), oi.arg_ty, oi.resume_ty, oi.payload_ty))
            .collect();
        for (op, arg_ty, resume_ty, payload_ty) in op_list {
            let p = self.fresh_binding();
            let r = self.fresh_binding();
            let resume_fn_ty = self.fun_ty(resume_ty, comp);
            // resume' = \x. bind (r x) k
            let x = self.fresh_binding();
            let x_var = self.var(x, resume_ty);
            let r_var = self.var(r, resume_fn_ty);
            let rx = self.mk(TlcExpr::App(r_var, x_var), comp);
            let bind_var = self.var(bind_binding, bind_ty);
            let bind_inner_ty = self.fun_ty(cont_ty, comp);
            let b1 = self.mk(TlcExpr::App(bind_var, rx), bind_inner_ty);
            let kb_var2 = self.var(kb, cont_ty);
            let b2 = self.mk(TlcExpr::App(b1, kb_var2), comp);
            let new_resume = self.mk(TlcExpr::Lam(x, resume_ty, b2), resume_fn_ty);
            let p_var = self.var(p, arg_ty);
            let rec = self.mk(
                TlcExpr::Record(vec![
                    ("payload".to_string(), p_var),
                    ("resume".to_string(), new_resume),
                ]),
                payload_ty,
            );
            let op_node = self.mk(TlcExpr::Variant(op_tag(&op), rec), comp);
            arms.push(TlcAlt {
                pat: op_pat(&op, p, r),
                guard: None,
                body: op_node,
            });
        }

        let m_var = self.var(m, comp);
        let case = self.mk(TlcExpr::Case(m_var, arms), comp);
        let inner_lam_ty = self.fun_ty(cont_ty, comp);
        let inner_lam = self.mk(TlcExpr::Lam(kb, cont_ty, case), inner_lam_ty);
        let outer_lam = self.mk(TlcExpr::Lam(m, comp, inner_lam), bind_ty);
        let decl = self.module.decl_arena.alloc(TlcDecl::Value {
            binding: bind_binding,
            ty: bind_ty,
            body: outer_lam,
        });
        self.module.decls.push(decl);
    }

    /// Generate the recursive `run : Computation -> HandleResult` driver and
    /// return its binding.
    fn emit_run_decl(
        &mut self,
        value_clause: Option<TlcExprId>,
        ops_clauses: &[TlcHandleClause],
        handle_result_ty: TlcTypeId,
    ) -> BindingId {
        let comp = self.ctx().comp_ref_ty;
        let carrier = self.ctx().carrier_ty;
        let run_binding = self.fresh_binding();
        let run_ty = self.fun_ty(comp, handle_result_ty);
        let m = self.fresh_binding();

        let mut arms: Vec<TlcAlt> = Vec::new();
        // pure arm: value clause applied, or identity.
        let v = self.fresh_binding();
        let pure_body = if let Some(vc) = value_clause {
            let v_var = self.var(v, carrier);
            self.mk(TlcExpr::App(vc, v_var), handle_result_ty)
        } else {
            self.var(v, carrier)
        };
        arms.push(TlcAlt {
            pat: pure_pat(v),
            guard: None,
            body: pure_body,
        });

        let op_list: Vec<(String, TlcTypeId, TlcTypeId, TlcExprId)> = ops_clauses
            .iter()
            .map(|c| {
                let oi = &self.ctx().ops[&c.op];
                (c.op.clone(), oi.arg_ty, oi.resume_ty, oi.handler_body)
            })
            .collect();
        for (op, arg_ty, resume_ty, handler_body) in op_list {
            let p = self.fresh_binding();
            let r = self.fresh_binding();
            let resume_fn_ty = self.fun_ty(resume_ty, comp);
            let handler_rw =
                self.rewrite_resume(handler_body, run_binding, run_ty, r, resume_fn_ty);
            let p_var = self.var(p, arg_ty);
            let body = self.mk(TlcExpr::App(handler_rw, p_var), handle_result_ty);
            arms.push(TlcAlt {
                pat: op_pat(&op, p, r),
                guard: None,
                body,
            });
        }

        let m_var = self.var(m, comp);
        let case = self.mk(TlcExpr::Case(m_var, arms), handle_result_ty);
        let lam = self.mk(TlcExpr::Lam(m, comp, case), run_ty);
        let decl = self.module.decl_arena.alloc(TlcDecl::Value {
            binding: run_binding,
            ty: run_ty,
            body: lam,
        });
        self.module.decls.push(decl);
        run_binding
    }

    /// Rewrite a handler clause body, replacing `resume X` with `run (r X)`.
    /// Subtrees with no `resume` are shared unchanged.
    fn rewrite_resume(
        &mut self,
        id: TlcExprId,
        run_binding: BindingId,
        run_ty: TlcTypeId,
        r_binding: BindingId,
        resume_fn_ty: TlcTypeId,
    ) -> TlcExprId {
        if self.no_residual_control(id) {
            return id;
        }
        let ty = self.expr_ty(id);
        let rec = |this: &mut Self, child: TlcExprId| {
            this.rewrite_resume(child, run_binding, run_ty, r_binding, resume_fn_ty)
        };
        match self.module.expr_arena[id].clone() {
            TlcExpr::Resume { value } => {
                let comp = self.ctx().comp_ref_ty;
                let r_var = self.var(r_binding, resume_fn_ty);
                let rx = self.mk(TlcExpr::App(r_var, value), comp);
                let run_var = self.var(run_binding, run_ty);
                self.mk(TlcExpr::App(run_var, rx), ty)
            }
            TlcExpr::Lam(b, lty, body) => {
                let body = rec(self, body);
                self.mk(TlcExpr::Lam(b, lty, body), ty)
            }
            TlcExpr::App(f, a) => {
                let f = rec(self, f);
                let a = rec(self, a);
                self.mk(TlcExpr::App(f, a), ty)
            }
            TlcExpr::Let {
                binding,
                ty: lty,
                value,
                body,
            } => {
                let value = rec(self, value);
                let body = rec(self, body);
                self.mk(
                    TlcExpr::Let {
                        binding,
                        ty: lty,
                        value,
                        body,
                    },
                    ty,
                )
            }
            TlcExpr::Case(scrut, alts) => {
                let scrut = rec(self, scrut);
                let alts = alts
                    .into_iter()
                    .map(|alt| {
                        let guard = alt.guard.map(|g| rec(self, g));
                        let body = rec(self, alt.body);
                        TlcAlt {
                            pat: alt.pat,
                            guard,
                            body,
                        }
                    })
                    .collect();
                self.mk(TlcExpr::Case(scrut, alts), ty)
            }
            TlcExpr::Builtin(op, l, r) => {
                let l = rec(self, l);
                let r = rec(self, r);
                self.mk(TlcExpr::Builtin(op, l, r), ty)
            }
            TlcExpr::Sequence(items) => {
                let items = items.into_iter().map(|i| rec(self, i)).collect();
                self.mk(TlcExpr::Sequence(items), ty)
            }
            TlcExpr::Variant(tag, payload) => {
                let payload = rec(self, payload);
                self.mk(TlcExpr::Variant(tag, payload), ty)
            }
            TlcExpr::Record(fields) => {
                let fields = fields.into_iter().map(|(n, e)| (n, rec(self, e))).collect();
                self.mk(TlcExpr::Record(fields), ty)
            }
            TlcExpr::GetField(base, field) => {
                let base = rec(self, base);
                self.mk(TlcExpr::GetField(base, field), ty)
            }
            TlcExpr::TyApp(e, t) => {
                let e = rec(self, e);
                self.mk(TlcExpr::TyApp(e, t), ty)
            }
            TlcExpr::TyLam(v, kind, body) => {
                let body = rec(self, body);
                self.mk(TlcExpr::TyLam(v, kind, body), ty)
            }
            // No other node kind can contain a `resume` in a validated handler body.
            _ => id,
        }
    }
}

fn pure_pat(v: BindingId) -> TlcPat {
    TlcPat::Variant(
        PURE_TAG.to_string(),
        Box::new(TlcPat::Record(vec![("value".to_string(), TlcPat::Bind(v))])),
    )
}

fn op_pat(op: &str, p: BindingId, r: BindingId) -> TlcPat {
    TlcPat::Variant(
        op_tag(op),
        Box::new(TlcPat::Record(vec![
            ("payload".to_string(), TlcPat::Bind(p)),
            ("resume".to_string(), TlcPat::Bind(r)),
        ])),
    )
}
