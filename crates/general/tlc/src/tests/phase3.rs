// ── Phase 3 tests: row kind + row polymorphism + capture-avoiding substitution ─

use super::make_module;
use crate::*;

#[test]
fn open_record_rvar_is_inert_under_normalize() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let rest_var = TlcTypeVar::Named(0);
    let text_ty = type_arena.alloc(TlcType::Prim(PrimTy::Str));
    let record_ty = type_arena.alloc(TlcType::Record(Row::RExtend {
        label: "host".to_string(),
        ty: text_ty,
        optional: false,
        tail: Box::new(Row::RVar(rest_var)),
    }));
    let mut m = make_module(type_arena);
    let norm = m
        .normalize(record_ty)
        .expect("open record normalize must not fail");
    assert!(
        matches!(m.type_arena[norm], TlcType::Record(_)),
        "expected Record, got {:?}",
        m.type_arena[norm]
    );
    if let TlcType::Record(ref row) = m.type_arena[norm] {
        match row {
            Row::RExtend { label, tail, .. } => {
                assert_eq!(label, "host");
                assert!(
                    matches!(**tail, Row::RVar(_)),
                    "RVar tail must survive normalization, got {:?}",
                    tail
                );
            }
            _ => panic!("expected RExtend as top of row after normalize"),
        }
    }
}

#[test]
fn subst_row_var_flattens_open_to_closed() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let rest_var = TlcTypeVar::Named(0);
    let text_ty = type_arena.alloc(TlcType::Prim(PrimTy::Str));
    // Open row: { host : Text, ...rest }
    let open_row = Row::RExtend {
        label: "host".to_string(),
        ty: text_ty,
        optional: false,
        tail: Box::new(Row::RVar(rest_var)),
    };
    // Splice in REmpty to close the row.
    let closed = open_row.subst_row_var(rest_var, Row::REmpty);
    match closed {
        Row::RExtend {
            ref label,
            ref tail,
            ..
        } => {
            assert_eq!(label, "host");
            assert_eq!(
                **tail,
                Row::REmpty,
                "tail must be REmpty after subst_row_var"
            );
        }
        _ => panic!("expected RExtend after subst_row_var, got {:?}", closed),
    }
}

#[test]
fn row_permutation_equality_holds() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    let str_ty = type_arena.alloc(TlcType::Prim(PrimTy::Str));
    // { a : Int, b : Str }
    let record_ab = type_arena.alloc(TlcType::Record(Row::from_record_fields([
        ("a".to_string(), int_ty, false),
        ("b".to_string(), str_ty, false),
    ])));
    // { b : Str, a : Int } — reversed order
    let record_ba = type_arena.alloc(TlcType::Record(Row::from_record_fields([
        ("b".to_string(), str_ty, false),
        ("a".to_string(), int_ty, false),
    ])));
    let mut m = make_module(type_arena);
    assert!(
        m.types_equal(record_ab, record_ba),
        "row permutation equality must hold: {{a,b}} ≡ {{b,a}}"
    );
}

#[test]
fn row_field_type_mismatch_is_unequal() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    let str_ty = type_arena.alloc(TlcType::Prim(PrimTy::Str));
    let record_int = type_arena.alloc(TlcType::Record(Row::from_record_fields([(
        "a".to_string(),
        int_ty,
        false,
    )])));
    let record_str = type_arena.alloc(TlcType::Record(Row::from_record_fields([(
        "a".to_string(),
        str_ty,
        false,
    )])));
    let mut m = make_module(type_arena);
    assert!(
        !m.types_equal(record_int, record_str),
        "field type mismatch must not be equal: {{a:Int}} ≢ {{a:Str}}"
    );
}

