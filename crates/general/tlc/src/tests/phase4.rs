// ── Phase 4 tests: effect row on Fun + effect-row eraser ─────────────────────

use super::{make_module, tlc_of};
use crate::*;

/// Every v0 function lowers to `Fun(_, _, Row::REmpty)` — invariant #10 holds vacuously:
/// no v0 path produces a non-empty effect row, so "erase before DC" is a no-op.
#[test]
fn v0_function_lowers_with_empty_effect_row() {
    let m = tlc_of("id :: Int -> Int = \\x. x;\nid 1");
    let TlcDecl::Value { ty, .. } = &m.decl_arena[m.decls[0]] else {
        panic!("expected Value decl")
    };
    let TlcType::Fun(_, _, ref eff) = m.type_arena[*ty] else {
        panic!("expected Fun, got {:?}", m.type_arena[*ty])
    };
    assert_eq!(
        *eff,
        Row::REmpty,
        "v0 function must have REmpty effect row (invariant #10 vacuous), got {:?}",
        eff
    );
}

/// A non-empty effect row (arbitrary placeholder labels/types — *not* the final effect
/// encoding, which requires the deferred free-monad elaboration) survives `normalize`.
/// This proves `normalize_ty` threads the field rather than dropping it.
#[test]
fn effect_row_survives_normalize() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    // Placeholder effect row: RExtend("e", Bool, REmpty).
    // Label "e" and type Bool are arbitrary stand-ins — effect entry contents are
    // not yet pinned (Kind::Effect is deferred with free-monad elaboration).
    let bool_placeholder = type_arena.alloc(TlcType::Prim(PrimTy::Bool));
    let eff_row = Row::RExtend {
        label: "e".to_string(),
        ty: bool_placeholder,
        optional: false,
        tail: Box::new(Row::REmpty),
    };
    let fun_ty = type_arena.alloc(TlcType::Fun(int_ty, int_ty, eff_row));
    let mut m = make_module(type_arena);
    let norm = m.normalize(fun_ty).expect("normalize must not fail");
    let TlcType::Fun(_, _, ref eff_after) = m.type_arena[norm] else {
        panic!("expected Fun after normalize, got {:?}", m.type_arena[norm])
    };
    match eff_after {
        Row::RExtend { label, .. } => {
            assert_eq!(label, "e", "effect row label must survive normalization");
        }
        _ => panic!(
            "expected RExtend in effect row after normalize, got {:?}",
            eff_after
        ),
    }
}

/// Two functions identical in `from` and `to` but differing in their effect row
/// must NOT be equal. This is the discriminating test: `REmpty→REmpty` alone cannot
/// distinguish "field compared" from "field ignored"; a non-empty vs. empty row can.
#[test]
fn functions_differing_only_in_effect_row_are_unequal() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    let bool_placeholder = type_arena.alloc(TlcType::Prim(PrimTy::Bool));
    let pure_fun = type_arena.alloc(TlcType::Fun(int_ty, int_ty, Row::REmpty));
    let effectful_fun = type_arena.alloc(TlcType::Fun(
        int_ty,
        int_ty,
        Row::RExtend {
            label: "e".to_string(),
            ty: bool_placeholder,
            optional: false,
            tail: Box::new(Row::REmpty),
        },
    ));
    let mut m = make_module(type_arena);
    assert!(
        !m.types_equal(pure_fun, effectful_fun),
        "Fun with REmpty effect row must not equal Fun with non-empty effect row"
    );
}

/// Effect rows compare order-insensitively (they model a set of effects).
/// `Fun(A, B, {e1, e2})` must equal `Fun(A, B, {e2, e1})`.
/// Labels "e1"/"e2" and placeholder type `Bool` are arbitrary — effect entry
/// contents are not yet pinned (deferred with free-monad elaboration).
#[test]
fn effect_row_permutation_equality_holds() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    let bool_placeholder = type_arena.alloc(TlcType::Prim(PrimTy::Bool));
    // Fun(Int, Int, {e1: Bool, e2: Bool})
    let fun_e1e2 = type_arena.alloc(TlcType::Fun(
        int_ty,
        int_ty,
        Row::RExtend {
            label: "e1".to_string(),
            ty: bool_placeholder,
            optional: false,
            tail: Box::new(Row::RExtend {
                label: "e2".to_string(),
                ty: bool_placeholder,
                optional: false,
                tail: Box::new(Row::REmpty),
            }),
        },
    ));
    // Fun(Int, Int, {e2: Bool, e1: Bool}) — reversed order
    let fun_e2e1 = type_arena.alloc(TlcType::Fun(
        int_ty,
        int_ty,
        Row::RExtend {
            label: "e2".to_string(),
            ty: bool_placeholder,
            optional: false,
            tail: Box::new(Row::RExtend {
                label: "e1".to_string(),
                ty: bool_placeholder,
                optional: false,
                tail: Box::new(Row::REmpty),
            }),
        },
    ));
    let mut m = make_module(type_arena);
    assert!(
        m.types_equal(fun_e1e2, fun_e2e1),
        "effect rows must compare order-insensitively: {{e1,e2}} ≡ {{e2,e1}}"
    );
}

/// `erase_effects` is a no-op on a module where every `Fun` already has `REmpty`.
/// This is the invariant that holds for all v0 programs.
#[test]
fn effect_eraser_is_noop_for_pure_functions() {
    let mut m = tlc_of("id :: Int -> Int = \\x. x;\nid 1");
    // Precondition: v0 lowering sets every Fun eff to REmpty already.
    let before: Vec<Row> = m
        .type_arena
        .iter()
        .filter_map(|(_, ty)| {
            if let TlcType::Fun(_, _, eff) = ty {
                Some(eff.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(
        before.iter().all(|r| *r == Row::REmpty),
        "precondition: all v0 Fun effect rows must already be REmpty before erasing"
    );
    m.erase_effects();
    let after: Vec<Row> = m
        .type_arena
        .iter()
        .filter_map(|(_, ty)| {
            if let TlcType::Fun(_, _, eff) = ty {
                Some(eff.clone())
            } else {
                None
            }
        })
        .collect();
    // No-op: the two snapshots must be identical.
    assert_eq!(
        before, after,
        "erase_effects must be a no-op when all Fun rows are already REmpty"
    );
}

/// `erase_effects` clears a non-empty effect row: any `Fun(A, B, {e: T})` becomes
/// `Fun(A, B, REmpty)`. This tests the active path through the eraser — the only
/// way to reach it in v0 is via a hand-built `TlcModule`.
#[test]
fn effect_eraser_clears_nonempty_effect_row() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    let bool_placeholder = type_arena.alloc(TlcType::Prim(PrimTy::Bool));
    // Fun(Int, Int, {e: Bool}) — non-empty effect row.
    let effectful_id = type_arena.alloc(TlcType::Fun(
        int_ty,
        int_ty,
        Row::RExtend {
            label: "e".to_string(),
            ty: bool_placeholder,
            optional: false,
            tail: Box::new(Row::REmpty),
        },
    ));
    let mut m = make_module(type_arena);
    // Before: the Fun has a non-empty effect row.
    assert!(
        matches!(
            &m.type_arena[effectful_id],
            TlcType::Fun(_, _, Row::RExtend { .. })
        ),
        "precondition: Fun must have non-empty eff row before erasing"
    );
    m.erase_effects();
    // After: every Fun effect row must be REmpty.
    assert!(
        matches!(&m.type_arena[effectful_id], TlcType::Fun(_, _, Row::REmpty)),
        "Fun effect row must be REmpty after erase_effects, got {:?}",
        m.type_arena[effectful_id]
    );
}
