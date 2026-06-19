// ── Phase 5 tests: dictionary-passing elaboration ────────────────────────────

use super::tlc_of;
use crate::*;

/// A constraint witness decl lowers to a `TlcDecl::Value` whose body is a `Record`.
#[test]
fn witness_decl_lowers_to_record_value() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
1
"#,
    );
    let has_record = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Record(_)));
    assert!(has_record, "expected TlcExpr::Record for witness dict body");
}

/// A bounded function gets TyLam + dict Lam wrapping; its type starts with ForAll.
#[test]
fn bounded_function_gets_forall_and_dict_lam() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
same 1 1
"#,
    );
    let has_forall = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::ForAll(_, _, _)));
    assert!(
        has_forall,
        "expected ForAll type for bounded function `same`"
    );
    let has_tylam = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyLam(_, _, _)));
    assert!(has_tylam, "expected TyLam body for bounded function `same`");
}

/// Inside a bounded function body, a constraint-method call becomes GetField.
#[test]
fn constraint_method_call_lowers_to_get_field() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
same 1 1
"#,
    );
    let has_get_field = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::GetField(_, _)));
    assert!(
        has_get_field,
        "expected TlcExpr::GetField for constraint method call inside bounded function"
    );
}

/// A call to a bounded function injects TyApp for the type argument.
/// The call must be inside a declaration (not the final expression) so that
/// TLC lowers it.
#[test]
fn call_to_bounded_function_injects_tyapp() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
result :: Bool = same 1 1
result
"#,
    );
    let has_tyapp = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_tyapp,
        "expected TlcExpr::TyApp at call site of bounded function `same`"
    );
}

/// The bounded function body wraps with at least one extra Lam for the dict param.
#[test]
fn bounded_function_body_contains_dict_lam() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
same 1 1
"#,
    );
    // dict Lam + 2 value param Lams (x and y) = at least 3
    let lam_count = m
        .expr_arena
        .iter()
        .filter(|(_, e)| matches!(e, TlcExpr::Lam(_, _, _)))
        .count();
    assert!(
        lam_count >= 3,
        "expected at least 3 Lam nodes (dict + value params), got {}",
        lam_count
    );
}

/// A constraint with two methods produces a witness Record with two fields.
#[test]
fn witness_with_two_methods_has_two_record_fields() {
    let m = tlc_of(
        r#"
Ord :: <A> @A { lt :: A -> A -> Bool; gt :: A -> A -> Bool; }
Ord @Int :: { lt = \a b. a < b; gt = \a b. a > b; }
1
"#,
    );
    let two_field_record = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Record(fields) = e {
            fields.len() == 2
        } else {
            false
        }
    });
    assert!(
        two_field_record,
        "expected a Record with exactly 2 fields for Ord @Int witness"
    );
}

#[test]
fn derive_witness_synthesizes_method_field() {
    let m = tlc_of(
        r#"
Point :: type { x : Int; y : Int; }
p1 :: Point = { x = 1; y = 2; }
p2 :: Point = { x = 1; y = 2; }
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Point :: derive
eq p1 p2
"#,
    );
    let has_eq_field = m.expr_arena.iter().any(|(_, e)| {
        matches!(e, TlcExpr::Record(fields) if fields.iter().any(|(name, _)| name == "eq"))
    });
    assert!(
        has_eq_field,
        "derive witness should synthesize an `eq` field"
    );
}

/// Every expression in the module still has a type entry after Phase 5 elaboration.
#[test]
fn every_expr_has_type_after_phase5() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
same 1 1
"#,
    );
    for (id, _) in m.expr_arena.iter() {
        assert!(
            m.expr_types.contains_key(&id),
            "expr {:?} missing from expr_types after Phase 5 elaboration",
            id
        );
    }
}
