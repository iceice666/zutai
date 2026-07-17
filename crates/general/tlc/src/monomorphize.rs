//! Row-erased monomorphization of open-row field selects (Phase C).
//!
//! A function whose parameter is an open record (`getN :: { n : Int; ...; } ->
//! Int = x => x.n`) reads `x.n` by SLOT, but the slot is computed from the open
//! VIEW (`{n}` → n at slot 0) while the concrete argument `{ extra; n }` puts `n`
//! at slot 1 (records are name-sorted). The slot-based backend would miscompile,
//! so it is gated before Dataflow Core.
//!
//! This pass closes that gap at concrete call sites: each `f arg` where `f` is an
//! open-row-selecting function and `arg` has a closed record type is INLINED — the
//! call becomes `let x = arg in body`, with the function's row variable
//! substituted by the argument's extra fields throughout the cloned body. The
//! inlined `body`'s field selects then see the concrete record type and DC
//! computes the correct slot. Fully-inlined (dead) function declarations are
//! dropped so the gate no longer sees their open-row selects.
//!
//! Genuinely polymorphic uses (a function passed as a value, or applied to a
//! still-open argument) are left untouched and stay gated by design.

use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;

use crate::ir::{
    Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcHandleClause, TlcModule, TlcTupleField,
    TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};

/// One open-row-selecting function eligible for call-site inlining.
struct Candidate {
    /// The function's value-parameter binding (becomes the `let` binding).
    param: BindingId,
    /// The open row's tail variable, substituted per call site.
    row_var: TlcTypeVar,
    /// The parameter's known field labels (everything else is the row tail).
    known: Vec<String>,
    /// The function body under the parameter lambda (the part to clone+inline).
    body: TlcExprId,
}

/// Inline concrete-argument calls to open-row-selecting functions, then drop the
/// declarations that become dead. Idempotent and a no-op when no candidate exists.
pub fn monomorphize_open_row_selects(module: &mut TlcModule) {
    let candidates = collect_candidates(module);
    if candidates.is_empty() {
        return;
    }
    let sites = collect_call_sites(module, &candidates);
    if sites.is_empty() {
        return;
    }
    for (app_id, f, arg) in sites {
        inline_site(module, &candidates[&f], app_id, arg);
    }
    drop_dead_candidates(module, &candidates);
}

/// A function `f` is a candidate when its declaration body is a single value
/// lambda whose parameter is an *open* record type.
fn collect_candidates(module: &TlcModule) -> FxHashMap<BindingId, Candidate> {
    let mut out = FxHashMap::default();
    for &decl_id in &module.decls {
        let TlcDecl::Value { binding, body, .. } = &module.decl_arena[decl_id] else {
            continue;
        };
        let TlcExpr::Lam(param, param_ty, fn_body) = &module.expr_arena[*body] else {
            continue;
        };
        let TlcType::Record(row) = &module.type_arena[*param_ty] else {
            continue;
        };
        let Some(row_var) = open_row_var(row) else {
            continue;
        };
        // Only specialize a function that actually reads a field by slot from its
        // open parameter (otherwise inlining is unnecessary), and never a
        // recursive one: `clone_expr` reuses binder ids, so inlining a concrete
        // self-call would nest a clone that re-binds and removes a still-live
        // binding under DC's flat scope.
        if !has_open_row_select(module, *fn_body) || references_binding(module, *body, *binding) {
            continue;
        }
        out.insert(
            *binding,
            Candidate {
                param: *param,
                row_var,
                known: row.fields().map(|(l, _)| l.to_string()).collect(),
                body: *fn_body,
            },
        );
    }
    out
}

/// `App(Var(f), arg)` call sites where `f` is a candidate. Whether `arg` provides
/// a concrete field set is decided per site in [`inline_site`].
fn collect_call_sites(
    module: &TlcModule,
    candidates: &FxHashMap<BindingId, Candidate>,
) -> Vec<(TlcExprId, BindingId, TlcExprId)> {
    let mut sites = Vec::new();
    for (id, expr) in module.expr_arena.iter() {
        let TlcExpr::App(callee, arg) = expr else {
            continue;
        };
        let TlcExpr::Var(f) = &module.expr_arena[*callee] else {
            continue;
        };
        if candidates.contains_key(f) {
            sites.push((id, *f, *arg));
        }
    }
    sites
}

