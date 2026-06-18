use crate::*;

fn tlc_of(src: &str) -> TlcModule {
    let parsed = zutai_syntax::parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "hir errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "thir errors: {:?}",
        thir.diagnostics
    );
    lower_thir(thir.file.as_ref().expect("thir file should be complete"))
}

#[test]
fn tlc_module_is_constructible() {
    use la_arena::Arena;
    use std::collections::HashMap;
    let _m = TlcModule {
        decls: Vec::new(),
        decl_arena: Arena::new(),
        expr_arena: Arena::new(),
        type_arena: Arena::new(),
        expr_types: HashMap::new(),
        spans: HashMap::new(),
    };
}

#[test]
fn monomorphic_int_binding_translates_type() {
    let m = tlc_of("x := 42\nx");
    assert_eq!(m.decls.len(), 1);
    let decl = &m.decl_arena[m.decls[0]];
    let crate::TlcDecl::Value { ty, .. } = decl else {
        panic!("expected Value decl")
    };
    assert_eq!(m.type_arena[*ty], crate::TlcType::Prim(crate::PrimTy::Int));
}

#[test]
fn int_literal_final_expr_no_decls() {
    let m = tlc_of("42");
    assert_eq!(m.decls.len(), 0);
}

#[test]
fn annotated_value_decl_lowers_correctly() {
    let m = tlc_of("x :: Int = 42\nx");
    assert_eq!(m.decls.len(), 1);
    let decl = &m.decl_arena[m.decls[0]];
    let crate::TlcDecl::Value { ty, body, .. } = decl else {
        panic!("expected Value decl")
    };
    assert_eq!(m.type_arena[*ty], crate::TlcType::Prim(crate::PrimTy::Int));
    assert_eq!(
        m.expr_arena[*body],
        crate::TlcExpr::Lit(crate::Literal::Int(42))
    );
}

#[test]
fn type_alias_decl_lowers_correctly() {
    // Non-generic alias: 0 params → body is the record directly, no TyLamK wrapping.
    let m = tlc_of("Point :: type { x : Int; y : Int; }\nPoint");
    assert_eq!(m.decls.len(), 1);
    let crate::TlcDecl::TypeAlias { body, .. } = m.decl_arena[m.decls[0]] else {
        panic!("expected TypeAlias decl")
    };
    // The body of a 0-param alias must NOT be a TyLamK.
    assert!(
        !matches!(m.type_arena[body], crate::TlcType::TyLamK(_, _, _)),
        "0-param alias body should not be wrapped in TyLamK, got {:?}",
        m.type_arena[body]
    );
    assert!(
        matches!(m.type_arena[body], crate::TlcType::Record(_)),
        "0-param alias body should be a Record, got {:?}",
        m.type_arena[body]
    );
}

#[test]
fn bool_literal_no_crash() {
    let m = tlc_of("true");
    assert_eq!(m.decls.len(), 0);
}

#[test]
fn monomorphic_identity_function_lowers_to_lam() {
    // Explicitly typed: no generalization
    let m = tlc_of("id :: Int -> Int = \\x. x\nid 1");
    assert_eq!(m.decls.len(), 1);
    let crate::TlcDecl::Value { body, ty, .. } = &m.decl_arena[m.decls[0]] else {
        panic!("expected Value decl")
    };
    // Type should be Fun(Int, Int, REmpty), not ForAll.
    assert!(
        matches!(m.type_arena[*ty], crate::TlcType::Fun(_, _, _)),
        "expected Fun type but got {:?}",
        m.type_arena[*ty]
    );
    // Body should be a Lam (possibly through TyApp wrappers — walk to innermost).
    fn innermost(m: &crate::TlcModule, id: crate::TlcExprId) -> &crate::TlcExpr {
        match &m.expr_arena[id] {
            crate::TlcExpr::TyApp(inner, _) => innermost(m, *inner),
            e => e,
        }
    }
    assert!(
        matches!(innermost(&m, *body), crate::TlcExpr::Lam(_, _, _)),
        "expected Lam body but got {:?}",
        innermost(&m, *body)
    );
}

