//! Differential oracle: the THIR tree-walker (`eval_file`) and the TLC
//! eager walker (`eval_tlc_file`) must agree on every well-typed, import-free
//! `.zt` program. A divergence is a bug in one of the two evaluators.

use zutai_eval::{Value, eval_file, eval_tlc_file};

/// Programs that both evaluators must agree on, with the expected value.
fn battery() -> Vec<(&'static str, &'static str)> {
    vec![
        ("arith", "1 + 2 * 3"),
        ("int_div", "7 / 2"),
        ("float", "1.0 + 2.0"),
        ("bool_and", "1 < 2 && 2 < 3"),
        ("bool_or", "1 < 2 || 5 < 0"),
        ("text_cmp", "\"a\" < \"b\""),
        ("if_expr", "if 3 > 2 then 10 else 20"),
        ("record", "{ a = 1; b = 2; }.b"),
        ("tuple", "(1, 2)"),
        ("list_eq", "[1; 2; 3;] == [1; 2; 3;]"),
        (
            "factorial",
            "f :: Int -> Int {\n  | 0 => 1;\n  | n => n * f (n - 1);\n}\nf 5",
        ),
        ("lambda", "(\\x. x * 2) 21"),
        (
            "curry",
            "add :: Int -> Int -> Int {\n  | a b => a + b;\n}\nadd 3 4",
        ),
        (
            "coalesce_absent",
            "S :: type { p? : Int; }\ns :: S = {}\ns.p ?? 8080",
        ),
        ("coalesce_explicit_none", "x :: Int? = #none\nx ?? 5"),
        (
            "coalesce_explicit_some",
            "x :: Int? = #some { value = 9; }\nx ?? 5",
        ),
        (
            "optional_field_match_none",
            "S :: type { p? : Int; }\ns :: S = {}\nmatch s.p { | #none => 1; | #some { value = n; } => n; }",
        ),
        (
            "optional_field_match_some",
            "S :: type { p? : Int; }\ns :: S = { p = 7; }\nmatch s.p { | #none => 1; | #some { value = n; } => n; }",
        ),
        (
            "optional_access_chain_some",
            "Inner :: type { val : Int; }\nOuter :: type { inner? : Inner; }\no :: Outer = { inner = { val = 5; }; }\no.inner?.val ?? 0",
        ),
        (
            "optional_access_chain_none",
            "Inner :: type { val : Int; }\nOuter :: type { inner? : Inner; }\no :: Outer = {}\no.inner?.val ?? 0",
        ),
        (
            "optional_explicit_some_access",
            "Inner :: type { val : Int; }\ncfg :: Inner? = #some { value = { val = 9; }; }\ncfg?.val ?? 0",
        ),
        (
            "match_union",
            "Shape :: type [ c: { r: Int; }; s: { v: Int; }; ]\nf :: Shape -> Int {\n  | #c { r = r; } => r;\n  | #s { v = v; } => v;\n}\nf (#c { r = 7; })",
        ),
        (
            "guard",
            "f :: Int -> Text {\n  | n if n > 0 => \"pos\";\n  | _ => \"nonpos\";\n}\nf 5",
        ),
        (
            "operator_witness_direct_eq",
            r#"Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
1 == 1"#,
        ),
        (
            "operator_witness_bounded_eq",
            r#"Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
same :: <A: Eq> A -> A -> Bool { | x y => x == y; }
same 1 1"#,
        ),
        (
            "operator_witness_bounded_ne_from_eq",
            r#"Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
same :: <A: Eq> A -> A -> Bool { | x y => x != y; }
same 1 1"#,
        ),
        (
            "operator_witness_bounded_lt",
            r#"Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Int :: { (<) = \a b. true; }
less :: <A: Ord> A -> A -> Bool { | x y => x < y; }
less 2 1"#,
        ),
    ]
}

#[test]
fn thir_and_tlc_walkers_agree() {
    let mut divergences = Vec::new();
    for (label, src) in battery() {
        let thir = eval_file(src);
        let tlc = eval_tlc_file(src);
        let agree = match (&thir, &tlc) {
            (Ok(a), Ok(b)) => values_match(a, b),
            (Err(_), Err(_)) => true, // both refuse — acceptable
            _ => false,
        };
        if !agree {
            divergences.push(format!("{label}: THIR={thir:?} TLC={tlc:?}"));
        }
        if let Some(expected) = expected_display(label) {
            match (&thir, &tlc) {
                (Ok(a), Ok(b)) if a.to_string() == expected && b.to_string() == expected => {}
                _ => divergences.push(format!(
                    "{label}: expected both walkers to display {expected:?}, got THIR={thir:?} TLC={tlc:?}"
                )),
            }
        }
    }
    assert!(
        divergences.is_empty(),
        "THIR/TLC walker divergences:\n{}",
        divergences.join("\n")
    );
}

fn expected_display(label: &str) -> Option<&'static str> {
    match label {
        "optional_field_match_none" => Some("1"),
        "optional_field_match_some" => Some("7"),
        "optional_access_chain_some" => Some("5"),
        "optional_access_chain_none" => Some("0"),
        "optional_explicit_some_access" => Some("9"),
        "operator_witness_direct_eq" => Some("false"),
        "operator_witness_bounded_eq" => Some("false"),
        "operator_witness_bounded_ne_from_eq" => Some("true"),
        "operator_witness_bounded_lt" => Some("true"),
        _ => None,
    }
}

/// Compare two forced values structurally via their `Display` form (both
/// walkers `force_deep` before returning, so Display is total).
fn values_match(a: &Value, b: &Value) -> bool {
    a.to_string() == b.to_string()
}
