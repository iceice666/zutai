// ── types_equal_deep: direct structural equality tests ───────────────────────

use super::make_module;
use crate::*;

#[test]
fn types_equal_deep_list_arm() {
    // Two separately-allocated List(Int) nodes have distinct arena indices
    // (defeating the a==b fast path) but must be structurally equal.
    // This exercises the List arm of types_equal_deep.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let list_a = ta.alloc(TlcType::List(int_a));
    let list_b = ta.alloc(TlcType::List(int_b));
    // list_a and list_b have different arena indices — fast-path a==b is false.
    assert_ne!(
        list_a, list_b,
        "indices must differ to exercise types_equal_deep"
    );
    let mut m = make_module(ta);
    assert!(
        m.types_equal(list_a, list_b),
        "List(Int) == List(Int) structurally"
    );
}

#[test]
fn types_equal_deep_optional_arm() {
    // Two separately-allocated Optional(Int) nodes — exercises the Optional arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let opt_a = ta.alloc(TlcType::Optional(int_a));
    let opt_b = ta.alloc(TlcType::Optional(int_b));
    assert_ne!(opt_a, opt_b, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(opt_a, opt_b),
        "Optional(Int) == Optional(Int) structurally"
    );
}

#[test]
fn types_equal_deep_tyvar_arm() {
    // Two separately-allocated TyVar(Named(5), ground) nodes — exercises the TyVar arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let tv_a = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let tv_b = ta.alloc(TlcType::TyVar(var, kind));
    assert_ne!(tv_a, tv_b, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(tv_a, tv_b),
        "TyVar(5) == TyVar(5) structurally"
    );
}

#[test]
fn types_equal_deep_forall_arm() {
    // Two separately-allocated ForAll(5, ground, Int) nodes — exercises the ForAll arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let fa = ta.alloc(TlcType::ForAll(var, kind.clone(), int_a));
    let fb = ta.alloc(TlcType::ForAll(var, kind, int_b));
    assert_ne!(fa, fb, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(fa, fb),
        "ForAll(5, ground, Int) == ForAll(5, ground, Int) structurally"
    );
}

#[test]
fn types_equal_deep_tylam_arm() {
    // Two separately-allocated TyLamK(5, ground, Int) nodes — exercises the TyLamK arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let la = ta.alloc(TlcType::TyLamK(var, kind.clone(), int_a));
    let lb = ta.alloc(TlcType::TyLamK(var, kind, int_b));
    assert_ne!(la, lb, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(la, lb),
        "TyLamK(5, ground, Int) == TyLamK(5, ground, Int) structurally"
    );
}

#[test]
fn types_equal_deep_tyapp_arm() {
    // Two separately-allocated TyApp(List, Int) nodes — exercises the TyApp arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let str_a = ta.alloc(TlcType::Prim(PrimTy::Str));
    let str_b = ta.alloc(TlcType::Prim(PrimTy::Str));
    // TyApp(Int, Str) — not semantically meaningful but structurally testable.
    let app_a = ta.alloc(TlcType::TyApp(int_a, str_a));
    let app_b = ta.alloc(TlcType::TyApp(int_b, str_b));
    assert_ne!(app_a, app_b, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(app_a, app_b),
        "TyApp(Int, Str) == TyApp(Int, Str) structurally"
    );
}

#[test]
fn types_equal_deep_tuple_arm() {
    // Two separately-allocated Tuple([Positional(Int), Positional(Bool)]) — Tuple arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_a = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let bool_b = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let tup_a = ta.alloc(TlcType::Tuple(vec![
        TlcTupleField::Positional(int_a),
        TlcTupleField::Positional(bool_a),
    ]));
    let tup_b = ta.alloc(TlcType::Tuple(vec![
        TlcTupleField::Positional(int_b),
        TlcTupleField::Positional(bool_b),
    ]));
    assert_ne!(tup_a, tup_b, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(tup_a, tup_b),
        "Tuple([Int, Bool]) == Tuple([Int, Bool]) structurally"
    );
}

#[test]
fn types_equal_deep_fun_arm() {
    // Two separately-allocated Fun(Int, Bool, REmpty) — Fun arm (with effect row).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_a = ta.alloc(TlcType::Prim(PrimTy::Int));
    let int_b = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_a = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let bool_b = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let fun_a = ta.alloc(TlcType::Fun(int_a, bool_a, Row::REmpty));
    let fun_b = ta.alloc(TlcType::Fun(int_b, bool_b, Row::REmpty));
    assert_ne!(fun_a, fun_b, "indices must differ");
    let mut m = make_module(ta);
    assert!(
        m.types_equal(fun_a, fun_b),
        "Fun(Int, Bool, REmpty) == Fun(Int, Bool, REmpty) structurally"
    );
}

#[test]
fn types_equal_distinguishes_concrete_universe_levels() {
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let ground = ta.alloc(TlcType::TyVar(var, Kind::Type(0)));
    let higher = ta.alloc(TlcType::TyVar(var, Kind::Type(1)));
    let mut m = make_module(ta);

    assert!(!m.types_equal(ground, higher));
}
