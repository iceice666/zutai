// ── Normalize (hand-built IR): subst + β-reduction tests ─────────────────────

use super::make_module;
use crate::*;

#[test]
fn subst_replaces_matching_tyvar() {
    // TyApp(TyLamK(var=5, _, body=TyVar(5)), Int) β-reduces to Int.
    // Exercises: subst TyVar(v == var) → replacement.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let body = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, body));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize must succeed");
    assert!(
        matches!(m.type_arena[norm], TlcType::Prim(PrimTy::Int)),
        "TyVar(5) should be replaced with Int, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_leaves_non_matching_tyvar() {
    // TyApp(TyLamK(var=5, _, body=TyVar(6)), Int) → body unchanged: TyVar(6).
    // Exercises: subst TyVar(v != var) → ty.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let other = TlcTypeVar::Named(6);
    let kind = Kind::ground();
    let body = ta.alloc(TlcType::TyVar(other, kind.clone()));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, body));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize must succeed");
    // Result should be TyVar(6) since var=5 was substituted but body had var=6.
    assert!(
        matches!(m.type_arena[norm], TlcType::TyVar(TlcTypeVar::Named(6), _)),
        "non-matching TyVar should survive substitution, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_stops_at_shadowed_tylam() {
    // TyApp(TyLamK(var=5, _, body=TyLamK(var=5, _, inner=TyVar(5))), Int)
    // The inner TyLamK re-binds var=5 — subst must NOT descend.
    // Result: TyLamK(5, _, TyVar(5)) (inner lam body unchanged).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let inner_var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    // Inner TyLamK(var=5, _, TyVar(5)) — rebinds var
    let inner_lam = ta.alloc(TlcType::TyLamK(var, kind.clone(), inner_var_ref));
    // Outer TyLamK(var=5, _, inner_lam)
    let outer_lam = ta.alloc(TlcType::TyLamK(var, kind, inner_lam));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(outer_lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    // After β-reducing outer: subst(inner_lam, var=5, Int) → inner_lam unchanged (shadowed)
    // norm = TyLamK(5, _, TyVar(5))
    assert!(
        matches!(
            m.type_arena[norm],
            TlcType::TyLamK(TlcTypeVar::Named(5), _, _)
        ),
        "shadowed TyLamK should not be changed by subst, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_recurses_into_non_shadowed_tylam() {
    // TyApp(TyLamK(var=5, _, body=TyLamK(var=6, _, inner=TyVar(5))), Int)
    // Inner TyLamK binds var=6 (≠5), so subst DOES recurse → TyVar(5) → Int.
    // Result: TyLamK(6, _, Int).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let other = TlcTypeVar::Named(6);
    let kind = Kind::ground();
    let inner_var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    // Inner TyLamK(var=6, _, TyVar(5)) — binds different var
    let inner_lam = ta.alloc(TlcType::TyLamK(other, kind.clone(), inner_var_ref));
    // Outer TyLamK(var=5, _, inner_lam)
    let outer_lam = ta.alloc(TlcType::TyLamK(var, kind, inner_lam));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(outer_lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    // subst(TyLamK(6, _, TyVar(5)), var=5, Int) → TyLamK(6, _, Int)
    assert!(
        matches!(m.type_arena[norm], TlcType::TyLamK(TlcTypeVar::Named(6), _, inner)
            if matches!(m.type_arena[inner], TlcType::Prim(PrimTy::Int))),
        "non-shadowed TyLamK should have its body subst'd, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_recurses_into_list_and_optional() {
    // TyApp(TyLamK(var=5, _, body=List(TyVar(5))), Int) → List(Int).
    // Exercises: subst List(inner) arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let list_body = ta.alloc(TlcType::List(var_ref));
    let lam = ta.alloc(TlcType::TyLamK(var, kind.clone(), list_body));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::List(inner) if matches!(m.type_arena[inner], TlcType::Prim(PrimTy::Int))),
        "subst into List should produce List(Int), got {:?}",
        m.type_arena[norm]
    );

    // Similarly: TyApp(TyLamK(var=5, _, Optional(TyVar(5))), Int) → Optional(Int).
    let mut ta2: Arena<TlcType> = Arena::new();
    let var2 = TlcTypeVar::Named(5);
    let var_ref2 = ta2.alloc(TlcType::TyVar(var2, kind.clone()));
    let opt_body = ta2.alloc(TlcType::Optional(var_ref2));
    let lam2 = ta2.alloc(TlcType::TyLamK(var2, kind, opt_body));
    let int_ty2 = ta2.alloc(TlcType::Prim(PrimTy::Int));
    let app2 = ta2.alloc(TlcType::TyApp(lam2, int_ty2));
    let mut m2 = make_module(ta2);
    let norm2 = m2.normalize(app2).expect("normalize optional");
    assert!(
        matches!(m2.type_arena[norm2], TlcType::Optional(inner) if matches!(m2.type_arena[inner], TlcType::Prim(PrimTy::Int))),
        "subst into Optional should produce Optional(Int), got {:?}",
        m2.type_arena[norm2]
    );
}

#[test]
fn subst_recurses_into_fun() {
    // TyApp(TyLamK(var=5, _, Fun(TyVar(5), TyVar(5), REmpty)), Bool) → Fun(Bool, Bool, REmpty).
    // Exercises: subst Fun(from, to, eff) arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let fun_body = ta.alloc(TlcType::Fun(var_ref, var_ref, Row::REmpty));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, fun_body));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let app = ta.alloc(TlcType::TyApp(lam, bool_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::Fun(f, t, Row::REmpty)
            if matches!(m.type_arena[f], TlcType::Prim(PrimTy::Bool))
            && matches!(m.type_arena[t], TlcType::Prim(PrimTy::Bool))),
        "subst into Fun should produce Fun(Bool, Bool, REmpty), got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_recurses_into_record_and_variantt() {
    // TyApp(TyLamK(var=5, _, Record({x: TyVar(5)})), Int) → Record({x: Int}).
    // Exercises: subst Record(row) and subst_row RExtend arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let record_body = ta.alloc(TlcType::Record(Row::from_record_fields([(
        "x".to_string(),
        var_ref,
        false,
    )])));
    let lam = ta.alloc(TlcType::TyLamK(var, kind.clone(), record_body));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize record");
    assert!(
        matches!(&m.type_arena[norm], TlcType::Record(_)),
        "subst into Record should produce Record, got {:?}",
        m.type_arena[norm]
    );

    // VariantT: TyApp(TyLamK(var=5, _, VariantT({ok: TyVar(5)})), Bool) → VariantT({ok: Bool}).
    // Exercises: subst VariantT(row) arm.
    let mut ta2: Arena<TlcType> = Arena::new();
    let var2 = TlcTypeVar::Named(5);
    let var_ref2 = ta2.alloc(TlcType::TyVar(var2, kind.clone()));
    let variant_body = ta2.alloc(TlcType::VariantT(Row::from_fields([(
        "ok".to_string(),
        var_ref2,
    )])));
    let lam2 = ta2.alloc(TlcType::TyLamK(var2, kind, variant_body));
    let bool_ty = ta2.alloc(TlcType::Prim(PrimTy::Bool));
    let app2 = ta2.alloc(TlcType::TyApp(lam2, bool_ty));
    let mut m2 = make_module(ta2);
    let norm2 = m2.normalize(app2).expect("normalize variantt");
    assert!(
        matches!(&m2.type_arena[norm2], TlcType::VariantT(_)),
        "subst into VariantT should produce VariantT, got {:?}",
        m2.type_arena[norm2]
    );
}

