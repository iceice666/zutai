use rustc_hash::FxHashSet;

use la_arena::Arena;

use crate::ir::{Row, TlcTupleField, TlcType, TlcTypeId, TlcTypeVar};

// ── Free type variables ───────────────────────────────────────────────────────

/// Collect all type variables (`TyVar`) that appear free in `ty`.
/// Row variables (`RVar`) are ignored — they have a different kind and cannot be
/// captured by `TyLamK`/`ForAll` binders, which bind type-kinded variables.
pub(super) fn free_type_vars(arena: &Arena<TlcType>, ty: TlcTypeId) -> FxHashSet<TlcTypeVar> {
    let mut free = FxHashSet::default();
    collect_free(arena, ty, &FxHashSet::default(), &mut free);
    free
}

pub(super) fn collect_free(
    arena: &Arena<TlcType>,
    ty: TlcTypeId,
    bound: &FxHashSet<TlcTypeVar>,
    free: &mut FxHashSet<TlcTypeVar>,
) {
    match arena[ty].clone() {
        TlcType::TyVar(v, _) => {
            if !bound.contains(&v) {
                free.insert(v);
            }
        }
        TlcType::TyLamK(binder, _, body) => {
            let mut new_bound = bound.clone();
            new_bound.insert(binder);
            collect_free(arena, body, &new_bound, free);
        }
        TlcType::ForAll(binder, _, body) => {
            let mut new_bound = bound.clone();
            new_bound.insert(binder);
            collect_free(arena, body, &new_bound, free);
        }
        TlcType::TyApp(f, a) => {
            collect_free(arena, f, bound, free);
            collect_free(arena, a, bound, free);
        }
        TlcType::Fun(from, to, eff) => {
            collect_free(arena, from, bound, free);
            collect_free(arena, to, bound, free);
            collect_free_row(arena, &eff, bound, free);
        }
        TlcType::List(inner) | TlcType::Optional(inner) | TlcType::Maybe(inner) => {
            collect_free(arena, inner, bound, free);
        }
        TlcType::Record(row) | TlcType::VariantT(row) => {
            collect_free_row(arena, &row, bound, free);
        }
        TlcType::Tuple(items) => {
            for item in &items {
                let t = match item {
                    TlcTupleField::Positional(t) => *t,
                    TlcTupleField::Named { ty, .. } => *ty,
                };
                collect_free(arena, t, bound, free);
            }
        }
        TlcType::Prim(_) | TlcType::Opaque(_) | TlcType::Singleton(_) => {}
    }
}

pub(super) fn collect_free_row(
    arena: &Arena<TlcType>,
    row: &Row,
    bound: &FxHashSet<TlcTypeVar>,
    free: &mut FxHashSet<TlcTypeVar>,
) {
    match row {
        Row::REmpty => {}
        // RVar shares the TlcTypeVar namespace with type-kinded variables. A TyLamK/ForAll
        // binder whose id appears as a free RVar in the replacement must be freshened.
        Row::RVar(v) => {
            if !bound.contains(v) {
                free.insert(*v);
            }
        }
        Row::RExtend { ty, tail, .. } => {
            collect_free(arena, *ty, bound, free);
            collect_free_row(arena, tail, bound, free);
        }
    }
}

// ── Alpha-renaming ────────────────────────────────────────────────────────────

