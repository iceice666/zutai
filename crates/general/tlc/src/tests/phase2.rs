// ── Phase 2 tests: TyLamK + lazy alias lowering + NbE ────────────────────────

use super::tlc_of;

#[test]
fn generic_alias_decl_lowers_to_tylamk_chain() {
    // A 2-param alias should produce: TyLamK(A, _, TyLamK(B, _, Record(...)))
    let m = tlc_of(
        r#"
Pair :: <A, B> type { first : A; second : B; }
x :: Pair Int Int = { first = 1; second = 2; }
x
"#,
    );
    // Find the TypeAlias decl for Pair.
    let alias_body = m.decls.iter().find_map(|&id| {
        if let crate::TlcDecl::TypeAlias { body, .. } = m.decl_arena[id] {
            Some(body)
        } else {
            None
        }
    });
    let body = alias_body.expect("expected a TypeAlias decl for Pair");
    assert!(
        matches!(m.type_arena[body], crate::TlcType::TyLamK(_, _, _)),
        "2-param alias body must be TyLamK at outermost level, got {:?}",
        m.type_arena[body]
    );
    // The inner body must also be TyLamK (second parameter).
    if let crate::TlcType::TyLamK(_, _, inner) = m.type_arena[body] {
        assert!(
            matches!(m.type_arena[inner], crate::TlcType::TyLamK(_, _, _)),
            "inner body of 2-param alias must also be TyLamK, got {:?}",
            m.type_arena[inner]
        );
    }
}

#[test]
fn generic_alias_head_var_carries_arrow_kind() {
    // An applied alias `Pair Text Int` lowers to TyApp(TyApp(TyVar(alias, arrowKind), Text), Int).
    // The head TyVar must carry an Arrow kind, not ground.
    let m = tlc_of(
        r#"
Pair :: <A, B> type { first : A; second : B; }
x :: Pair Int Int = { first = 1; second = 2; }
x
"#,
    );
    // Find a TyVar with an Arrow kind anywhere in the type arena.
    let has_arrow_kinded_var = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, crate::TlcType::TyVar(_, crate::Kind::Arrow(_, _))));
    assert!(
        has_arrow_kinded_var,
        "expected a TyVar with Kind::Arrow for the alias head in applied alias"
    );
}

#[test]
fn nbe_reduces_alias_application_to_record() {
    // `Pair Text Int` should normalize to `{ first : Text; second : Int }`.
    let mut m = tlc_of(
        r#"
Pair :: <A, B> type { first : A; second : B; }
x :: Pair Text Int = { first = "hello"; second = 1; }
x
"#,
    );

    // Find the type of the value decl `x` — it is the alias application spine.
    let x_ty = m.decls.iter().find_map(|&id| {
        if let crate::TlcDecl::Value { ty, .. } = m.decl_arena[id] {
            Some(ty)
        } else {
            None
        }
    });
    let x_ty = x_ty.expect("expected a Value decl for x");

    // Normalize the application spine.
    let norm = m
        .normalize(x_ty)
        .expect("normalization must not exhaust fuel");

    // The normal form must be a Record with two fields.
    assert!(
        matches!(m.type_arena[norm], crate::TlcType::Record(_)),
        "Pair Text Int should normalize to a Record, got {:?}",
        m.type_arena[norm]
    );
    if let crate::TlcType::Record(ref row) = m.type_arena[norm] {
        // Walk the Row in declaration order (normal form preserves declaration order).
        let fields: Vec<_> = row.fields().collect();
        assert_eq!(fields.len(), 2, "Pair record must have exactly 2 fields");
        assert_eq!(fields[0].0, "first");
        assert_eq!(fields[1].0, "second");
        // Field types must be Text (Str) and Int after substitution.
        assert!(
            matches!(
                m.type_arena[fields[0].1],
                crate::TlcType::Prim(crate::PrimTy::Str)
            ),
            "first field should be Str (Text), got {:?}",
            m.type_arena[fields[0].1]
        );
        assert!(
            matches!(
                m.type_arena[fields[1].1],
                crate::TlcType::Prim(crate::PrimTy::Int)
            ),
            "second field should be Int, got {:?}",
            m.type_arena[fields[1].1]
        );
    }
}

#[test]
fn nbe_fuel_exhaustion_is_clean_error() {
    // Build a hand-crafted IR module with a self-referential alias application to exhaust fuel.
    // THIR rejects recursive type functions, so we cannot go through tlc_of here.
    use crate::ir::{Kind, TlcDecl, TlcModule, TlcType, TlcTypeVar};
    use la_arena::Arena;
    use std::collections::HashMap;
    use zutai_hir::BindingId;

    let mut type_arena: Arena<TlcType> = Arena::new();
    let mut decl_arena: Arena<TlcDecl> = Arena::new();

    // alias_binding: BindingId(99)
    let alias_binding = BindingId(99);
    let alias_var = TlcTypeVar::Named(99);

    // body of "alias" = TyVar(alias, ground) — self-referential
    let alias_head = type_arena.alloc(TlcType::TyVar(alias_var, Kind::ground()));
    // A dummy arg type (Int) to make it a TyApp
    let arg_ty = type_arena.alloc(TlcType::Prim(crate::PrimTy::Int));

    // The alias decl stores: alias_binding → alias_head (TyVar of itself)
    let _decl_id = decl_arena.alloc(TlcDecl::TypeAlias {
        binding: alias_binding,
        params: vec![alias_binding],
        body: alias_head,
    });

    // Root: TyApp(TyVar(alias, ground), Int) — applying the self-referential alias
    let root = type_arena.alloc(TlcType::TyApp(alias_head, arg_ty));

    let mut m = TlcModule {
        decls: Vec::new(),
        decl_arena,
        expr_arena: Arena::new(),
        type_arena,
        expr_types: HashMap::new(),
        spans: HashMap::new(),
    };

    // With small fuel (5 steps) this must return FuelExhausted, never panic.
    // We use small fuel (not DEFAULT_FUEL=1000) because a self-referential type
    // creates one stack frame per fuel unit; 1000 frames in debug mode can overflow
    // the ~512 KB pthread stack on macOS.
    let result = m.normalize_with_fuel(root, 5);
    assert!(
        matches!(
            result,
            Err(crate::NormalizeError::FuelExhausted { limit: 5 })
        ),
        "expected FuelExhausted, got {:?}",
        result
    );

    // Verify that normalize_with_fuel(_, 10) also returns Err (not an infinite loop).
    let result2 = m.normalize_with_fuel(root, 10);
    assert!(
        matches!(
            result2,
            Err(crate::NormalizeError::FuelExhausted { limit: 10 })
        ),
        "expected FuelExhausted with limit=10, got {:?}",
        result2
    );
}