/// The concrete `(label, ty, optional)` fields an argument supplies. A record
/// literal names its fields directly (the call-site *type* is the open parameter
/// view, so it cannot be used); otherwise a closed record type works.
fn concrete_record_fields(
    module: &TlcModule,
    arg: TlcExprId,
) -> Option<Vec<(String, TlcTypeId, bool)>> {
    if let TlcExpr::Record(fields) = &module.expr_arena[arg] {
        return fields
            .iter()
            .map(|(label, value)| {
                module
                    .expr_types
                    .get(value)
                    .map(|ty| (label.clone(), *ty, false))
            })
            .collect();
    }
    let arg_ty = module.expr_types.get(&arg).copied()?;
    if let TlcType::Record(row) = &module.type_arena[arg_ty]
        && open_row_var(row).is_none()
    {
        return Some(row_fields_full(row));
    }
    None
}

/// Inline one call site: replace `App(Var(f), arg)` with `let param = arg in
/// clone(body)`, substituting the function's row variable by the argument's extra
/// fields throughout the clone. A no-op when the argument's concrete fields cannot
/// be recovered (the call stays gated).
fn inline_site(module: &mut TlcModule, cand: &Candidate, app_id: TlcExprId, arg: TlcExprId) {
    let Some(concrete) = concrete_record_fields(module, arg) else {
        return;
    };
    // The row variable binds to the argument fields the view does not name.
    let rest: Vec<(String, TlcTypeId, bool)> = concrete
        .into_iter()
        .filter(|(label, _, _)| !cand.known.iter().any(|k| k == label))
        .collect();
    let rest_row = Row::from_record_fields(rest);
    // The call's result type is the genuine type of the inlined expression. The
    // clause body (a `Case`) is recorded with the surrounding *function* type — a
    // tolerated quirk in lambda-body position, but as the `let` body it would be
    // read as the entry type, so override it with the result type.
    let result_ty = module.expr_types.get(&app_id).copied();
    let mut tmemo = FxHashMap::default();
    let body = clone_expr(module, cand.body, cand.row_var, &rest_row, &mut tmemo);
    if let Some(rt) = result_ty {
        module.expr_types.insert(body, rt);
    }
    let arg_ty = module.expr_types[&arg];
    module.expr_arena[app_id] = TlcExpr::Let {
        binding: cand.param,
        ty: arg_ty,
        value: arg,
        body,
    };
}

/// Drop candidate declarations no longer reachable from the live program (their
/// only uses were inlined). A candidate stays if it is referenced from a
/// non-candidate declaration, the final expression, or a retained candidate.
fn drop_dead_candidates(module: &mut TlcModule, candidates: &FxHashMap<BindingId, Candidate>) {
    // Var(f) references reachable from each declaration body / the final expr.
    let mut refs_from: FxHashMap<Option<BindingId>, FxHashSet<BindingId>> = FxHashMap::default();
    for &decl_id in &module.decls {
        if let TlcDecl::Value { binding, body, .. } = &module.decl_arena[decl_id] {
            refs_from.insert(Some(*binding), var_refs_reachable(module, *body));
        }
    }
    if let Some(fe) = module.final_expr {
        refs_from.insert(None, var_refs_reachable(module, fe));
    }

    // Seed the keep-set with candidates referenced from a non-candidate root,
    // then close it under references made by retained candidates.
    let mut kept: FxHashSet<BindingId> = FxHashSet::default();
    for (owner, refs) in &refs_from {
        let owner_is_candidate = owner.is_some_and(|b| candidates.contains_key(&b));
        if owner_is_candidate {
            continue;
        }
        for r in refs {
            if candidates.contains_key(r) {
                kept.insert(*r);
            }
        }
    }
    loop {
        let mut added = false;
        let frontier: Vec<BindingId> = kept.iter().copied().collect();
        for f in frontier {
            if let Some(refs) = refs_from.get(&Some(f)) {
                for r in refs {
                    if candidates.contains_key(r) && kept.insert(*r) {
                        added = true;
                    }
                }
            }
        }
        if !added {
            break;
        }
    }

    module
        .decls
        .retain(|&decl_id| match &module.decl_arena[decl_id] {
            TlcDecl::Value { binding, .. } => {
                !candidates.contains_key(binding) || kept.contains(binding)
            }
            TlcDecl::TypeAlias { .. } => true,
        });
}