#[test]
fn forall_row_kinded_var_roundtrips() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let rest_var = TlcTypeVar::Named(42);
    let row_kind = Kind::Row(Box::new(Kind::ground()));
    // ForAll(rest, Kind::Row(Type(0)), Record(RVar(rest)))
    let open_record = type_arena.alloc(TlcType::Record(Row::RVar(rest_var)));
    let forall_ty = type_arena.alloc(TlcType::ForAll(rest_var, row_kind.clone(), open_record));
    let mut m = make_module(type_arena);
    let norm = m
        .normalize(forall_ty)
        .expect("ForAll with row kind should normalize without fuel error");
    assert!(
        matches!(&m.type_arena[norm], TlcType::ForAll(_, k, _) if *k == row_kind),
        "ForAll with row kind must preserve Kind::Row, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn optional_record_field_roundtrips() {
    use la_arena::Arena;
    let mut type_arena: Arena<TlcType> = Arena::new();
    let int_ty = type_arena.alloc(TlcType::Prim(PrimTy::Int));
    // Record with one optional field: { age? : Int }
    let record_ty = type_arena.alloc(TlcType::Record(Row::RExtend {
        label: "age".to_string(),
        ty: int_ty,
        optional: true,
        tail: Box::new(Row::REmpty),
    }));
    let mut m = make_module(type_arena);
    let norm = m
        .normalize(record_ty)
        .expect("optional record must normalize without error");
    assert!(
        matches!(m.type_arena[norm], TlcType::Record(_)),
        "expected Record after normalize, got {:?}",
        m.type_arena[norm]
    );
    if let TlcType::Record(ref row) = m.type_arena[norm] {
        match row {
            Row::RExtend {
                label, optional, ..
            } => {
                assert_eq!(label, "age");
                assert!(*optional, "optional flag must survive normalization");
            }
            _ => panic!("expected RExtend, got {:?}", row),
        }
    }
}

/// `(λa. ∀b. a) b` — the binder `b` in `∀b` would capture the free `b` in the
/// replacement if substitution were naive. The normalizer must freshen the `ForAll`
/// binder before descending, so the result is `∀b'. b` where `b' ≠ b`.
#[test]
fn subst_is_capture_avoiding_for_forall_binder() {
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let a = TlcTypeVar::Named(1);
    let b = TlcTypeVar::Named(2);
    let kind = Kind::ground();
    let tyvar_a = ta.alloc(TlcType::TyVar(a, kind.clone()));
    let tyvar_b = ta.alloc(TlcType::TyVar(b, kind.clone()));
    // Body of TyLamK: ∀b. TyVar(a)  — b shadows a different var, a is free inside
    let forall_b_a = ta.alloc(TlcType::ForAll(b, kind.clone(), tyvar_a));
    // Full type: λa. ∀b. a
    let outer_lam = ta.alloc(TlcType::TyLamK(a, kind, forall_b_a));
    // Application: (λa. ∀b. a) b  — substitute a := TyVar(b) under ∀b
    let app = ta.alloc(TlcType::TyApp(outer_lam, tyvar_b));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize must succeed");
    // After capture-avoiding subst: result is ∀b'. b  where b' is a fresh var ≠ b
    match &m.type_arena[norm] {
        TlcType::ForAll(new_binder, _, inner) => {
            assert_ne!(*new_binder, b, "binder must be freshened to avoid capture");
            assert!(
                matches!(m.type_arena[*inner], TlcType::TyVar(v, _) if v == b),
                "inner must be TyVar(b) after capture-avoiding subst, got {:?}",
                m.type_arena[*inner]
            );
        }
        other => panic!("expected ForAll after normalization, got {:?}", other),
    }
}

/// `(λa. λb. a) b` — the inner `λb` would capture the free `b` in the replacement
/// if substitution were naive. The normalizer must freshen the inner `TyLamK` binder.
#[test]
fn subst_is_capture_avoiding_for_tylam_binder() {
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let a = TlcTypeVar::Named(1);
    let b = TlcTypeVar::Named(2);
    let kind = Kind::ground();
    let tyvar_a = ta.alloc(TlcType::TyVar(a, kind.clone()));
    let tyvar_b = ta.alloc(TlcType::TyVar(b, kind.clone()));
    // Inner lambda: λb. TyVar(a)
    let inner_lam = ta.alloc(TlcType::TyLamK(b, kind.clone(), tyvar_a));
    // Outer lambda: λa. λb. a
    let outer_lam = ta.alloc(TlcType::TyLamK(a, kind, inner_lam));
    // Application: (λa. λb. a) b  — substitute a := TyVar(b) under λb
    let app = ta.alloc(TlcType::TyApp(outer_lam, tyvar_b));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize must succeed");
    // After capture-avoiding subst: result is λb'. b  where b' is a fresh var ≠ b
    match &m.type_arena[norm] {
        TlcType::TyLamK(new_binder, _, inner) => {
            assert_ne!(*new_binder, b, "binder must be freshened to avoid capture");
            assert!(
                matches!(m.type_arena[*inner], TlcType::TyVar(v, _) if v == b),
                "inner must be TyVar(b) after capture-avoiding subst, got {:?}",
                m.type_arena[*inner]
            );
        }
        other => panic!("expected TyLamK after normalization, got {:?}", other),
    }
}
