// ── Phase 0 tests: close the live data-loss holes ────────────────────────────

use super::{assert_no_data_loss, tlc_of};
use crate::*;

#[test]
fn union_type_lowers_to_variant_t_not_empty_record() {
    // Arm names are bare identifiers in the type declaration; values use # prefix.
    let m = tlc_of(
        r#"
Color :: type {
  #red;
  #green;
  #blue;
};
x :: Color = #red;
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
Dir :: type { #north; #south; #east; #west; };
x :: Dir = #north;
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
YesNo :: type { #yes; #no; };
x :: YesNo = #yes;
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
Status :: type { #dev; #test; #prod; };
x :: Status = #dev;
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
Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
};
x :: Shape = #circle { radius = 5; };
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
Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
};
area :: Shape -> Int
  = #circle { radius = r; } => r;
  = #square { side = s; } => s;
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
Color :: type { #red; #green; #blue; };
is_red :: Color -> Bool
  = #red => true;
  = _ => false;
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