/// The open row's tail variable, if the row is open.
fn open_row_var(row: &Row) -> Option<TlcTypeVar> {
    match row {
        Row::REmpty => None,
        Row::RVar(v) => Some(*v),
        Row::RExtend { tail, .. } => open_row_var(tail),
    }
}

/// All `(label, ty, optional)` of a closed row in declaration order.
fn row_fields_full(row: &Row) -> Vec<(String, TlcTypeId, bool)> {
    let mut out = Vec::new();
    let mut cur = row;
    while let Row::RExtend {
        label,
        ty,
        optional,
        tail,
    } = cur
    {
        out.push((label.clone(), *ty, *optional));
        cur = tail;
    }
    out
}

// ── Type substitution ──────────────────────────────────────────────────────────

/// Substitute the row variable `v` by `rest` throughout `ty`, allocating fresh
/// types only where something changes. Memoized per call site.
fn subst_type(
    module: &mut TlcModule,
    ty: TlcTypeId,
    v: TlcTypeVar,
    rest: &Row,
    memo: &mut FxHashMap<TlcTypeId, TlcTypeId>,
) -> TlcTypeId {
    if let Some(&done) = memo.get(&ty) {
        return done;
    }
    let new = match module.type_arena[ty].clone() {
        TlcType::Prim(_) | TlcType::Opaque(_) | TlcType::Singleton(_) | TlcType::TyVar(_, _) => ty,
        TlcType::Fun(from, to, eff) => {
            let from2 = subst_type(module, from, v, rest, memo);
            let to2 = subst_type(module, to, v, rest, memo);
            let eff2 = subst_row(module, &eff, v, rest, memo);
            if from2 == from && to2 == to && eff2 == eff {
                ty
            } else {
                module.type_arena.alloc(TlcType::Fun(from2, to2, eff2))
            }
        }
        TlcType::ForAll(tv, kind, body) => {
            let body2 = subst_type(module, body, v, rest, memo);
            if body2 == body {
                ty
            } else {
                module.type_arena.alloc(TlcType::ForAll(tv, kind, body2))
            }
        }
        TlcType::TyApp(f, x) => {
            let f2 = subst_type(module, f, v, rest, memo);
            let x2 = subst_type(module, x, v, rest, memo);
            if f2 == f && x2 == x {
                ty
            } else {
                module.type_arena.alloc(TlcType::TyApp(f2, x2))
            }
        }
        TlcType::TyLamK(tv, kind, body) => {
            let body2 = subst_type(module, body, v, rest, memo);
            if body2 == body {
                ty
            } else {
                module.type_arena.alloc(TlcType::TyLamK(tv, kind, body2))
            }
        }
        TlcType::Record(row) => {
            let row2 = subst_row(module, &row, v, rest, memo);
            if row2 == row {
                ty
            } else {
                module.type_arena.alloc(TlcType::Record(row2))
            }
        }
        TlcType::VariantT(row) => {
            let row2 = subst_row(module, &row, v, rest, memo);
            if row2 == row {
                ty
            } else {
                module.type_arena.alloc(TlcType::VariantT(row2))
            }
        }
        TlcType::Tuple(fields) => {
            let mut changed = false;
            let fields2: Vec<TlcTupleField> = fields
                .iter()
                .map(|f| match f {
                    TlcTupleField::Named { name, ty: fty } => {
                        let fty2 = subst_type(module, *fty, v, rest, memo);
                        changed |= fty2 != *fty;
                        TlcTupleField::Named {
                            name: name.clone(),
                            ty: fty2,
                        }
                    }
                    TlcTupleField::Positional(fty) => {
                        let fty2 = subst_type(module, *fty, v, rest, memo);
                        changed |= fty2 != *fty;
                        TlcTupleField::Positional(fty2)
                    }
                })
                .collect();
            if changed {
                module.type_arena.alloc(TlcType::Tuple(fields2))
            } else {
                ty
            }
        }
        TlcType::List(inner) => {
            let inner2 = subst_type(module, inner, v, rest, memo);
            if inner2 == inner {
                ty
            } else {
                module.type_arena.alloc(TlcType::List(inner2))
            }
        }
        TlcType::Optional(inner) => {
            let inner2 = subst_type(module, inner, v, rest, memo);
            if inner2 == inner {
                ty
            } else {
                module.type_arena.alloc(TlcType::Optional(inner2))
            }
        }
        TlcType::Maybe(inner) => {
            let inner2 = subst_type(module, inner, v, rest, memo);
            if inner2 == inner {
                ty
            } else {
                module.type_arena.alloc(TlcType::Maybe(inner2))
            }
        }
    };
    memo.insert(ty, new);
    new
}

