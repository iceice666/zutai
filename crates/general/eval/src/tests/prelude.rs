use super::*;

// ─── ambient function prelude (stdlib slice B) ───────────────────────────────
//
// `id`/`const`/`compose`/`flip` are ordinary polymorphic source declarations
// injected as an ambient fallback (no import needed). The default `run` path is
// TLC-first; the THIR oracle is checked alongside it, since the milestone wants
// the interpreter and TLC paths to agree on representative higher-order uses.

#[test]
fn prelude_id_applies_polymorphically() {
    assert_eq!(run("id 5"), Value::Int(5));
    assert_eq!(run("id true"), Value::Bool(true));
    assert_eq!(run("id \"x\""), Value::Text("x".into()));
}

#[test]
fn prelude_const_returns_first_argument() {
    assert_eq!(run("const 5 \"x\""), Value::Int(5));
    assert_eq!(run("const \"x\" 5"), Value::Text("x".into()));
}

#[test]
fn prelude_compose_chains_two_functions() {
    // compose (\\x. x + 1) (\\x. x * 2) 3 = (\\x. x + 1) ((\\x. x * 2) 3) = 7.
    assert_eq!(run("compose (\\x. x + 1) (\\x. x * 2) 3"), Value::Int(7));
}

#[test]
fn prelude_flip_swaps_two_arguments() {
    // flip (\\x y. x - y) 3 10 = (\\x y. x - y) 10 3 = 7.
    assert_eq!(run("flip (\\x y. x - y) 3 10"), Value::Int(7));
}

#[test]
fn prelude_not_negates_bool() {
    assert_eq!(run("not false"), Value::Bool(true));
    assert_eq!(run("not true"), Value::Bool(false));
    assert_eq!(run("not (1 == 2)"), Value::Bool(true));
}

#[test]
fn prelude_compose_is_curried_when_partially_applied() {
    // compose f g yields a function; applying it later must work on both paths.
    assert_eq!(
        run("(compose (\\x. x + 1) (\\x. x * 2)) 10"),
        Value::Int(21)
    );
}

#[test]
fn prelude_thir_oracle_matches_tlc_path() {
    // The milestone wants the interpreter (THIR oracle) and TLC paths to agree.
    let srcs = [
        "id 5",
        "const 5 \"x\"",
        "compose (\\x. x + 1) (\\x. x * 2) 3",
        "flip (\\x y. x - y) 3 10",
        "not false",
        "compose (\\x. x + 1) (\\x. x * 2) (id 10)",
    ];
    for src in srcs {
        let tlc = eval_file(src).expect("TLC eval failed");
        let thir = eval_thir_file(src).expect("THIR oracle eval failed");
        assert_eq!(tlc, thir, "TLC and THIR oracle disagree for:\n{src}");
    }
}

#[test]
fn list_prelude_pipeline_maps_filters_and_folds() {
    let src =
        "{1; 2; 3; 4;} |> filter (\\x. x > 1) |> map (\\x. x * 2) |> fold (\\acc x. acc + x) 0";
    assert_eq!(run(src), Value::Int(18));
}

#[test]
fn list_prelude_uncons_head_tail_edges() {
    assert_eq!(run("length {9; 8; 7;}"), Value::Int(3));
    assert_eq!(format!("{}", run("append {1; 2;} {3;}")), "[1; 2; 3]");
    assert_eq!(format!("{}", run("uncons {;}")), "#none");
    assert_eq!(format!("{}", run("head? {7; 8;}")), "#some (7)");
    assert_eq!(format!("{}", run("tail? {7; 8;}")), "#some ([8])");
}

#[test]
fn list_prelude_user_binding_shadows_map() {
    let src = "map :: Int -> Int = x => x + 100;\nmap 5";
    assert_eq!(run(src), Value::Int(105));
}

#[test]
fn list_prelude_thir_oracle_matches_tlc_path() {
    let srcs = [
        "{1; 2; 3; 4;} |> filter (\\x. x > 1) |> map (\\x. x * 2) |> fold (\\acc x. acc + x) 0",
        "append {1; 2;} {3;}",
        "head? {7; 8;}",
        "tail? {7; 8;}",
    ];
    for src in srcs {
        let tlc = eval_file(src).expect("TLC eval failed");
        let thir = eval_thir_file(src).expect("THIR oracle eval failed");
        assert_eq!(tlc, thir, "TLC and THIR oracle disagree for:\n{src}");
    }
}

// ─── user shadowing ──────────────────────────────────────────────────────────

#[test]
fn prelude_user_binding_shadows_id() {
    // A user `id` wins over the ambient fallback; no duplicate-binding error.
    let src = "id :: Int -> Int = x => x + 100;\nid 5";
    assert_eq!(run(src), Value::Int(105));
}

#[test]
fn prelude_user_binding_shadows_compose() {
    let src = "compose :: Int -> Int -> Int = a b => a - b;\ncompose 10 3";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn prelude_user_shadow_does_not_reach_imported_module() {
    // A user `id` shadows the ambient one in the importing module, but the
    // imported `stdlib.prelude` module has its own scope, so `p.id` is still the
    // prelude identity.
    let src = "id :: Int -> Int = x => x + 100;\n\
               p ::= import stdlib.prelude;\n\
               p.id 5";
    assert_eq!(run(src), Value::Int(5));
}
