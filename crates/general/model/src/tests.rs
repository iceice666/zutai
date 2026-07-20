//! Engine unit tests: canonical equality, first-order rejection, interface
//! validation, and an end-to-end search.

use std::sync::Once;

use crate::{CheckOptions, CheckOutcome, ModelError, PassedKind, check_analysis};

static STDLIB: Once = Once::new();

fn analyze(src: &str) -> zutai_semantic::Analysis {
    STDLIB.call_once(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../stdlib");
        zutai_semantic::configure_stdlib_root(root).expect("configure test stdlib root");
    });
    zutai_semantic::analyze(src)
}

fn check(src: &str) -> Result<CheckOutcome, ModelError> {
    let analysis = analyze(src);
    assert!(
        analysis.is_thir_complete(),
        "model program did not type-check: {:?}",
        analysis.diagnostics
    );
    check_analysis(&analysis, CheckOptions::default())
}

/// Check a well-typed model that the engine is expected to reject at decode or
/// search time.
fn check_err(src: &str) -> ModelError {
    check(src).expect_err("expected the engine to reject this model")
}

fn passed(outcome: CheckOutcome) -> Vec<crate::ScenarioReport> {
    match outcome {
        CheckOutcome::Passed { scenarios } => scenarios,
        other => panic!("expected Passed, got {other:?}"),
    }
}

/// Always-true safety (`ok`) and reachability (`rok`) predicates over `state_ty`,
/// so canonicalization tests never fail an obligation.
fn always_true_preds(state_ty: &str) -> String {
    format!(
        "ok :: {{ name : Text; holds : {state_ty} -> Bool; }} = {{ name = \"ok\"; holds = \\s. true; }};\nrok :: {{ name : Text; reached : {state_ty} -> Bool; }} = {{ name = \"rok\"; reached = \\s. true; }};\n"
    )
}

/// A `next` that produces no successors, typed for `state_ty`.
fn dead_next(state_ty: &str) -> String {
    format!("next :: {state_ty} -> List {{ action : Text; state : {state_ty}; }} = s => {{;}};\n")
}

/// Wrap decls plus a model record and one scenario expectation into a program.
fn program(decls: &str, model: &str, expect: &str) -> String {
    format!(
        "{decls}model ::= {model};\n{{ scenarios = {{ {{ name = \"s\"; model = model; expect = {expect}; }}; }}; }}\n"
    )
}

/// A model whose state type is bound to the alias `St = state_ty` (so tuple and
/// record state annotations parse unambiguously), with the given `initial`
/// list, always-true safety and reachability, and a `next` producing no
/// successors.
fn dead_model(state_ty: &str, initial: &str, expect: &str) -> String {
    let decls = format!(
        "St :: type {state_ty};\n{}{}",
        dead_next("St"),
        always_true_preds("St")
    );
    let model = format!(
        "{{ initial = {initial}; next = next; safety = {{ ok; }}; reachability = {{ rok; }}; }}"
    );
    program(&decls, &model, expect)
}

/// A 3-state counter model: `0 -> 1 -> 2`, safe below 5, reaches 2.
fn counter_safe() -> String {
    let decls = concat!(
        "next :: Int -> List { action : Text; state : Int; }\n",
        "  = n => cond { n < 2 => { { action = \"inc\"; state = n + 1; }; }; _ => {;}; };\n",
        "bounded :: { name : Text; holds : Int -> Bool; }\n",
        "  = { name = \"bounded\"; holds = \\n. n <= 5; };\n",
        "hitsTwo :: { name : Text; reached : Int -> Bool; }\n",
        "  = { name = \"hitsTwo\"; reached = \\n. n == 2; };\n",
    );
    program(
        decls,
        "{ initial = { 0; }; next = next; safety = { bounded; }; reachability = { hitsTwo; }; }",
        "#safe",
    )
}

#[test]
fn end_to_end_safe_counter_visits_three_states() {
    let scenarios = passed(check(&counter_safe()).expect("counter should check"));
    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0].name, "s");
    assert_eq!(scenarios[0].visited, 3);
    assert_eq!(scenarios[0].kind, PassedKind::Safe);
}

// ─── canonical fingerprint equality ─────────────────────────────────────────────

#[test]
fn record_field_order_is_canonical() {
    let src = dead_model(
        "{ a : Int; b : Int; }",
        "{ { a = 1; b = 2; }; { b = 2; a = 1; }; }",
        "#safe",
    );
    let scenarios = passed(check(&src).expect("record-order model should check"));
    assert_eq!(scenarios[0].visited, 1, "differently-ordered records dedup");
}

