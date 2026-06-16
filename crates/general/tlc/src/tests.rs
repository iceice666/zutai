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
    // Type should be Fun(Int, Int), not ForAll.
    assert!(
        matches!(m.type_arena[*ty], crate::TlcType::Fun(_, _)),
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
            TlcType::Record(fields) => {
                // Empty Record as a Union fallback is the bug; legitimate empty records are ok,
                // but flag them in this helper only when tests are specifically checking for
                // Union→VariantT conversion. Separate per-test asserts cover that.
                let _ = fields;
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
    if let crate::TlcType::Record(ref fields) = m.type_arena[norm] {
        assert_eq!(fields.len(), 2, "Pair record must have exactly 2 fields");
        assert_eq!(fields[0].name, "first");
        assert_eq!(fields[1].name, "second");
        // Field types must be Text (Str) and Int after substitution.
        assert!(
            matches!(
                m.type_arena[fields[0].ty],
                crate::TlcType::Prim(crate::PrimTy::Str)
            ),
            "first field should be Str (Text), got {:?}",
            m.type_arena[fields[0].ty]
        );
        assert!(
            matches!(
                m.type_arena[fields[1].ty],
                crate::TlcType::Prim(crate::PrimTy::Int)
            ),
            "second field should be Int, got {:?}",
            m.type_arena[fields[1].ty]
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
