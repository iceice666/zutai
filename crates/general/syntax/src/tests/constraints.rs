use super::*;

// ---------------------------------------------------------------------------
// Constraint / witness parser tests (v1 syntax)
// ---------------------------------------------------------------------------

fn as_constraint(
    d: &Decl,
) -> (
    &str,
    &Vec<TypeParam>,
    &TypeExpr,
    &Vec<ConstraintMethod>,
    bool,
) {
    match d {
        Decl::Constraint {
            name,
            params,
            target,
            methods,
            derivable,
            ..
        } => (name, params, target, methods, *derivable),
        other => panic!("expected Constraint, got {other:?}"),
    }
}

fn as_witness(d: &Decl) -> (&str, &TypeExpr, &Vec<TypeParam>, &WitnessBody) {
    match d {
        Decl::Witness {
            constraint,
            target,
            params,
            body,
            ..
        } => (constraint, target, params, body),
        other => panic!("expected Witness, got {other:?}"),
    }
}

/// P1: basic constraint definition with one method
#[test]
fn p1_constraint_def_basic() {
    let f = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool; }\n1");
    let d = decl_by(&f, "Eq");
    let (name, params, _target, methods, derivable) = as_constraint(d);
    assert_eq!(name, "Eq");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name.as_str(), "eq");
    assert!(!derivable);
}

