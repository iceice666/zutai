//! ANF → SSA lowering.
//!
//! Converts flat ANF bindings into basic blocks with phi nodes at join points.

use rustc_hash::FxHashSet;
use zutai_anf::{AnfArm, AnfAtom, AnfBody, AnfExpr, AnfPattern, AnfTupleItem, AnfTuplePatItem};

// ── Free variable analysis ─────────────────────────────────────────────────────

pub(super) fn free_vars_atom(atom: &AnfAtom) -> FxHashSet<String> {
    match atom {
        AnfAtom::Var(name) => {
            let mut s = FxHashSet::default();
            s.insert(name.clone());
            s
        }
        AnfAtom::Lit(_) | AnfAtom::Global(_) => FxHashSet::default(),
    }
}

pub(super) fn free_vars_expr(expr: &AnfExpr) -> FxHashSet<String> {
    match expr {
        AnfExpr::Atom(atom) => free_vars_atom(atom),
        AnfExpr::Apply { func, arg } => free_vars_atom(func)
            .union(&free_vars_atom(arg))
            .cloned()
            .collect(),
        AnfExpr::HostPrint { value } => free_vars_atom(value),
        AnfExpr::TyApp { poly, ty_args: _ } => free_vars_atom(poly),
        AnfExpr::Lambda { param, body } => {
            let mut fv = free_vars_body(body);
            fv.remove(param);
            fv
        }
        AnfExpr::TyLam { ty_params: _, body } => free_vars_body(body),
        AnfExpr::Record(fields) => fields.iter().flat_map(free_vars_atom).collect(),
        AnfExpr::RecordUpdate { base, updates } => {
            let mut fv = free_vars_atom(base);
            for (_, value) in updates {
                fv.extend(free_vars_atom(value));
            }
            fv
        }
        AnfExpr::Tuple(items) => items
            .iter()
            .flat_map(|i| match i {
                AnfTupleItem::Named { name: _, value } => free_vars_atom(value),
                AnfTupleItem::Positional(a) => free_vars_atom(a),
            })
            .collect(),
        AnfExpr::List(elems) => elems.iter().flat_map(free_vars_atom).collect(),
        AnfExpr::ListPrim { args, .. } => args.iter().flat_map(free_vars_atom).collect(),
        AnfExpr::Select { base, slot: _ } => free_vars_atom(base),
        AnfExpr::Match { scrutinee, arms } => {
            let mut fv = free_vars_atom(scrutinee);
            for arm in arms {
                fv.extend(free_vars_arm(arm));
            }
            fv
        }
        AnfExpr::Coalesce { value, fallback } => free_vars_atom(value)
            .union(&free_vars_atom(fallback))
            .cloned()
            .collect(),
        AnfExpr::Builtin { op: _, lhs, rhs } => free_vars_atom(lhs)
            .union(&free_vars_atom(rhs))
            .cloned()
            .collect(),
        AnfExpr::Variant { value, .. } => free_vars_atom(value),
        AnfExpr::HostOp { value, .. } => free_vars_atom(value),
        AnfExpr::Error => FxHashSet::default(),
    }
}

pub(super) fn free_vars_arm(arm: &AnfArm) -> FxHashSet<String> {
    let mut fv = free_vars_body(&arm.body);
    if let Some(guard) = &arm.guard {
        fv.extend(free_vars_body(guard));
    }
    let bound = pattern_bindings(&arm.pattern);
    for b in &bound {
        fv.remove(b);
    }
    fv
}

pub(super) fn pattern_bindings(pat: &AnfPattern) -> Vec<String> {
    match pat {
        AnfPattern::Wildcard | AnfPattern::Lit(_) | AnfPattern::Atom(_) => vec![],
        AnfPattern::Bind(name) => vec![name.clone()],
        AnfPattern::Tuple(items) => items
            .iter()
            .flat_map(|i| match i {
                AnfTuplePatItem::Named { name: _, pattern } => pattern_bindings(pattern),
                AnfTuplePatItem::Positional(p) => pattern_bindings(p),
            })
            .collect(),
        AnfPattern::Record(fields) => fields
            .iter()
            .flat_map(|(_, p)| pattern_bindings(p))
            .collect(),
        AnfPattern::Variant { pattern, .. } => pattern_bindings(pattern),
    }
}

pub(super) fn free_vars_body(body: &AnfBody) -> FxHashSet<String> {
    let mut fv = FxHashSet::default();
    let mut bound = FxHashSet::default();
    for (name, expr) in &body.bindings {
        for v in free_vars_expr(expr) {
            if !bound.contains(&v) {
                fv.insert(v);
            }
        }
        bound.insert(name.clone());
    }
    for v in free_vars_atom(&body.result) {
        if !bound.contains(&v) {
            fv.insert(v);
        }
    }
    fv
}