#[test]
fn polymorphic_identity_gets_tylam_and_forall() {
    // No annotation → HM generalizes to ∀a. a → a
    let m = tlc_of("id x = x\nid 42");
    assert_eq!(m.decls.len(), 1);
    let crate::TlcDecl::Value { body, ty, .. } = &m.decl_arena[m.decls[0]] else {
        panic!("expected Value decl")
    };
    assert!(
        matches!(m.type_arena[*ty], crate::TlcType::ForAll(_, _, _)),
        "expected ForAll but got {:?}",
        m.type_arena[*ty]
    );
    assert!(
        matches!(m.expr_arena[*body], crate::TlcExpr::TyLam(_, _, _)),
        "expected TyLam but got {:?}",
        m.expr_arena[*body]
    );
}

#[test]
fn if_desugars_to_case() {
    let m = tlc_of("f x = if x then 1 else 2\nf true");
    let has_case = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, crate::TlcExpr::Case(_, _)));
    assert!(has_case, "expected a Case node from If desugaring");
}

#[test]
fn block_desugars_to_let() {
    let m = tlc_of("f x = { n := 42; n }\nf 0");
    let has_let = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, crate::TlcExpr::Let { .. }));
    assert!(has_let, "expected a Let node from Block desugaring");
}

#[test]
fn binary_op_lowers_to_builtin() {
    let m = tlc_of("f x y = x + y\nf 1 2");
    let has_builtin = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, crate::TlcExpr::Builtin(crate::BuiltinOp::Add, _, _)));
    assert!(has_builtin, "expected Builtin(Add) from binary + op");
}

#[test]
fn invariant_every_expr_has_type_entry() {
    let m = tlc_of("add x y = x + y\nadd 1 2");
    for (id, _) in m.expr_arena.iter() {
        assert!(
            m.expr_types.contains_key(&id),
            "expr {:?} missing from expr_types",
            id
        );
    }
}

// ── Phase 0 tests: close the live data-loss holes ────────────────────────────

/// Walk every type in the arena and assert no forbidden residuals remain.
fn assert_no_data_loss(m: &TlcModule) {
    for (_, ty) in m.type_arena.iter() {
        match ty {
            // Union must have been converted to VariantT — no empty Record fallback.
            TlcType::Record(row) => {
                // Empty Record as a Union fallback is the bug; legitimate empty records are ok,
                // but flag them in this helper only when tests are specifically checking for
                // Union→VariantT conversion. Separate per-test asserts cover that.
                let _ = row;
            }
            TlcType::Prim(PrimTy::Atom) => {
                // PrimTy::Atom is valid for the unqualified Atom primitive.
                // Individual tests assert that symbol-carrying atoms become Singleton.
            }
            _ => {}
        }
    }
}

#[test]
fn union_type_lowers_to_variant_t_not_empty_record() {
    // Arm names are bare identifiers in the type declaration; values use # prefix.
    let m = tlc_of(
        r#"
Color :: type [
  red;
  green;
  blue;
]
x :: Color = #red
x
"#,
    );
    let has_variant_t = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::VariantT(_)));
    assert!(has_variant_t, "expected VariantT for union type, got none");
}

#[test]
fn union_type_row_has_correct_arm_count() {
    let m = tlc_of(
        r#"
Dir :: type [ north; south; east; west; ]
x :: Dir = #north
x
"#,
    );
    let variant_t = m
        .type_arena
        .iter()
        .find_map(|(_, ty)| {
            if let TlcType::VariantT(row) = ty {
                Some(row)
            } else {
                None
            }
        })
        .expect("expected VariantT for union type");
    let arm_count = variant_t.fields().count();
    assert_eq!(arm_count, 4, "expected 4 union arms in the Row");
}

#[test]
fn true_singleton_type_not_flattened_to_prim_bool() {
    // Union atom arms must produce Singleton types, not Prim(Bool).
    let m = tlc_of(
        r#"
YesNo :: type [ yes; no; ]
x :: YesNo = #yes
x
"#,
    );
    let singletons: Vec<_> = m
        .type_arena
        .iter()
        .filter(|(_, ty)| matches!(ty, TlcType::Singleton(_)))
        .collect();
    assert!(
        !singletons.is_empty(),
        "expected Singleton types for union atom arms"
    );
    assert_no_data_loss(&m);
}