#[test]
fn tagged_record_payload_order_is_canonical() {
    let decls = concat!(
        "St :: type { #pair : { x : Int; y : Int; }; };\n",
        "next :: St -> List { action : Text; state : St; } = s => {;};\n",
        "ok :: { name : Text; holds : St -> Bool; } = { name = \"ok\"; holds = \\s. true; };\n",
        "rok :: { name : Text; reached : St -> Bool; } = { name = \"rok\"; reached = \\s. true; };\n",
    );
    let model = "{ initial = { #pair { x = 1; y = 2; }; #pair { y = 2; x = 1; }; }; next = next; safety = { ok; }; reachability = { rok; }; }";
    let src = program(decls, model, "#safe");
    let scenarios = passed(check(&src).expect("tagged-payload model should check"));
    assert_eq!(
        scenarios[0].visited, 1,
        "record-style tagged payloads dedup by name-sorted order"
    );
}

#[test]
fn list_elements_are_positional() {
    let src = dead_model("List Int", "{ { 1; 2; }; { 2; 1; }; }", "#safe");
    let scenarios = passed(check(&src).expect("positional-list model should check"));
    assert_eq!(
        scenarios[0].visited, 2,
        "reordered lists are distinct states"
    );
}

#[test]
fn tuple_items_are_positional() {
    let src = dead_model("(Int, Int)", "{ (1, 2); (2, 1); }", "#safe");
    let scenarios = passed(check(&src).expect("positional-tuple model should check"));
    assert_eq!(
        scenarios[0].visited, 2,
        "reordered tuples are distinct states"
    );
}

// ─── non-first-order rejection ──────────────────────────────────────────────────

#[test]
fn float_state_is_rejected() {
    let src = dead_model("Float", "{ 1.5; }", "#safe");
    assert!(matches!(
        check_err(&src),
        ModelError::NonFirstOrderState("Float")
    ));
}

#[test]
fn function_in_state_is_rejected() {
    let src = dead_model("{ f : Int -> Int; }", "{ { f = \\x. x; }; }", "#safe");
    assert!(matches!(
        check_err(&src),
        ModelError::NonFirstOrderState("Function")
    ));
}

#[test]
fn builtin_in_state_is_rejected() {
    // `listEmpty` is a compiler-provided builtin value seeded by the evaluator.
    let src = dead_model(
        "{ f : Unit -> List Int; }",
        "{ { f = listEmpty; }; }",
        "#safe",
    );
    assert!(matches!(
        check_err(&src),
        ModelError::NonFirstOrderState("Builtin")
    ));
}

#[test]
fn type_export_erasure_in_state_is_rejected_as_nothing() {
    // `T = T` in a runtime record is erased by `TlcSession` to internal
    // `Nothing`; model states reject it rather than treating it as data.
    let src = concat!(
        "T :: type Int;\n",
        "St :: type { T : Type; };\n",
        "next :: St -> List { action : Text; state : St; } = s => {;};\n",
        "ok :: { name : Text; holds : St -> Bool; } = { name = \"ok\"; holds = \\s. true; };\n",
        "rok :: { name : Text; reached : St -> Bool; } = { name = \"rok\"; reached = \\s. true; };\n",
        "model ::= { initial = { { T = T; }; }; next = next; safety = { ok; }; reachability = { rok; }; };\n",
        "{ scenarios = { { name = \"s\"; model = model; expect = #safe; }; }; }\n",
    );
    assert!(matches!(
        check_err(src),
        ModelError::NonFirstOrderState("Nothing")
    ));
}

#[test]
fn posit_state_is_rejected() {
    let src = dead_model("Posit64e5", "{ 1p64e5; }", "#safe");
    assert!(matches!(
        check_err(&src),
        ModelError::NonFirstOrderState("Posit")
    ));
}

#[test]
fn function_action_is_rejected() {
    let src = concat!(
        "Action :: type Int -> Int;\n",
        "next :: Int -> List { action : Action; state : Int; } = s => { { action = \\x. x; state = s + 1; }; };\n",
        "ok :: { name : Text; holds : Int -> Bool; } = { name = \"ok\"; holds = \\s. true; };\n",
        "rok :: { name : Text; reached : Int -> Bool; } = { name = \"rok\"; reached = \\s. true; };\n",
        "model ::= { initial = { 0; }; next = next; safety = { ok; }; reachability = { rok; }; };\n",
        "{ scenarios = { { name = \"s\"; model = model; expect = #safe; }; }; }\n",
    );
    assert!(matches!(
        check_err(src),
        ModelError::NonFirstOrderAction("Function")
    ));
}

