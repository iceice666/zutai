//! Differential oracle: the THIR regression oracle and the strict TLC evaluator
//! must agree on every accepted parity `.zt` program. A divergence is a bug in
//! one of the two evaluators.

use zutai_eval::{
    Value, eval_thir_file, eval_thir_path, eval_thir_with_base, eval_tlc_file, eval_tlc_path,
    eval_tlc_with_base,
};

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
            "f :: Int -> Int\n  = 0 => 1;\n  = n => n * f (n - 1);\nf 5",
        ),
        ("lambda", "(\\x. x * 2) 21"),
        (
            "curry",
            "add :: Int -> Int -> Int\n  = a b => a + b;\nadd 3 4",
        ),
        (
            "coalesce_absent",
            "S :: type { p? : Int; }\ns :: S = {}\ns.p ?? 8080",
        ),
        ("coalesce_explicit_none", "x :: Int? = #none\nx ?? 5"),
        ("coalesce_explicit_some", "x :: Int? = #some (9)\nx ?? 5"),
        (
            "maybe_field_absent_direct",
            "S :: type { p? : Int; }\ns :: S = {}\ns.p",
        ),
        (
            "maybe_field_present_direct",
            "S :: type { p? : Int; }\ns :: S = { p = 7; }\ns.p",
        ),
        (
            "maybe_optional_field_present_none",
            "S :: type { p? : Int?; }\ns :: S = { p = #none; }\ns.p",
        ),
        (
            "maybe_optional_field_present_some",
            "S :: type { p? : Int?; }\ns :: S = { p = #some (7); }\ns.p",
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
            "Inner :: type { val : Int; }\ncfg :: Inner? = #some ({ val = 9; })\ncfg?.val ?? 0",
        ),
        (
            "match_union",
            "Shape :: type { #c: { r: Int; }; #s: { v: Int; }; }\nf :: Shape -> Int\n  = #c { r = r; } => r;\n  = #s { v = v; } => v;\nf (#c { r = 7; })",
        ),
        (
            "guard",
            "f :: Int -> Text\n  = n if n > 0 => \"pos\";\n  = _ => \"nonpos\";\nf 5",
        ),
        (
            "lazy_record_projection",
            "bad :: Int = bad\n{ ok = 1; bad = bad; }.ok",
        ),
        (
            "lazy_block_local",
            "{ y := 10 / 0; if 1 > 2 then y else 99 }",
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
same :: <A: Eq> A -> A -> Bool
  = x y => x == y;
same 1 1"#,
        ),
        (
            "operator_witness_bounded_ne_from_eq",
            r#"Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
same :: <A: Eq> A -> A -> Bool
  = x y => x != y;
same 1 1"#,
        ),
        (
            "operator_witness_bounded_lt",
            r#"Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Int :: { (<) = \a b. true; }
less :: <A: Ord> A -> A -> Bool
  = x y => x < y;
less 2 1"#,
        ),
    ]
}

#[test]
fn thir_and_tlc_walkers_agree() {
    let mut divergences = Vec::new();
    for (label, src) in battery() {
        let thir = eval_thir_file(src);
        let tlc = eval_tlc_file(src);
        let agree = matches!((&thir, &tlc), (Ok(a), Ok(b)) if values_match(a, b));
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

fn imports_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports")
}

#[test]
fn thir_and_tlc_imports_agree() {
    let base = imports_dir();
    let cases = [
        (
            "zti_import",
            "cfg := import \"config.zti\"\ncfg.port",
            "8080",
        ),
        ("zt_import", "n := import \"other.zt\"\nn", "42"),
        (
            "imported_function",
            "f := import \"func_module.zt\"\nf 2 3",
            "5",
        ),
        (
            "optional_import",
            "m := import \"optional_module.zt\"\nm",
            "#none",
        ),
        (
            "imported_operator_witness",
            "w := import \"witness_eq_int_operator.zt\"\n1 == 1",
            "false",
        ),
        (
            "imported_bounded_operator_witness",
            "w := import \"witness_eq_int_operator_bounded.zt\"\nw",
            "false",
        ),
    ];

    for (label, src, expected) in cases {
        let thir = eval_thir_with_base(src, Some(&base));
        let tlc = eval_tlc_with_base(src, Some(&base));
        match (&thir, &tlc) {
            (Ok(a), Ok(b)) if values_match(a, b) && a.to_string() == expected => {}
            _ => panic!(
                "{label}: expected both walkers to display {expected:?}, got THIR={thir:?} TLC={tlc:?}"
            ),
        }
    }

    let path = base.join("chain_top.zt");
    let thir = eval_thir_path(&path);
    let tlc = eval_tlc_path(&path);
    match (&thir, &tlc) {
        (Ok(a), Ok(b)) if values_match(a, b) && a.to_string() == "8080" => {}
        _ => panic!(
            "chain_top.zt: expected both walkers to display \"8080\", got THIR={thir:?} TLC={tlc:?}"
        ),
    }
}

fn expected_display(label: &str) -> Option<&'static str> {
    match label {
        "lazy_record_projection" => Some("1"),
        "lazy_block_local" => Some("99"),
        "maybe_field_absent_direct" => Some("#absent"),
        "maybe_field_present_direct" => Some("#present (7)"),
        "maybe_optional_field_present_none" => Some("#present (#none)"),
        "maybe_optional_field_present_some" => Some("#present (#some (7))"),
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