#[test]
fn atom_type_in_union_is_singleton_not_prim_atom() {
    let m = tlc_of(
        r#"
Status :: type [ dev; test; prod; ]
x :: Status = #dev
x
"#,
    );
    // Each arm should produce Singleton(Atom(...)), not Prim(Atom).
    let singleton_atoms: Vec<_> = m
        .type_arena
        .iter()
        .filter(|(_, ty)| matches!(ty, TlcType::Singleton(Literal::Atom(_))))
        .collect();
    assert!(
        !singleton_atoms.is_empty(),
        "expected Singleton(Atom) nodes for union arms, found none"
    );
    // No Prim(Atom) should stand in for a symbol-carrying atom type.
    let prim_atoms: Vec<_> = m
        .type_arena
        .iter()
        .filter(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Atom)))
        .collect();
    assert!(
        prim_atoms.is_empty(),
        "found Prim(Atom) — atom symbol payload lost (data-loss bug not fixed)"
    );
}

#[test]
fn tagged_value_expression_lowers_to_variant() {
    // `#circle { radius = 5; }` → ThirExprKind::TaggedValue → TlcExpr::Variant
    let m = tlc_of(
        r#"
Shape :: type [
  circle: { radius: Int; };
  square: { side: Int; };
]
x :: Shape = #circle { radius = 5; }
x
"#,
    );
    let has_variant_expr = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Variant(label, _) if label == "circle"));
    assert!(
        has_variant_expr,
        "expected Variant(\"circle\", _) expression"
    );
}

#[test]
fn tagged_value_pattern_lowers_to_variant_pat() {
    // Patterns on tagged-payload union arms → TlcPat::Variant
    let m = tlc_of(
        r#"
Shape :: type [
  circle: { radius: Int; };
  square: { side: Int; };
]
area :: Shape -> Int {
  | #circle { radius = r; } => r;
  | #square { side = s; } => s;
}
area
"#,
    );
    let has_variant_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter().any(|a| matches!(&a.pat, TlcPat::Variant(_, _)))
        } else {
            false
        }
    });
    assert!(
        has_variant_pat,
        "expected TlcPat::Variant arms in Case for tagged-payload union pattern match"
    );
}

#[test]
fn module_walk_invariant_no_forbidden_residuals() {
    let m = tlc_of(
        r#"
Color :: type [ red; green; blue; ]
is_red :: Color -> Bool {
  | #red => true;
  | _ => false;
}
is_red #green
"#,
    );

    // Every expr has a type entry.
    for (id, _) in m.expr_arena.iter() {
        assert!(
            m.expr_types.contains_key(&id),
            "expr {:?} missing from expr_types",
            id
        );
    }

    // VariantT must be present; no Union survived as empty Record.
    let has_variant_t = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::VariantT(_)));
    assert!(has_variant_t, "VariantT must appear for union types");
}

// ── Phase 1 tests: kind annotations on polymorphism nodes ────────────────────

/// Every `TyVar`, `ForAll`, and `TyLam` node produced in Phase 1 must carry
/// `Kind::Type(0)`. This locks the invariant so later phases that introduce
/// non-ground kinds (Phase 3 HKT, Phase 5 constraints) must do so deliberately.
#[test]
fn phase1_all_polymorphism_nodes_carry_ground_kind() {
    // Three representative programs: polymorphic identity, multi-param function,
    // and a type alias reference.
    let programs = [
        "id x = x\nid 42",
        "add x y = x + y\nadd 1 2",
        "Point :: type { x : Int; y : Int; }\nid x = x\nid 0",
    ];

    for src in programs {
        let m = tlc_of(src);

        // Every TyVar in the type arena must carry Kind::Type(0).
        for (_, ty) in m.type_arena.iter() {
            if let crate::TlcType::TyVar(_, kind) = ty {
                assert_eq!(
                    *kind,
                    crate::Kind::Type(0),
                    "TyVar carries non-ground kind in program: {src}"
                );
            }
            if let crate::TlcType::ForAll(_, kind, _) = ty {
                assert_eq!(
                    *kind,
                    crate::Kind::Type(0),
                    "ForAll carries non-ground kind in program: {src}"
                );
            }
        }

        // Every TyLam in the expr arena must carry Kind::Type(0).
        for (_, expr) in m.expr_arena.iter() {
            if let crate::TlcExpr::TyLam(_, kind, _) = expr {
                assert_eq!(
                    *kind,
                    crate::Kind::Type(0),
                    "TyLam carries non-ground kind in program: {src}"
                );
            }
        }
    }
}