/// P2: constraint with single bound `<A: Eq>`
#[test]
fn p2_single_bound() {
    let f = parse_str("Ord :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, params, _, _, _) = as_constraint(d);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
    assert_eq!(params[0].bounds.len(), 1);
    assert_eq!(params[0].bounds[0].name, "Eq");
    // No spurious TopLevelSingleColon diagnostic
    assert!(parse_kinds("Ord :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1").is_empty());
}

/// P3: multi-bound `<A: Eq + Show>`
#[test]
fn p3_multi_bound() {
    let f = parse_str("Hash :: <A: Eq + Show> @A { hash :: A -> Int; }\n1");
    let d = decl_by(&f, "Hash");
    let (_, params, _, _, _) = as_constraint(d);
    assert_eq!(params[0].bounds.len(), 2);
    assert_eq!(params[0].bounds[0].name, "Eq");
    assert_eq!(params[0].bounds[1].name, "Show");
}

/// P4: HKT kind annotation `<F :: Type -> Type>`
#[test]
fn p4_hkt_kind() {
    let f = parse_str("Functor :: <F :: Type -> Type> @F { map :: Int -> F Int; }\n1");
    let d = decl_by(&f, "Functor");
    let (_, params, _, _, _) = as_constraint(d);
    assert_eq!(params[0].name, "F");
    assert_eq!(params[0].bounds.len(), 0);
    assert!(params[0].kind.is_some());
}

/// P5: method with method-level type params `<A, B>`
#[test]
fn p5_method_level_params() {
    let f = parse_str("Conv :: <F> @F { convert :: <A, B> A -> F B; }\n1");
    let d = decl_by(&f, "Conv");
    let (_, _, _, methods, _) = as_constraint(d);
    assert_eq!(methods[0].params.len(), 2);
    assert_eq!(methods[0].params[0].name, "A");
    assert_eq!(methods[0].params[1].name, "B");
}

/// P6: operator method name `(<)`
#[test]
fn p6_operator_method() {
    let f = parse_str("Ord :: <A> @A { (<) :: A -> A -> Bool; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, _, _, methods, _) = as_constraint(d);
    assert!(matches!(&methods[0].name, MethodName::Operator(s) if s == "<"));
}

/// P7: optional method `max?`
#[test]
fn p7_optional_method() {
    let f = parse_str("Ord :: <A> @A { lt :: A -> A -> Bool; max? :: A -> A -> A; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, _, _, methods, _) = as_constraint(d);
    assert!(!methods[0].optional);
    assert!(methods[1].optional);
}

#[test]
fn parse_constraint_method_default_clauses() {
    let f = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool = _ _ => true; }\n1");
    let (_, _, _, methods, _) = as_constraint(decl_by(&f, "Eq"));
    assert_eq!(methods[0].default.len(), 1);
}

/// P8: trailing `derive` marker on constraint def
#[test]
fn p8_constraint_derivable() {
    let f = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool; } derive\n1");
    let d = decl_by(&f, "Eq");
    let (_, _, _, _, derivable) = as_constraint(d);
    assert!(derivable);
}

/// P9: basic witness with field body
#[test]
fn p9_witness_basic() {
    let f = parse_str("Eq @Int :: { eq = \\ a b. a == b; }\n1");
    let d = decl_by(&f, "Eq");
    let (constraint, _target, params, body) = as_witness(d);
    assert_eq!(constraint, "Eq");
    assert!(params.is_empty());
    assert!(matches!(body, WitnessBody::Fields(fields) if fields.len() == 1));
    if let WitnessBody::Fields(fields) = body {
        assert_eq!(fields[0].name.as_str(), "eq");
    }
}

/// P10: conditional witness `<A: Eq>`
#[test]
fn p10_conditional_witness() {
    let f = parse_str("Eq @List :: <A: Eq> { eq = eqList; }\n1");
    let d = decl_by(&f, "Eq");
    let (_, _, params, _) = as_witness(d);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
    assert_eq!(params[0].bounds[0].name, "Eq");
    // No spurious TopLevelSingleColon diagnostic
    assert!(parse_kinds("Eq @List :: <A: Eq> { eq = eqList; }\n1").is_empty());
}

/// P11: derive body witness `:: derive`
#[test]
fn p11_derive_witness() {
    let f = parse_str("Eq @Server :: derive\n1");
    let d = decl_by(&f, "Eq");
    let (_, _, _, body) = as_witness(d);
    assert!(matches!(body, WitnessBody::Derive));
}

/// P12: operator witness field `(<) = ...`
#[test]
fn p12_operator_witness_field() {
    let f = parse_str("Ord @Int :: { (<) = intLt; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, _, _, body) = as_witness(d);
    if let WitnessBody::Fields(fields) = body {
        assert!(matches!(&fields[0].name, MethodName::Operator(s) if s == "<"));
    } else {
        panic!("expected Fields body");
    }
}

/// P13: partial-application target `@(List A)` — paren-grouped
#[test]
fn p13_partial_app_target() {
    let f = parse_str("Eq @(List A) :: <A: Eq> { eq = eqList; }\n1");
    let d = decl_by(&f, "Eq");
    let (_, target, _, _) = as_witness(d);
    // Target should be Apply(List, A)
    assert!(matches!(target, TypeExpr::Apply { .. }));
}

/// P14: plain function with bound `contains :: <A: Eq>`  — zero pre-pass diagnostics
#[test]
fn p14_plain_fn_bound_no_diagnostic() {
    let kinds = parse_kinds("contains :: <A: Eq> List -> A -> Bool\n  = xs x => false;\n1");
    assert!(kinds.is_empty(), "expected no diagnostics, got {kinds:?}");
}

/// P15: `derive := 1` is still a normal inferred binding (D4 guard)
#[test]
fn p15_derive_as_normal_binding() {
    let f = parse_str("derive ::= 1;\n1");
    let d = decl_by(&f, "derive");
    assert!(matches!(d, Decl::Inferred { .. }));
}

/// P16: every new form produces zero pre-pass diagnostics
#[test]
fn p16_no_prepass_diagnostics() {
    let forms = [
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\n1",
        "Ord :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1",
        "Hash :: <A: Eq + Show> @A { hash :: A -> Int; }\n1",
        "Functor :: <F :: Type -> Type> @F { map :: Int -> F Int; }\n1",
        "Eq @Int :: { eq = intEq; }\n1",
        "Eq @List :: <A: Eq> { eq = eqList; }\n1",
        "Eq @Server :: derive\n1",
        "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\n1",
    ];
    for src in &forms {
        let kinds = parse_kinds(src);
        assert!(
            kinds.is_empty(),
            "unexpected diagnostics for {src:?}: {kinds:?}"
        );
    }
}

#[test]
fn line_index_converts_byte_and_utf16_positions() {
    let index = LineIndex::new("a\né😀z");
    assert_eq!(index.line_col(0).line, 0);
    assert_eq!(index.line_col(2).line, 1);
    assert_eq!(index.line_col(2).col, 0);
    let offset = "a\né😀".len();
    let utf16 = index.utf16_line_col(offset);
    assert_eq!(utf16.line, 1);
    assert_eq!(utf16.col, 3);
}