/// Substitute `v` by `rest` in a row: splice `rest` at `RVar(v)`, recursing into
/// field types.
fn subst_row(
    module: &mut TlcModule,
    row: &Row,
    v: TlcTypeVar,
    rest: &Row,
    memo: &mut FxHashMap<TlcTypeId, TlcTypeId>,
) -> Row {
    match row {
        Row::REmpty => Row::REmpty,
        Row::RVar(x) if *x == v => rest.clone(),
        Row::RVar(x) => Row::RVar(*x),
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } => Row::RExtend {
            label: label.clone(),
            ty: subst_type(module, *ty, v, rest, memo),
            optional: *optional,
            tail: Box::new(subst_row(module, tail, v, rest, memo)),
        },
    }
}

// ── Expression cloning ──────────────────────────────────────────────────────────

/// Deep-clone an expression subtree into fresh arena ids, substituting the row
/// variable `v` by `rest` in every type. Copies `expr_types`, spans, and dict
/// side tables for each cloned node.
fn clone_expr(
    module: &mut TlcModule,
    orig: TlcExprId,
    v: TlcTypeVar,
    rest: &Row,
    tmemo: &mut FxHashMap<TlcTypeId, TlcTypeId>,
) -> TlcExprId {
    let expr = module.expr_arena[orig].clone();
    let new_expr = match expr {
        TlcExpr::Var(b) => TlcExpr::Var(b),
        TlcExpr::Lit(l) => TlcExpr::Lit(l),
        TlcExpr::Import(s) => TlcExpr::Import(s),
        TlcExpr::Lam(b, ty, body) => {
            let ty2 = subst_type(module, ty, v, rest, tmemo);
            let body2 = clone_expr(module, body, v, rest, tmemo);
            TlcExpr::Lam(b, ty2, body2)
        }
        TlcExpr::App(f, a) => {
            let f2 = clone_expr(module, f, v, rest, tmemo);
            let a2 = clone_expr(module, a, v, rest, tmemo);
            TlcExpr::App(f2, a2)
        }
        TlcExpr::TyLam(tv, kind, body) => {
            let body2 = clone_expr(module, body, v, rest, tmemo);
            TlcExpr::TyLam(tv, kind, body2)
        }
        TlcExpr::TyApp(e, ty) => {
            let e2 = clone_expr(module, e, v, rest, tmemo);
            let ty2 = subst_type(module, ty, v, rest, tmemo);
            TlcExpr::TyApp(e2, ty2)
        }
        TlcExpr::Let {
            binding,
            ty,
            value,
            body,
        } => {
            let ty2 = subst_type(module, ty, v, rest, tmemo);
            let value2 = clone_expr(module, value, v, rest, tmemo);
            let body2 = clone_expr(module, body, v, rest, tmemo);
            TlcExpr::Let {
                binding,
                ty: ty2,
                value: value2,
                body: body2,
            }
        }
        TlcExpr::Letrec { bindings, body } => {
            let bindings2 = bindings
                .into_iter()
                .map(|(b, ty, e)| {
                    let ty2 = subst_type(module, ty, v, rest, tmemo);
                    let e2 = clone_expr(module, e, v, rest, tmemo);
                    (b, ty2, e2)
                })
                .collect();
            let body2 = clone_expr(module, body, v, rest, tmemo);
            TlcExpr::Letrec {
                bindings: bindings2,
                body: body2,
            }
        }
        TlcExpr::Case(scrut, alts) => {
            let scrut2 = clone_expr(module, scrut, v, rest, tmemo);
            let alts2 = alts
                .into_iter()
                .map(|alt| TlcAlt {
                    pat: alt.pat,
                    guard: alt.guard.map(|g| clone_expr(module, g, v, rest, tmemo)),
                    body: clone_expr(module, alt.body, v, rest, tmemo),
                })
                .collect();
            TlcExpr::Case(scrut2, alts2)
        }
        TlcExpr::Record(fields) => {
            let fields2 = fields
                .into_iter()
                .map(|(n, e)| (n, clone_expr(module, e, v, rest, tmemo)))
                .collect();
            TlcExpr::Record(fields2)
        }
        TlcExpr::RecordUpdate { receiver, fields } => {
            let receiver2 = clone_expr(module, receiver, v, rest, tmemo);
            let fields2 = fields
                .into_iter()
                .map(|(n, e)| (n, clone_expr(module, e, v, rest, tmemo)))
                .collect();
            TlcExpr::RecordUpdate {
                receiver: receiver2,
                fields: fields2,
            }
        }
        TlcExpr::GetField(base, f) => {
            let base2 = clone_expr(module, base, v, rest, tmemo);
            TlcExpr::GetField(base2, f)
        }
        TlcExpr::Tuple(items) => {
            let items2 = items
                .into_iter()
                .map(|it| match it {
                    TlcTupleItem::Named { name, value } => TlcTupleItem::Named {
                        name,
                        value: clone_expr(module, value, v, rest, tmemo),
                    },
                    TlcTupleItem::Positional(e) => {
                        TlcTupleItem::Positional(clone_expr(module, e, v, rest, tmemo))
                    }
                })
                .collect();
            TlcExpr::Tuple(items2)
        }
        TlcExpr::List(es) => {
            let es2 = es
                .into_iter()
                .map(|e| clone_expr(module, e, v, rest, tmemo))
                .collect();
            TlcExpr::List(es2)
        }
        TlcExpr::ListAppend(l, r) => {
            let l2 = clone_expr(module, l, v, rest, tmemo);
            let r2 = clone_expr(module, r, v, rest, tmemo);
            TlcExpr::ListAppend(l2, r2)
        }
        TlcExpr::Builtin(op, l, r) => {
            let l2 = clone_expr(module, l, v, rest, tmemo);
            let r2 = clone_expr(module, r, v, rest, tmemo);
            TlcExpr::Builtin(op, l2, r2)
        }
        TlcExpr::Variant(tag, e) => {
            let e2 = clone_expr(module, e, v, rest, tmemo);
            TlcExpr::Variant(tag, e2)
        }
        TlcExpr::Perform { op, arg } => {
            let arg2 = clone_expr(module, arg, v, rest, tmemo);
            TlcExpr::Perform { op, arg: arg2 }
        }
        TlcExpr::Handle {
            expr,
            value,
            finally,
            ops,
        } => {
            let expr2 = clone_expr(module, expr, v, rest, tmemo);
            let value2 = value.map(|val| clone_expr(module, val, v, rest, tmemo));
            let finally2 = finally.map(|fin| clone_expr(module, fin, v, rest, tmemo));
            let ops2 = ops
                .into_iter()
                .map(|c| TlcHandleClause {
                    op: c.op,
                    body: clone_expr(module, c.body, v, rest, tmemo),
                })
                .collect();
            TlcExpr::Handle {
                expr: expr2,
                value: value2,
                finally: finally2,
                ops: ops2,
            }
        }
        TlcExpr::Resume { value } => {
            let value2 = clone_expr(module, value, v, rest, tmemo);
            TlcExpr::Resume { value: value2 }
        }
        TlcExpr::Sequence(es) => {
            let es2 = es
                .into_iter()
                .map(|e| clone_expr(module, e, v, rest, tmemo))
                .collect();
            TlcExpr::Sequence(es2)
        }
    };
    let new_ty = module
        .expr_types
        .get(&orig)
        .copied()
        .map(|t| subst_type(module, t, v, rest, tmemo));
    let span = module.spans.get(&orig).copied();
    let slot = module.dict_field_slots.get(&orig).copied();
    let key = module.dict_dispatch_keys.get(&orig).cloned();
    let id = module.expr_arena.alloc(new_expr);
    if let Some(t) = new_ty {
        module.expr_types.insert(id, t);
    }
    if let Some(s) = span {
        module.spans.insert(id, s);
    }
    if let Some(slot) = slot {
        module.dict_field_slots.insert(id, slot);
    }
    if let Some(k) = key {
        module.dict_dispatch_keys.insert(id, k);
    }
    id
}