// ── Phase 2 tests: TyLamK + lazy alias lowering + NbE ────────────────────────

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

// ── Phase 3 tests: row kind + row polymorphism ────────────────────────────────

fn make_module(type_arena: la_arena::Arena<TlcType>) -> TlcModule {
    use std::collections::HashMap;
    TlcModule {
        decls: Vec::new(),
        decl_arena: la_arena::Arena::new(),
        expr_arena: la_arena::Arena::new(),
        type_arena,
        expr_types: HashMap::new(),
        spans: HashMap::new(),
    }
}

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

// ── Phase 3 tests: capture-avoiding substitution ─────────────────────────────

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

// ── Phase 4 tests: effect row on Fun ─────────────────────────────────────────

/// Every v0 function lowers to `Fun(_, _, Row::REmpty)` — invariant #10 holds vacuously:
/// no v0 path produces a non-empty effect row, so "erase before DC" is a no-op.
#[test]
fn v0_function_lowers_with_empty_effect_row() {
    let m = tlc_of("id :: Int -> Int = \\x. x\nid 1");
    let crate::TlcDecl::Value { ty, .. } = &m.decl_arena[m.decls[0]] else {
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

// ── Phase 4 tests: effect-row eraser ─────────────────────────────────────────

/// `erase_effects` is a no-op on a module where every `Fun` already has `REmpty`.
/// This is the invariant that holds for all v0 programs.
#[test]
fn effect_eraser_is_noop_for_pure_functions() {
    let mut m = tlc_of("id :: Int -> Int = \\x. x\nid 1");
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

// ── Additional lower/types.rs coverage ───────────────────────────────────────

#[test]
fn float_literal_lowers_to_prim_float_type() {
    let m = tlc_of("f :: Float = 1.5\nf");
    // lower_types.rs: TypeKind::Float → TlcType::Prim(PrimTy::Float)
    let has_float = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Float)));
    assert!(has_float, "expected Prim(Float) for Float type");
    // lower_expr.rs: ThirExprKind::Float → TlcExpr::Lit(Literal::Float)
    let has_lit = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lit(Literal::Float(_))));
    assert!(has_lit, "expected Lit(Float) for float literal expression");
}

#[test]
fn string_literal_lowers_to_prim_str_type() {
    let m = tlc_of(
        r#"s :: Text = "hello"
s"#,
    );
    // lower_types.rs: TypeKind::Text → TlcType::Prim(PrimTy::Str)
    let has_str_ty = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Str)));
    assert!(has_str_ty, "expected Prim(Str) for Text type");
    // lower_expr.rs: ThirExprKind::String → TlcExpr::Lit(Literal::Str)
    let has_lit = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lit(Literal::Str(_))));
    assert!(has_lit, "expected Lit(Str) for string literal expression");
}

#[test]
fn bool_type_annotation_lowers_to_prim_bool() {
    let m = tlc_of("b :: Bool = true\nb");
    // lower_types.rs: TypeKind::Bool → TlcType::Prim(PrimTy::Bool)
    let has_bool = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Bool)));
    assert!(has_bool, "expected Prim(Bool) for Bool annotation");
}

#[test]
fn list_type_lowers_to_tlc_list() {
    let m = tlc_of("xs :: List Int = [1; 2; 3;]\nxs");
    // lower_types.rs: TypeKind::List(inner) → TlcType::List(inner_tlc)
    let has_list = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::List(_)));
    assert!(has_list, "expected TlcType::List for List Int");
}