#[test]
fn subst_recurses_into_tuple_positional_and_named() {
    // Exercises: subst Tuple(items) arm, TlcTupleField::Positional and ::Named.
    use la_arena::Arena;
    let kind = Kind::ground();

    // Positional: TyApp(TyLamK(var=5, _, Tuple([Positional(TyVar(5))])), Int) → Tuple([Positional(Int)]).
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let tuple_body = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Positional(var_ref)]));
    let lam = ta.alloc(TlcType::TyLamK(var, kind.clone(), tuple_body));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize tuple positional");
    assert!(
        matches!(&m.type_arena[norm], TlcType::Tuple(items)
            if matches!(items.as_slice(), [TlcTupleField::Positional(inner)]
                if matches!(m.type_arena[*inner], TlcType::Prim(PrimTy::Int)))),
        "subst into Tuple(Positional) should produce Tuple([Positional(Int)]), got {:?}",
        m.type_arena[norm]
    );

    // Named: TyApp(TyLamK(var=5, _, Tuple([Named{x, TyVar(5)}])), Bool) → Tuple([Named{x, Bool}]).
    let mut ta2: Arena<TlcType> = Arena::new();
    let var2 = TlcTypeVar::Named(5);
    let var_ref2 = ta2.alloc(TlcType::TyVar(var2, kind.clone()));
    let named_tuple = ta2.alloc(TlcType::Tuple(vec![TlcTupleField::Named {
        name: "x".to_string(),
        ty: var_ref2,
    }]));
    let lam2 = ta2.alloc(TlcType::TyLamK(var2, kind, named_tuple));
    let bool_ty = ta2.alloc(TlcType::Prim(PrimTy::Bool));
    let app2 = ta2.alloc(TlcType::TyApp(lam2, bool_ty));
    let mut m2 = make_module(ta2);
    let norm2 = m2.normalize(app2).expect("normalize tuple named");
    assert!(
        matches!(&m2.type_arena[norm2], TlcType::Tuple(items)
            if matches!(items.as_slice(), [TlcTupleField::Named { name, ty }]
                if name == "x" && matches!(m2.type_arena[*ty], TlcType::Prim(PrimTy::Bool)))),
        "subst into Tuple(Named) should produce Tuple([Named{{x, Bool}}]), got {:?}",
        m2.type_arena[norm2]
    );
}