/// Rename all free occurrences of `old` to `fresh` in `ty`.
/// `fresh` must not appear anywhere in `ty` (guaranteed when allocated from the
/// downward counter in `subst_inner`), so no capture check is needed here.
pub(super) fn alpha_rename(
    arena: &mut Arena<TlcType>,
    ty: TlcTypeId,
    old: TlcTypeVar,
    fresh: TlcTypeVar,
) -> TlcTypeId {
    match arena[ty].clone() {
        TlcType::TyVar(v, k) if v == old => arena.alloc(TlcType::TyVar(fresh, k)),
        TlcType::TyVar(_, _) => ty,
        TlcType::TyLamK(binder, kind, body) => {
            if binder == old {
                return ty; // old is re-bound here — stop descending
            }
            let new_body = alpha_rename(arena, body, old, fresh);
            arena.alloc(TlcType::TyLamK(binder, kind, new_body))
        }
        TlcType::ForAll(binder, kind, body) => {
            if binder == old {
                return ty; // old is re-bound here — stop descending
            }
            let new_body = alpha_rename(arena, body, old, fresh);
            arena.alloc(TlcType::ForAll(binder, kind, new_body))
        }
        TlcType::TyApp(f, a) => {
            let nf = alpha_rename(arena, f, old, fresh);
            let na = alpha_rename(arena, a, old, fresh);
            arena.alloc(TlcType::TyApp(nf, na))
        }
        TlcType::Fun(from, to, eff) => {
            let nf = alpha_rename(arena, from, old, fresh);
            let nt = alpha_rename(arena, to, old, fresh);
            let neff = alpha_rename_row(arena, eff, old, fresh);
            arena.alloc(TlcType::Fun(nf, nt, neff))
        }
        TlcType::List(inner) => {
            let ni = alpha_rename(arena, inner, old, fresh);
            arena.alloc(TlcType::List(ni))
        }
        TlcType::Optional(inner) => {
            let ni = alpha_rename(arena, inner, old, fresh);
            arena.alloc(TlcType::Optional(ni))
        }
        TlcType::Maybe(inner) => {
            let ni = alpha_rename(arena, inner, old, fresh);
            arena.alloc(TlcType::Maybe(ni))
        }
        TlcType::Record(row) => {
            let new_row = alpha_rename_row(arena, row, old, fresh);
            arena.alloc(TlcType::Record(new_row))
        }
        TlcType::VariantT(row) => {
            let new_row = alpha_rename_row(arena, row, old, fresh);
            arena.alloc(TlcType::VariantT(new_row))
        }
        TlcType::Tuple(items) => {
            let new_items: Vec<TlcTupleField> = items
                .iter()
                .map(|item| match item {
                    TlcTupleField::Positional(t) => {
                        TlcTupleField::Positional(alpha_rename(arena, *t, old, fresh))
                    }
                    TlcTupleField::Named { name, ty } => TlcTupleField::Named {
                        name: name.clone(),
                        ty: alpha_rename(arena, *ty, old, fresh),
                    },
                })
                .collect();
            arena.alloc(TlcType::Tuple(new_items))
        }
        TlcType::Prim(_) | TlcType::Opaque(_) | TlcType::Singleton(_) => ty,
    }
}

pub(super) fn alpha_rename_row(
    arena: &mut Arena<TlcType>,
    row: Row,
    old: TlcTypeVar,
    fresh: TlcTypeVar,
) -> Row {
    match row {
        Row::REmpty => Row::REmpty,
        // RVar shares the TlcTypeVar namespace; rename it when it matches `old`.
        Row::RVar(v) if v == old => Row::RVar(fresh),
        Row::RVar(v) => Row::RVar(v),
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } => Row::RExtend {
            label,
            ty: alpha_rename(arena, ty, old, fresh),
            optional,
            tail: Box::new(alpha_rename_row(arena, *tail, old, fresh)),
        },
    }
}

// ── Capture-avoiding substitution ────────────────────────────────────────────

/// Entry point: substitute all free occurrences of `var` in `ty` with `replacement`.
///
/// Capture-avoiding: if a `TyLamK`/`ForAll` binder `b ≠ var` has the same id as a free
/// variable in `replacement`, `b` is alpha-renamed to a fresh `Inferred(next_fresh)` before
/// descending. This prevents the replacement's free variable from being shadowed by `b`.
///
/// In v0 all replacements from THIR lowering are closed types, so the freshening path is
/// unreachable for any real `.zt` program. The upgrade is mandatory for v1 row-polymorphic
/// types where open-record/union types can carry free type variables.
pub(super) fn subst(
    arena: &mut Arena<TlcType>,
    ty: TlcTypeId,
    var: TlcTypeVar,
    replacement: TlcTypeId,
    next_fresh: &mut u32,
) -> TlcTypeId {
    let replacement_free = free_type_vars(arena, replacement);
    subst_inner(arena, ty, var, replacement, &replacement_free, next_fresh)
}