// ── Reachability ────────────────────────────────────────────────────────────────

/// Expressions reachable from the module's declarations and final expression —
/// exactly the set Dataflow Core lowers. The open-row gate restricts its check to
/// this set so an inlined-away (dead) declaration's open-row select, still present
/// in the arena, does not falsely reject the program.
pub fn reachable_exprs(module: &TlcModule) -> FxHashSet<TlcExprId> {
    let mut seen = FxHashSet::default();
    let mut stack: Vec<TlcExprId> = Vec::new();
    for &decl_id in &module.decls {
        if let TlcDecl::Value { body, .. } = &module.decl_arena[decl_id] {
            stack.push(*body);
        }
    }
    if let Some(fe) = module.final_expr {
        stack.push(fe);
    }
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        push_child_exprs(&module.expr_arena[id], &mut stack);
    }
    seen
}

/// Var(f) references reachable from `root`.
fn var_refs_reachable(module: &TlcModule, root: TlcExprId) -> FxHashSet<BindingId> {
    let mut seen = FxHashSet::default();
    let mut refs = FxHashSet::default();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let TlcExpr::Var(b) = &module.expr_arena[id] {
            refs.insert(*b);
        }
        push_child_exprs(&module.expr_arena[id], &mut stack);
    }
    refs
}