#[test]
fn normalize_tuple_type_normalizes_fields() {
    // TyLamK(var=5, _, Tuple([Pos(TyVar(5)), Pos(TyVar(5))])) applied to Int
    // → Tuple([Pos(Int), Pos(Int)]).
    // Exercises: normalize_tuple_fields.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let var_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let tuple_body = ta.alloc(TlcType::Tuple(vec![
        TlcTupleField::Positional(var_ref),
        TlcTupleField::Positional(var_ref),
    ]));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, tuple_body));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize tuple");
    assert!(
        matches!(&m.type_arena[norm], TlcType::Tuple(items) if items.len() == 2),
        "normalized Tuple should have 2 fields, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn normalize_tyapp_third_arm_non_reducible_head() {
    // TyApp(Fun(Int, Int, REmpty), Bool) — head normalizes to Fun (not TyLamK or alias TyVar).
    // After normalizing both sides, head is Fun → hits the _ fallback arm.
    // Result: TyApp(Fun(Int, Int, REmpty), Bool) — irreducible TyApp preserved.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let fun_ty = ta.alloc(TlcType::Fun(int_ty, int_ty, Row::REmpty));
    // TyApp(Fun(Int,Int,REmpty), Bool) — weird but legal IR node.
    let app = ta.alloc(TlcType::TyApp(fun_ty, bool_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize non-reducible head");
    // After normalizing: head stays Fun (not lambda or alias), so _ arm → TyApp preserved.
    assert!(
        matches!(m.type_arena[norm], TlcType::TyApp(_, _)),
        "non-reducible TyApp head should stay as TyApp, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn normalize_tylam_body_recurses() {
    // TyLamK(var=5, _, TyVar(7)) where TyVar(7) is just itself (no alias).
    // normalize_ty for TyLamK → normalizes body (TyVar is atomic, returns itself).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let body_var = TlcTypeVar::Named(7);
    let kind = Kind::ground();
    let body = ta.alloc(TlcType::TyVar(body_var, kind.clone()));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, body));
    let mut m = make_module(ta);
    let norm = m.normalize(lam).expect("normalize TyLamK");
    assert!(
        matches!(
            m.type_arena[norm],
            TlcType::TyLamK(TlcTypeVar::Named(5), _, _)
        ),
        "TyLamK should survive normalization with its binder intact, got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_forall_shadowed_does_not_descend() {
    // TyApp(TyLamK(var=5, _, ForAll(var=5, _, TyVar(5))), Bool)
    // The TyLamK's β-reduction substitutes var=5, but ForAll(5) shadows it,
    // so the body TyVar(5) is NOT replaced.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let tvar_ref = ta.alloc(TlcType::TyVar(var, kind.clone()));
    // ForAll(5, _, TyVar(5)) — forall that shadows binder 5
    let forall_body = ta.alloc(TlcType::ForAll(var, kind.clone(), tvar_ref));
    // TyLamK(5, _, ForAll(5, _, TyVar(5)))
    let lam = ta.alloc(TlcType::TyLamK(var, kind, forall_body));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let app = ta.alloc(TlcType::TyApp(lam, bool_ty));
    let mut m = make_module(ta);
    // Normalize: β-reduce TyApp by substituting var=5 → Bool.
    // But inside the ForAll body the binder is shadowed, so TyVar(5) stays.
    // Result: ForAll(5, _, TyVar(5)) — the inner TyVar(5) is unchanged.
    let norm = m.normalize(app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::ForAll(v, _, _) if v == TlcTypeVar::Named(5)),
        "shadowed ForAll should remain unchanged; got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_forall_non_shadowed_substitutes_body() {
    // TyApp(TyLamK(var=5, _, ForAll(var=6, _, TyVar(5))), Bool)
    // ForAll(6) does NOT shadow var=5, so TyVar(5) → Bool.
    // Result: ForAll(6, _, Bool).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var5 = TlcTypeVar::Named(5);
    let var6 = TlcTypeVar::Named(6);
    let kind = Kind::ground();
    let tvar5_ref = ta.alloc(TlcType::TyVar(var5, kind.clone()));
    // ForAll(6, _, TyVar(5)) — forall with binder 6, body references var 5
    let forall_body = ta.alloc(TlcType::ForAll(var6, kind.clone(), tvar5_ref));
    // TyLamK(5, _, ForAll(6, _, TyVar(5)))
    let lam = ta.alloc(TlcType::TyLamK(var5, kind, forall_body));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let app = ta.alloc(TlcType::TyApp(lam, bool_ty));
    let mut m = make_module(ta);
    // Normalize: β-reduce, substituting TyVar(5) → Bool inside ForAll(6).
    // Result: ForAll(6, _, Bool).
    let norm = m.normalize(app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::ForAll(v, _, inner)
            if v == TlcTypeVar::Named(6) && matches!(m.type_arena[inner], TlcType::Prim(PrimTy::Bool))),
        "non-shadowed ForAll body should have TyVar substituted; got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_tyapp_recurses_into_both_sides() {
    // TyApp(TyLamK(5, _, TyApp(TyVar(5), TyVar(5))), Bool) →
    // TyApp(Bool, Bool).  Exercises subst TyApp arm on BOTH f and a.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    let tvar = ta.alloc(TlcType::TyVar(var, kind.clone()));
    // inner TyApp(TyVar(5), TyVar(5))
    let inner_app = ta.alloc(TlcType::TyApp(tvar, tvar));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, inner_app));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let outer_app = ta.alloc(TlcType::TyApp(lam, bool_ty));
    let mut m = make_module(ta);
    // Normalize β-reduces, calling subst on TyApp(TyVar(5), TyVar(5)).
    // Both f and a become Bool → result is TyApp(Bool, Bool).
    let norm = m.normalize(outer_app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::TyApp(f, a)
            if matches!(m.type_arena[f], TlcType::Prim(PrimTy::Bool))
            && matches!(m.type_arena[a], TlcType::Prim(PrimTy::Bool))),
        "subst TyApp should give TyApp(Bool, Bool); got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn subst_row_rvar_passthrough() {
    // TyApp(TyLamK(5, _, Record(RVar(99))), Bool) — the row variable 99 is not a
    // type variable and must pass through subst_row unchanged.
    // Result: Record(RVar(99)).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(5);
    let kind = Kind::ground();
    // A record with an open row tail (RVar).
    let rvar_row = Row::RVar(TlcTypeVar::Named(99));
    let record_ty = ta.alloc(TlcType::Record(rvar_row));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, record_ty));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let app = ta.alloc(TlcType::TyApp(lam, bool_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    // The RVar row variable is not a type var — subst_row passes it through.
    assert!(
        matches!(
            &m.type_arena[norm],
            TlcType::Record(Row::RVar(TlcTypeVar::Named(99)))
        ),
        "RVar should pass through subst_row unchanged; got {:?}",
        m.type_arena[norm]
    );
}

// ── types_equal_deep called from within normalize context ─────────────────────

#[test]
fn types_equal_deep_singleton_equality() {
    // Exercises: types_equal_deep Singleton arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let s1 = ta.alloc(TlcType::Singleton(Literal::Bool(true)));
    let s2 = ta.alloc(TlcType::Singleton(Literal::Bool(true)));
    let s3 = ta.alloc(TlcType::Singleton(Literal::Bool(false)));
    let mut m = make_module(ta);
    assert!(
        m.types_equal(s1, s2),
        "Singleton(Bool(true)) must equal Singleton(Bool(true))"
    );
    assert!(
        !m.types_equal(s1, s3),
        "Singleton(Bool(true)) must not equal Singleton(Bool(false))"
    );
}

#[test]
fn types_equal_deep_tyapp_equality() {
    // Exercises: types_equal_deep TyApp arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Named(99);
    let kind = Kind::ground();
    let f = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let app1 = ta.alloc(TlcType::TyApp(f, int_ty));
    let app2 = ta.alloc(TlcType::TyApp(f, int_ty));
    let app3 = ta.alloc(TlcType::TyApp(f, bool_ty));
    let mut m = make_module(ta);
    assert!(
        m.types_equal(app1, app2),
        "TyApp(F, Int) must equal TyApp(F, Int)"
    );
    assert!(
        !m.types_equal(app1, app3),
        "TyApp(F, Int) must not equal TyApp(F, Bool)"
    );
}

#[test]
fn types_equal_deep_tylam_equality() {
    // Exercises: types_equal_deep TyLamK arm.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var_a = TlcTypeVar::Named(1);
    let var_b = TlcTypeVar::Named(2);
    let kind = Kind::ground();
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let lam1 = ta.alloc(TlcType::TyLamK(var_a, kind.clone(), int_ty));
    let lam2 = ta.alloc(TlcType::TyLamK(var_a, kind.clone(), int_ty));
    let lam3 = ta.alloc(TlcType::TyLamK(var_b, kind.clone(), int_ty));
    let lam4 = ta.alloc(TlcType::TyLamK(var_a, kind, bool_ty));
    let mut m = make_module(ta);
    assert!(
        m.types_equal(lam1, lam2),
        "TyLamK(1,_,Int) must equal TyLamK(1,_,Int)"
    );
    assert!(
        !m.types_equal(lam1, lam3),
        "TyLamK(1,_,Int) must not equal TyLamK(2,_,Int)"
    );
    assert!(
        !m.types_equal(lam1, lam4),
        "TyLamK(1,_,Int) must not equal TyLamK(1,_,Bool)"
    );
}

#[test]
fn types_equal_deep_variantt_equality() {
    // Exercises: types_equal_deep VariantT arm (uses rows_equal_deep).
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));
    let vt1 = ta.alloc(TlcType::VariantT(Row::from_fields([(
        "ok".to_string(),
        int_ty,
    )])));
    let vt2 = ta.alloc(TlcType::VariantT(Row::from_fields([(
        "ok".to_string(),
        int_ty,
    )])));
    let vt3 = ta.alloc(TlcType::VariantT(Row::from_fields([(
        "ok".to_string(),
        bool_ty,
    )])));
    let mut m = make_module(ta);
    assert!(
        m.types_equal(vt1, vt2),
        "VariantT(ok:Int) must equal VariantT(ok:Int)"
    );
    assert!(
        !m.types_equal(vt1, vt3),
        "VariantT(ok:Int) must not equal VariantT(ok:Bool)"
    );
}

