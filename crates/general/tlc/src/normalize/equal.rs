use rustc_hash::FxHashMap;

use la_arena::Arena;

use crate::ir::{Row, TlcTupleField, TlcType, TlcTypeId, TlcTypeVar};

// ── Deep structural equality ──────────────────────────────────────────────────

/// Collect `(label, ty, optional)` fields from a row spine (stopping at `REmpty` or `RVar`),
/// and return the tail kind (`None` = closed, `Some(v)` = open via `RVar(v)`).
pub(super) fn row_fields_and_tail(row: &Row) -> (Vec<(&str, TlcTypeId, bool)>, Option<TlcTypeVar>) {
    let mut fields = Vec::new();
    let mut cur = row;
    loop {
        match cur {
            Row::REmpty => return (fields, None),
            Row::RVar(v) => return (fields, Some(*v)),
            Row::RExtend {
                label,
                ty,
                optional,
                tail,
            } => {
                fields.push((label.as_str(), *ty, *optional));
                cur = tail;
            }
        }
    }
}

/// Deep structural equality by dereferencing arena IDs (post-normalization).
/// Row comparison is order-insensitive (permutation by label). Binder α-equivalence
/// (`∀a.F(a)` ≡ `∀b.F(b)`) is NOT implemented — binders must be syntactically identical.
pub(super) fn types_equal_deep(arena: &Arena<TlcType>, a: TlcTypeId, b: TlcTypeId) -> bool {
    if a == b {
        return true; // fast path: same index
    }
    match (arena[a].clone(), arena[b].clone()) {
        (TlcType::Prim(pa), TlcType::Prim(pb)) => pa == pb,
        (TlcType::Opaque(a), TlcType::Opaque(b)) => a == b,
        (TlcType::Singleton(la), TlcType::Singleton(lb)) => la == lb,
        (TlcType::TyVar(va, ka), TlcType::TyVar(vb, kb)) => va == vb && ka == kb,
        (TlcType::Fun(f1, t1, e1), TlcType::Fun(f2, t2, e2)) => {
            types_equal_deep(arena, f1, f2)
                && types_equal_deep(arena, t1, t2)
                && rows_equal_deep(arena, &e1, &e2)
        }
        (TlcType::List(i1), TlcType::List(i2)) => types_equal_deep(arena, i1, i2),
        (TlcType::Optional(i1), TlcType::Optional(i2)) => types_equal_deep(arena, i1, i2),
        (TlcType::Maybe(i1), TlcType::Maybe(i2)) => types_equal_deep(arena, i1, i2),
        (TlcType::TyApp(f1, a1), TlcType::TyApp(f2, a2)) => {
            types_equal_deep(arena, f1, f2) && types_equal_deep(arena, a1, a2)
        }
        (TlcType::TyLamK(v1, k1, b1), TlcType::TyLamK(v2, k2, b2)) => {
            v1 == v2 && k1 == k2 && types_equal_deep(arena, b1, b2)
        }
        (TlcType::ForAll(v1, k1, b1), TlcType::ForAll(v2, k2, b2)) => {
            v1 == v2 && k1 == k2 && types_equal_deep(arena, b1, b2)
        }
        (TlcType::Record(r1), TlcType::Record(r2)) => rows_equal_deep(arena, &r1, &r2),
        (TlcType::VariantT(r1), TlcType::VariantT(r2)) => rows_equal_deep(arena, &r1, &r2),
        (TlcType::Tuple(i1), TlcType::Tuple(i2)) => {
            i1.len() == i2.len()
                && i1.iter().zip(i2.iter()).all(|(a, b)| match (a, b) {
                    (TlcTupleField::Positional(ta), TlcTupleField::Positional(tb)) => {
                        types_equal_deep(arena, *ta, *tb)
                    }
                    (
                        TlcTupleField::Named { name: na, ty: ta },
                        TlcTupleField::Named { name: nb, ty: tb },
                    ) => na == nb && types_equal_deep(arena, *ta, *tb),
                    _ => false,
                })
        }
        _ => false,
    }
}

/// Order-insensitive (permutation by label) row equality (post-normalization).
///
/// Two rows are equal iff they have the same label set (matching `optional` and field type) and
/// the same tail (`REmpty` or the same `RVar`). No α-equivalence over quantified row variables.
pub(super) fn rows_equal_deep(arena: &Arena<TlcType>, a: &Row, b: &Row) -> bool {
    let (fields_a, tail_a) = row_fields_and_tail(a);
    let (fields_b, tail_b) = row_fields_and_tail(b);

    // Tails must match (both closed, or same row variable).
    if tail_a != tail_b {
        return false;
    }
    if fields_a.len() != fields_b.len() {
        return false;
    }

    // Build a label → (ty, optional) map for b's fields.
    let map_b: FxHashMap<&str, (TlcTypeId, bool)> = fields_b
        .iter()
        .map(|&(l, ty, opt)| (l, (ty, opt)))
        .collect();

    fields_a.iter().all(|&(label, ty_a, opt_a)| {
        if let Some(&(ty_b, opt_b)) = map_b.get(label) {
            opt_a == opt_b && types_equal_deep(arena, ty_a, ty_b)
        } else {
            false
        }
    })
}