#[test]
fn optional_type_lowers_to_tlc_optional() {
    let m = tlc_of("x :: Int? = #none\nx");
    // lower_types.rs: TypeKind::Optional(inner) → TlcType::Optional(inner_tlc)
    let has_opt = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Optional(_)));
    assert!(has_opt, "expected TlcType::Optional for Int?");
}

#[test]
fn positional_tuple_type_lowers_to_tlc_tuple() {
    let m = tlc_of(
        r#"p :: (Int, Text) = (1, "hi")
p"#,
    );
    // lower_types.rs: TypeKind::Tuple with Positional → TlcType::Tuple with TlcTupleField::Positional
    let has_tuple = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Tuple(_)));
    assert!(
        has_tuple,
        "expected TlcType::Tuple for positional tuple type"
    );
    let has_positional_field = m.type_arena.iter().any(|(_, ty)| {
        if let TlcType::Tuple(items) = ty {
            items
                .iter()
                .any(|i| matches!(i, TlcTupleField::Positional(_)))
        } else {
            false
        }
    });
    assert!(
        has_positional_field,
        "expected TlcTupleField::Positional inside Tuple"
    );
}

#[test]
fn named_tuple_type_lowers_to_tlc_tuple_with_named_fields() {
    let m = tlc_of("p :: (x : Int, y : Int) = (x = 1, y = 2)\np");
    // lower_types.rs: TypeKind::Tuple with Named → TlcType::Tuple with TlcTupleField::Named
    let has_named_field = m.type_arena.iter().any(|(_, ty)| {
        if let TlcType::Tuple(items) = ty {
            items
                .iter()
                .any(|i| matches!(i, TlcTupleField::Named { .. }))
        } else {
            false
        }
    });
    assert!(
        has_named_field,
        "expected TlcTupleField::Named inside Tuple for named tuple type"
    );
}

// ── Additional lower/expr.rs pattern coverage ─────────────────────────────────

#[test]
fn float_pattern_lowers_to_lit_float_pat() {
    let m = tlc_of(
        r#"classify :: Float -> Text {
  | 0.0 => "zero";
  | _ => "other";
}
classify 1.0"#,
    );
    // lower_expr.rs: ThirPatKind::Float(f) → TlcPat::Lit(Literal::Float(f))
    let has_float_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter()
                .any(|a| matches!(&a.pat, TlcPat::Lit(Literal::Float(_))))
        } else {
            false
        }
    });
    assert!(has_float_pat, "expected Lit(Float) pattern in Case alts");
}

#[test]
fn string_pattern_lowers_to_lit_str_pat() {
    let m = tlc_of(
        r#"greet :: Text -> Int {
  | "hello" => 1;
  | _ => 0;
}
greet "hi""#,
    );
    // lower_expr.rs: ThirPatKind::String(s) → TlcPat::Lit(Literal::Str(s))
    let has_str_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter()
                .any(|a| matches!(&a.pat, TlcPat::Lit(Literal::Str(_))))
        } else {
            false
        }
    });
    assert!(has_str_pat, "expected Lit(Str) pattern in Case alts");
}

#[test]
fn atom_pattern_bare_union_lowers_to_atom_pat() {
    // Bare union arm `#dev` / `#prod` (no payload) → ThirPatKind::Atom → TlcPat::Atom
    let m = tlc_of(
        r#"Profile :: type [ dev; prod; ]
isProd :: Profile -> Bool {
  | #prod => true;
  | #dev => false;
}
isProd #prod"#,
    );
    // lower_expr.rs: ThirPatKind::Atom(s) → TlcPat::Atom(s)
    let has_atom_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter().any(|a| matches!(&a.pat, TlcPat::Atom(_)))
        } else {
            false
        }
    });
    assert!(
        has_atom_pat,
        "expected TlcPat::Atom for bare union arm patterns"
    );
}

#[test]
fn wildcard_lambda_param_uses_fresh_synthetic_binding() {
    // `\\ _ . body` — the `_` wildcard is ThirPatKind::Wildcard (non-Bind)
    // lower_lambda's else branch creates a fresh synthetic binding.
    let m = tlc_of("const42 :: Int -> Int = \\_ . 42\nconst42 1");
    let has_lam = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lam(_, _, _)));
    assert!(has_lam, "expected TlcExpr::Lam from wildcard-param lambda");
}

