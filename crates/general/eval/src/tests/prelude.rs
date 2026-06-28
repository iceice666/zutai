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
        "compose (\\x. x + 1) (\\x. x * 2) (id 10)",
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