pub(super) fn subst_inner(
    arena: &mut Arena<TlcType>,
    ty: TlcTypeId,
    var: TlcTypeVar,
    replacement: TlcTypeId,
    replacement_free: &FxHashSet<TlcTypeVar>,
    next_fresh: &mut u32,
) -> TlcTypeId {
    match arena[ty].clone() {
        TlcType::TyVar(v, _) if v == var => replacement,
        TlcType::TyVar(_, _) => ty,

        // Capture-avoiding: freshen any binder whose id appears free in `replacement`.
        TlcType::TyLamK(binder, kind, body) => {
            if binder == var {
                return ty; // shadowed — do not descend
            }
            let (new_binder, new_body) = if replacement_free.contains(&binder) {
                let fresh = TlcTypeVar::Inferred(*next_fresh);
                *next_fresh -= 1;
                let renamed = alpha_rename(arena, body, binder, fresh);
                (fresh, renamed)
            } else {
                (binder, body)
            };
            let subst_body = subst_inner(
                arena,
                new_body,
                var,
                replacement,
                replacement_free,
                next_fresh,
            );
            arena.alloc(TlcType::TyLamK(new_binder, kind, subst_body))
        }
        TlcType::ForAll(binder, kind, body) => {
            if binder == var {
                return ty; // shadowed
            }
            let (new_binder, new_body) = if replacement_free.contains(&binder) {
                let fresh = TlcTypeVar::Inferred(*next_fresh);
                *next_fresh -= 1;
                let renamed = alpha_rename(arena, body, binder, fresh);
                (fresh, renamed)
            } else {
                (binder, body)
            };
            let subst_body = subst_inner(
                arena,
                new_body,
                var,
                replacement,
                replacement_free,
                next_fresh,
            );
            arena.alloc(TlcType::ForAll(new_binder, kind, subst_body))
        }

        TlcType::TyApp(f, a) => {
            let nf = subst_inner(arena, f, var, replacement, replacement_free, next_fresh);
            let na = subst_inner(arena, a, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::TyApp(nf, na))
        }
        TlcType::Fun(from, to, eff) => {
            let nf = subst_inner(arena, from, var, replacement, replacement_free, next_fresh);
            let nt = subst_inner(arena, to, var, replacement, replacement_free, next_fresh);
            let neff = subst_row_inner(arena, &eff, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Fun(nf, nt, neff))
        }
        TlcType::List(inner) => {
            let ni = subst_inner(arena, inner, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::List(ni))
        }
        TlcType::Optional(inner) => {
            let ni = subst_inner(arena, inner, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Optional(ni))
        }
        TlcType::Maybe(inner) => {
            let ni = subst_inner(arena, inner, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Maybe(ni))
        }
        TlcType::Record(row) => {
            let new_row =
                subst_row_inner(arena, &row, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Record(new_row))
        }
        TlcType::VariantT(row) => {
            let new_row =
                subst_row_inner(arena, &row, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::VariantT(new_row))
        }
        TlcType::Tuple(items) => {
            let new_items: Vec<TlcTupleField> = items
                .iter()
                .map(|item| match item {
                    TlcTupleField::Positional(ty_id) => TlcTupleField::Positional(subst_inner(
                        arena,
                        *ty_id,
                        var,
                        replacement,
                        replacement_free,
                        next_fresh,
                    )),
                    TlcTupleField::Named { name, ty: ty_id } => TlcTupleField::Named {
                        name: name.clone(),
                        ty: subst_inner(
                            arena,
                            *ty_id,
                            var,
                            replacement,
                            replacement_free,
                            next_fresh,
                        ),
                    },
                })
                .collect();
            arena.alloc(TlcType::Tuple(new_items))
        }
        // Atoms — nothing to substitute.
        TlcType::Prim(_) | TlcType::Opaque(_) | TlcType::Singleton(_) => ty,
    }
}

pub(super) fn subst_row_inner(
    arena: &mut Arena<TlcType>,
    row: &Row,
    var: TlcTypeVar,
    replacement: TlcTypeId,
    replacement_free: &FxHashSet<TlcTypeVar>,
    next_fresh: &mut u32,
) -> Row {
    match row {
        Row::REmpty => Row::REmpty,
        // Type-variable substitution is inert on row variables — different kind discipline.
        Row::RVar(v) => Row::RVar(*v),
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } => Row::RExtend {
            label: label.clone(),
            ty: subst_inner(arena, *ty, var, replacement, replacement_free, next_fresh),
            optional: *optional,
            tail: Box::new(subst_row_inner(
                arena,
                tail,
                var,
                replacement,
                replacement_free,
                next_fresh,
            )),
        },
    }
}