#[test]
fn violates_ignores_non_target_safety_failures() {
    let src = concat!(
        "next :: Int -> List { action : Text; state : Int; } = s => cond { s < 1 => { { action = \"step\"; state = 1; }; }; _ => {;}; };\n",
        "other :: { name : Text; holds : Int -> Bool; } = { name = \"other\"; holds = \\s. false; };\n",
        "target :: { name : Text; holds : Int -> Bool; } = { name = \"target\"; holds = \\s. s != 1; };\n",
        "rok :: { name : Text; reached : Int -> Bool; } = { name = \"rok\"; reached = \\s. true; };\n",
        "model ::= { initial = { 0; }; next = next; safety = { other; target; }; reachability = { rok; }; };\n",
        "{ scenarios = { { name = \"s\"; model = model; expect = #violates { property = \"target\"; }; }; }; }\n",
    );
    let scenarios = passed(check(src).expect("target violation should pass"));
    assert_eq!(
        scenarios[0].kind,
        PassedKind::ExpectedViolation {
            property: "target".to_owned()
        }
    );
}
#[test]
fn reachability_only_stops_after_all_obligations_are_met() {
    let src = concat!(
        "next :: Int -> List { action : Text; state : Int; } = n => { { action = \"step\"; state = n + 1; }; };\n",
        "reached :: { name : Text; reached : Int -> Bool; } = { name = \"initial\"; reached = \\n. n == 0; };\n",
        "noSafety :: List { name : Text; holds : Int -> Bool; } = {;};\n",
        "model ::= { initial = { 0; }; next = next; safety = noSafety; reachability = { reached; }; };\n",
        "{ scenarios = { { name = \"s\"; model = model; expect = #safe; }; }; }\n",
    );
    let analysis = analyze(src);
    assert!(analysis.is_thir_complete(), "model did not type-check");
    let scenarios = passed(
        check_analysis(&analysis, CheckOptions { max_states: 1 })
            .expect("met reachability should stop before expanding successors"),
    );
    assert_eq!(scenarios[0].visited, 1);
}

#[test]
fn reports_every_unmet_reachability_obligation() {
    let src = concat!(
        "next :: Int -> List { action : Text; state : Int; } = n => {;};\n",
        "first :: { name : Text; reached : Int -> Bool; } = { name = \"first\"; reached = \\n. n == 1; };\n",
        "second :: { name : Text; reached : Int -> Bool; } = { name = \"second\"; reached = \\n. n == 2; };\n",
        "noSafety :: List { name : Text; holds : Int -> Bool; } = {;};\n",
        "model ::= { initial = { 0; }; next = next; safety = noSafety; reachability = { first; second; }; };\n",
        "{ scenarios = { { name = \"s\"; model = model; expect = #safe; }; }; }\n",
    );
    let outcome = check(src).expect("model check should complete");
    let CheckOutcome::Failed { message, .. } = outcome else {
        panic!("expected reachability failure, got {outcome:?}");
    };
    assert_eq!(
        message,
        "scenario \"s\": FAILED reachability obligations never reached:\n  - \"first\"\n  - \"second\""
    );
}

// ─── interface validation ───────────────────────────────────────────────────────

#[test]
fn missing_next_field_is_rejected() {
    let decls = always_true_preds("Int");
    let model = "{ initial = { 0; }; safety = { ok; }; reachability = { rok; }; }";
    let src = program(&decls, model, "#safe");
    assert!(matches!(check_err(&src), ModelError::MissingField(f) if f == "next"));
}

#[test]
fn duplicate_safety_name_is_rejected() {
    let decls = format!(
        "{}{}ok2 :: {{ name : Text; holds : Int -> Bool; }} = {{ name = \"ok\"; holds = \\s. true; }};\n",
        dead_next("Int"),
        always_true_preds("Int")
    );
    let model =
        "{ initial = { 0; }; next = next; safety = { ok; ok2; }; reachability = { rok; }; }";
    let src = program(&decls, model, "#safe");
    assert!(matches!(check_err(&src), ModelError::DuplicateSafety(n) if n == "ok"));
}

#[test]
fn duplicate_scenario_name_is_rejected() {
    let decls = format!(
        "{}{}model ::= {{ initial = {{ 0; }}; next = next; safety = {{ ok; }}; reachability = {{ rok; }}; }};\n",
        dead_next("Int"),
        always_true_preds("Int")
    );
    let src = format!(
        "{decls}{{ scenarios = {{ {{ name = \"same\"; model = model; expect = #safe; }}; {{ name = \"same\"; model = model; expect = #safe; }}; }}; }}\n"
    );
    assert!(matches!(check_err(&src), ModelError::DuplicateScenario(n) if n == "same"));
}

#[test]
fn violates_unknown_property_is_rejected() {
    let src = dead_model("Int", "{ 0; }", "#violates { property = \"ghost\"; }");
    assert!(matches!(check_err(&src), ModelError::UnknownViolatesProperty(n) if n == "ghost"));
}

#[test]
fn zero_state_limit_is_rejected() {
    let analysis = analyze(&counter_safe());
    assert!(matches!(
        check_analysis(&analysis, CheckOptions { max_states: 0 }),
        Err(ModelError::ZeroStateLimit)
    ));
}