#[test]
fn types_equal_deep_tuple_positional_and_named() {
    // Exercises: types_equal_deep Tuple arm, Positional and Named fields, mismatch case.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let bool_ty = ta.alloc(TlcType::Prim(PrimTy::Bool));

    // Positional equality
    let t1 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Positional(int_ty)]));
    let t2 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Positional(int_ty)]));
    let t3 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Positional(bool_ty)]));
    // Named equality
    let t4 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Named {
        name: "x".to_string(),
        ty: int_ty,
    }]));
    let t5 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Named {
        name: "x".to_string(),
        ty: int_ty,
    }]));
    let t6 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Named {
        name: "y".to_string(),
        ty: int_ty,
    }]));
    // Mixed (Positional vs Named) → must not be equal
    let t7 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Positional(int_ty)]));
    let t8 = ta.alloc(TlcType::Tuple(vec![TlcTupleField::Named {
        name: "x".to_string(),
        ty: int_ty,
    }]));

    let mut m = make_module(ta);
    assert!(m.types_equal(t1, t2), "Positional(Int) == Positional(Int)");
    assert!(
        !m.types_equal(t1, t3),
        "Positional(Int) != Positional(Bool)"
    );
    assert!(m.types_equal(t4, t5), "Named{{x:Int}} == Named{{x:Int}}");
    assert!(!m.types_equal(t4, t6), "Named{{x:Int}} != Named{{y:Int}}");
    assert!(
        !m.types_equal(t7, t8),
        "Positional(Int) != Named{{x:Int}} (mixed field kinds)"
    );
}