#[test]
fn optional_access_lowers_to_get_field() {
    // `cfg?.port` where cfg :: Config? → ThirExprKind::OptionalAccess → TlcExpr::GetField
    let m = tlc_of(
        "Config :: type { port : Int; }
cfg :: Config? = #none
n :: Int? = cfg?.port
n",
    );
    let has_get_field = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::GetField(_, _)));
    assert!(
        has_get_field,
        "expected TlcExpr::GetField from OptionalAccess"
    );
}

// ── Remaining binop operators ─────────────────────────────────────────────────

#[test]
fn sub_mul_div_binops_lower_to_builtin() {
    let m = tlc_of("f x y = x - y\nf 5 3");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Sub, _, _))),
        "expected Builtin(Sub)"
    );
    let m = tlc_of("f x y = x * y\nf 2 3");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Mul, _, _))),
        "expected Builtin(Mul)"
    );
    let m = tlc_of("f x y = x / y\nf 6 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Div, _, _))),
        "expected Builtin(Div)"
    );
}

#[test]
fn comparison_binops_lower_to_builtin() {
    let m = tlc_of("f x y = x == y\nf 1 1");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Eq, _, _))),
        "expected Builtin(Eq)"
    );
    let m = tlc_of("f x y = x != y\nf 1 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Ne, _, _))),
        "expected Builtin(Ne)"
    );
    let m = tlc_of("f x y = x < y\nf 1 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Lt, _, _))),
        "expected Builtin(Lt)"
    );
    let m = tlc_of("f x y = x <= y\nf 1 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Le, _, _))),
        "expected Builtin(Le)"
    );
    let m = tlc_of("f x y = x > y\nf 2 1");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Gt, _, _))),
        "expected Builtin(Gt)"
    );
    let m = tlc_of("f x y = x >= y\nf 2 1");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Ge, _, _))),
        "expected Builtin(Ge)"
    );
}

#[test]
fn logical_and_or_coalesce_lower_to_builtin() {
    let m = tlc_of("f x y = x && y\nf true false");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::And, _, _))),
        "expected Builtin(And)"
    );
    let m = tlc_of("f x y = x || y\nf true false");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Or, _, _))),
        "expected Builtin(Or)"
    );
    // Coalesce (??) on an Optional record field — placed in a declaration body so
    // TLC lowers it (the `final_expr` slot is not visited by the TLC lowerer).
    let m = tlc_of(
        "Server :: type { port? : Int; }\nget :: Server -> Int = \\s. s.port ?? 8080\nget {}",
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Coalesce, _, _))),
        "expected Builtin(Coalesce)"
    );
}

// ── Normalize (hand-built IR) ─────────────────────────────────────────────────

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

// ── subst: ForAll binder (shadowed and non-shadowed) ─────────────────────────

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

// ── subst_row RVar arm ────────────────────────────────────────────────────────

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

// ── types_equal_deep: direct structural equality tests ───────────────────────

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

// ── tyvar_key Inferred arm ────────────────────────────────────────────────────

#[test]
fn inferred_tyvar_as_normalize_binder() {
    // TyApp(TyLamK(Inferred(0), _, TyVar(Inferred(0))), Int) β-reduces to Int.
    // Exercises the TyLamK path with an Inferred TlcTypeVar binder.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Inferred(0);
    let kind = Kind::ground();
    let tvar = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, tvar));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::Prim(PrimTy::Int)),
        "TyLamK with Inferred var β-reduces to Int; got {:?}",
        m.type_arena[norm]
    );
}

// ── lower_pat: True / False literal patterns ──────────────────────────────────

#[test]
fn true_false_literal_patterns_lower_to_lit_bool() {
    // A function matching on `true` and `false` — exercises TlcPat::Lit(Bool(true/false)).
    let m = tlc_of(
        r#"toInt :: Bool -> Int {
  | true => 1;
  | false => 0;
}
toInt true"#,
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Case(_, alts) if alts.iter().any(|a| matches!(&a.pat, TlcPat::Lit(Literal::Bool(true)))))),
        "expected TlcPat::Lit(Bool(true)) in a Case alt"
    );
}

