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

mod analysis;
mod construct;

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