/// Whether `target` is referenced (as `Var`) anywhere reachable from `root`.
fn references_binding(module: &TlcModule, root: TlcExprId, target: BindingId) -> bool {
    var_refs_reachable(module, root).contains(&target)
}

/// Whether some value-record `GetField` reachable from `root` reads a field from
/// an open record row — the case row-erased monomorphization exists to lower.
fn has_open_row_select(module: &TlcModule, root: TlcExprId) -> bool {
    let mut seen = FxHashSet::default();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let TlcExpr::GetField(base, _) = &module.expr_arena[id]
            && !module.dict_field_slots.contains_key(&id)
            && let Some(&ty) = module.expr_types.get(base)
            && let TlcType::Record(row) = &module.type_arena[ty]
            && open_row_var(row).is_some()
        {
            return true;
        }
        push_child_exprs(&module.expr_arena[id], &mut stack);
    }
    false
}

pub fn push_child_exprs(expr: &TlcExpr, out: &mut Vec<TlcExprId>) {
    match expr {
        TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => {}
        TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
            out.push(*body)
        }
        TlcExpr::App(f, a) | TlcExpr::Builtin(_, f, a) | TlcExpr::ListAppend(f, a) => {
            out.push(*f);
            out.push(*a);
        }
        TlcExpr::Let { value, body, .. } => {
            out.push(*value);
            out.push(*body);
        }
        TlcExpr::Letrec { bindings, body } => {
            out.extend(bindings.iter().map(|(_, _, e)| *e));
            out.push(*body);
        }
        TlcExpr::Case(scrut, alts) => {
            out.push(*scrut);
            for alt in alts {
                if let Some(g) = alt.guard {
                    out.push(g);
                }
                out.push(alt.body);
            }
        }
        TlcExpr::Record(fields) => out.extend(fields.iter().map(|(_, e)| *e)),
        TlcExpr::RecordUpdate { receiver, fields } => {
            out.push(*receiver);
            out.extend(fields.iter().map(|(_, e)| *e));
        }
        TlcExpr::GetField(base, _) => out.push(*base),
        TlcExpr::Tuple(items) => out.extend(items.iter().map(|it| match it {
            TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => *value,
        })),
        TlcExpr::List(es) | TlcExpr::Sequence(es) => out.extend(es.iter().copied()),
        TlcExpr::Variant(_, e) | TlcExpr::Perform { arg: e, .. } | TlcExpr::Resume { value: e } => {
            out.push(*e)
        }
        TlcExpr::Handle {
            expr,
            value,
            finally,
            ops,
        } => {
            out.push(*expr);
            if let Some(val) = value {
                out.push(*val);
            }
            if let Some(fin) = finally {
                out.push(*fin);
            }
            out.extend(ops.iter().map(|c| c.body));
        }
    }
}