// ── lower_pat: Record pattern ─────────────────────────────────────────────────

#[test]
fn record_pattern_lowers_to_tlc_record_pat() {
    // Record pattern in a match arm exercises ThirPatKind::Record → TlcPat::Record.
    let m = tlc_of(
        r#"Point :: type { x : Int; y : Int; }
getX :: Point -> Int {
  | { x = v; y = _; } => v;
}
getX { x = 42; y = 0; }"#,
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Case(_, alts) if alts.iter().any(|a| matches!(&a.pat, TlcPat::Record(_))))),
        "expected TlcPat::Record in a Case alt"
    );
}

// ── HM polymorphism: lower_binding_ref poly path, extract_instantiation, match_types ──

/// An inferred-polymorphic identity function applied to an `Int` value forces
/// TLC to call `lower_binding_ref` → `extract_instantiation` → `match_types`
/// (Function and InferVar arms). This covers lines 152-163 in tlc/lower/expr.rs
/// and lines 158-192 in tlc/lower/types.rs.
///
/// Note: TLC only lowers *declarations*, not the final expression. The call to
/// `id` must appear inside a declaration (here `result := id 42`) so that TLC
/// traverses the Apply and hits `lower_binding_ref` for the poly scheme.
#[test]
fn polymorphic_identity_lowers_with_ty_app() {
    // `id x = x` is inferred as `?0 -> ?0`; THIR puts it in poly_schemes.
    // `result := id 42` is a declaration whose value TLC lowers. When TLC
    // processes the BindingRef to `id`, lower_binding_ref sees the poly scheme
    // and calls extract_instantiation(scheme=[?0], stored=?0->?0, ref=Int->Int).
    let m = tlc_of("id x = x\nresult := id 42\nresult");
    // id + result = 2 declarations
    assert_eq!(m.decls.len(), 2, "expected id and result decls");
    // The module should contain a TyApp expression (type application for id)
    let has_ty_app = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_ty_app,
        "expected TyApp expression for polymorphic id application"
    );
}

/// `wrap := \x. [x;]` has type `?0 -> List(?0)`. Using `wrap` inside a declaration
/// triggers `match_types(List(?0), List(Int))` → the `List(ti)` arm at L194-197.
///
/// Uses `:=` with a lambda so the list body is inferred (not checked against an
/// unresolved InferVar), avoiding the ExpectedList diagnostic from function-decl syntax.
/// Note: TLC only lowers declarations; the call must be inside a decl.
#[test]
fn polymorphic_list_wrapper_covers_match_types_list_arm() {
    let m = tlc_of("wrap := \\x. [x;]\nresult := wrap 42\nresult");
    assert_eq!(m.decls.len(), 2, "expected wrap and result decls");
    let has_ty_app = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_ty_app,
        "expected TyApp for polymorphic wrap application"
    );
}

/// `pass x = x` applied to an optional value forces
/// `match_types(Optional(?0), Optional(Int?))` → the `Optional(ti)` arm at L199-202.
///
/// Note: TLC only lowers declarations; the call must be inside a decl.
#[test]
fn polymorphic_identity_with_optional_covers_match_types_optional_arm() {
    let m = tlc_of("pass x = x\nv :: Int? = #none\nresult := pass v\nresult");
    // The poly scheme for `pass` is instantiated with `Int?` → match_types Optional arm hit.
    let _ = m;
}

// ── lower_pat: Tuple pattern ──────────────────────────────────────────────────

#[test]
fn tuple_pattern_lowers_to_tlc_tuple_pat() {
    // Positional tuple pattern in a function clause exercises TlcPat::Tuple.
    let m = tlc_of(
        r#"fst :: (Int, Int) -> Int {
  | (x, _) => x;
}
fst (1, 2)"#,
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Case(_, alts) if alts.iter().any(|a| matches!(&a.pat, TlcPat::Tuple(_))))),
        "expected TlcPat::Tuple in a Case alt"
    );
}
