use super::*;

// ─── arithmetic ───────────────────────────────────────────────────────────────

#[test]
fn int_add() {
    assert_eq!(run("1 + 2"), Value::Int(3));
}

#[test]
fn fixed_width_int_literal_evaluates_as_int_value() {
    let value = run("255u8");
    assert_eq!(value, Value::Int(255));
    assert_eq!(value.to_string(), "255");
}

#[test]
fn fixed_width_out_of_range_is_refused() {
    let err = run_err("256u8");
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages
            .iter()
            .any(|message| message.contains("out of range") && message.contains("u8")),
        "expected u8 range diagnostic, got {messages:?}"
    );
}

#[test]
fn int_precedence() {
    // 1 + 2 * 3 = 7
    assert_eq!(run("1 + 2 * 3"), Value::Int(7));
}

#[test]
fn int_sub() {
    assert_eq!(run("10 - 3"), Value::Int(7));
}

#[test]
fn int_div_truncates() {
    assert_eq!(run("7 / 2"), Value::Int(3));
}

#[test]
fn int_div_by_zero() {
    assert_eq!(run_err("1 / 0"), EvalError::DivByZero);
}

#[test]
fn float_add() {
    assert_eq!(run("1.0 + 2.0"), Value::Float(3.0));
}

#[test]
fn posit_arithmetic_preserves_posit_format() {
    assert_eq!(run("1p32 + 2p32").to_string(), "3p32");
    assert_eq!(run("1p64 + 2p64").to_string(), "3p64");
    assert_eq!(run("1p32e3 + 2p32e3").to_string(), "3p32e3");
    assert_eq!(run("4p64e5 / 2p64e5").to_string(), "2p64e5");
}

// ─── comparison ───────────────────────────────────────────────────────────────

#[test]
fn int_eq_true() {
    assert_eq!(run("1 == 1"), Value::Bool(true));
}

#[test]
fn int_eq_false() {
    assert_eq!(run("1 == 2"), Value::Bool(false));
}

#[test]
fn int_ne() {
    assert_eq!(run("1 != 2"), Value::Bool(true));
}

#[test]
fn int_lt() {
    assert_eq!(run("1 < 2"), Value::Bool(true));
    assert_eq!(run("2 < 1"), Value::Bool(false));
    assert_eq!(run("1 < 1"), Value::Bool(false));
}

#[test]
fn int_le() {
    assert_eq!(run("1 <= 1"), Value::Bool(true));
    assert_eq!(run("2 <= 1"), Value::Bool(false));
}

#[test]
fn int_gt() {
    assert_eq!(run("2 > 1"), Value::Bool(true));
}

#[test]
fn int_ge() {
    assert_eq!(run("1 >= 1"), Value::Bool(true));
}

// ─── boolean short-circuit ────────────────────────────────────────────────────

#[test]
fn and_short_circuits() {
    assert_eq!(run("false && true"), Value::Bool(false));
}

#[test]
fn or_short_circuits() {
    assert_eq!(run("true || false"), Value::Bool(true));
}

#[test]
fn and_both_true() {
    assert_eq!(run("true && true"), Value::Bool(true));
}

#[test]
fn or_both_false() {
    assert_eq!(run("false || false"), Value::Bool(false));
}

// ─── if / else ────────────────────────────────────────────────────────────────

#[test]
fn if_then_branch() {
    assert_eq!(run("if true then 1 else 2"), Value::Int(1));
}

#[test]
fn if_else_branch() {
    assert_eq!(run("if false then 1 else 2"), Value::Int(2));
}

// ─── let blocks ───────────────────────────────────────────────────────────────

#[test]
fn block_single_binding() {
    assert_eq!(run("{ a := 42; a }"), Value::Int(42));
}

#[test]
fn block_sequential_bindings() {
    assert_eq!(run("{ a := 1; b := a + 1; b }"), Value::Int(2));
}

// ─── records and field access ─────────────────────────────────────────────────

#[test]
fn record_field_access() {
    // Records require a trailing `;` after each field.
    assert_eq!(run("{ x = 10; y = 20; }.x"), Value::Int(10));
}

#[test]
fn record_equality() {
    assert_eq!(run("{ x = 1; } == { x = 1; }"), Value::Bool(true));
    assert_eq!(run("{ x = 1; } == { x = 2; }"), Value::Bool(false));
}

#[test]
fn record_update_replaces_only_named_field() {
    let v = run(r#"{ host = "h"; port = 80; } with { port = 8080; }"#);
    assert_eq!(record_field_value(&v, "host"), Value::Text("h".into()));
    assert_eq!(record_field_value(&v, "port"), Value::Int(8080));
}

#[test]
fn record_update_does_not_force_unchanged_fields() {
    let src = r#"
bad :: Int = bad
({ host = "h"; port = bad; } with { host = "new"; }).host
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Text("new".into()));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Text("new".into()));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Text("new".into()));
}

#[test]
fn record_update_preserves_present_optional_field_in_both_evaluators() {
    let src = r#"
S :: type { x : Int; y? : Int; }
s :: S = { x = 1; y = 2; }
(s with { x = 3; }).y
"#;
    assert_eq!(eval_file(src).unwrap().to_string(), "#present (2)");
    assert_eq!(eval_tlc_file(src).unwrap().to_string(), "#present (2)");
    assert_eq!(eval_thir_file(src).unwrap().to_string(), "#present (2)");
}

#[test]
fn record_update_preserves_absent_optional_field_in_both_evaluators() {
    let src = r#"
S :: type { x : Int; y? : Int; }
s :: S = { x = 1; }
(s with { x = 3; }).y
"#;
    assert_eq!(eval_file(src).unwrap().to_string(), "#absent");
    assert_eq!(eval_tlc_file(src).unwrap().to_string(), "#absent");
    assert_eq!(eval_thir_file(src).unwrap().to_string(), "#absent");
}

#[test]
fn overlay_replaces_patch_fields_in_both_evaluators() {
    let src = r#"
Config :: type {
  host : Text;
  port : Int;
}
base :: Config = { host = "localhost"; port = 80; }
patch :: Patch Config = { port = 8080; }
(overlay base patch).port
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Int(8080));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(8080));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Int(8080));
}

#[test]
fn overlay_does_not_force_unchanged_fields() {
    let src = r#"
Config :: type {
  host : Text;
  port : Int;
}
bad :: Int = bad
base :: Config = { host = "localhost"; port = bad; }
patch :: Patch Config = { host = "patched"; }
(overlay base patch).host
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Text("patched".into()));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Text("patched".into()));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Text("patched".into()));
}

#[test]
fn overlay_deep_merges_nested_records_in_both_evaluators() {
    let src = r#"
Server :: type {
  host : Text;
  port : Int;
}
Config :: type {
  server : Server;
  name : Text;
}
base :: Config = {
  server = { host = "localhost"; port = 80; };
  name = "dev";
}

patch :: DeepPatch Config = {
  server = { port = 8080; };
}
(overlayDeep base patch).server.host
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Text("localhost".into()));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Text("localhost".into()));
    assert_eq!(
        eval_thir_file(src).unwrap(),
        Value::Text("localhost".into())
    );
}

#[test]
fn overlay_partial_application_runs_in_both_evaluators() {
    let src = r#"
Config :: type {
  host : Text;
  port : Int;
}
base :: Config = { host = "localhost"; port = 80; }
applyBase ::= overlay base
patch :: Patch Config = { port = 8080; }
(applyBase patch).port
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Int(8080));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(8080));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Int(8080));
}

// ─── select projection ─────────────────────────────────────────────────────────

#[test]
fn select_projects_fields_in_requested_order() {
    // `select` projects exactly the named fields, in the requested order,
    // dropping the unselected `name`.
    let v = run("s ::= { host = \"h\"; port = 8080; name = \"n\"; }\nselect s { port; host; }");
    match v {
        Value::Record(fields) => {
            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_ref()).collect();
            assert_eq!(names, ["port", "host"]);
        }
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn select_preserves_field_values() {
    assert_eq!(
        run("s ::= { host = \"h\"; port = 8080; }\n(select s { port; }).port"),
        Value::Int(8080)
    );
}

#[test]
fn select_unknown_field_is_type_check_failure() {
    // An unknown selected field is a type error, so the interpreter refuses to
    // evaluate (evaluation is gated on complete typed IR).
    let err = run_err("s ::= { host = \"h\"; }\nselect s { missing; }");
    let EvalError::TypeCheckFailed(msgs) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        msgs.iter().any(|m| m.contains("missing")),
        "expected an unknown-field diagnostic mentioning `missing`, got {msgs:?}"
    );
}

// ─── lists ────────────────────────────────────────────────────────────────────

#[test]
fn list_equality() {
    // Lists require trailing `;` in Zutai syntax.
    assert_eq!(run("[1; 2; 3;] == [1; 2; 3;]"), Value::Bool(true));
    assert_eq!(run("[1; 2; 3;] == [1; 2; 4;]"), Value::Bool(false));
}

// ─── tuples ───────────────────────────────────────────────────────────────────

#[test]
fn tuple_equality() {
    assert_eq!(run("(1, 2) == (1, 2)"), Value::Bool(true));
    assert_eq!(run("(1, 2) == (1, 3)"), Value::Bool(false));
}

// ─── string and atom equality ─────────────────────────────────────────────────

#[test]
fn string_equality() {
    assert_eq!(run("\"hello\" == \"hello\""), Value::Bool(true));
    assert_eq!(run("\"hello\" == \"world\""), Value::Bool(false));
}

#[test]
fn atom_equality() {
    // Each atom literal has a singleton type (Atom("prod"), Atom("dev"), etc.).
    // Same-typed atoms are equal to themselves.
    assert_eq!(run("#prod == #prod"), Value::Bool(true));
    // Note: `#prod == #dev` is a THIR *type error* — Atom("prod") ≠ Atom("dev") as types.
    // Comparing atoms of different kinds requires a union type context.
}

// ─── top-level value binding ──────────────────────────────────────────────────

#[test]
fn top_level_value_binding() {
    // Type-annotated value binding: `name :: Type = value`
    let src = "
answer :: Int = 42
answer
";
    assert_eq!(run(src), Value::Int(42));
}

// ─── top-level function call ──────────────────────────────────────────────────

#[test]
fn top_level_function_call() {
    // Function declarations use equals-prefixed clauses after the signature.
    let src = "
inc :: Int -> Int
  = x => x + 1;
inc 41
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn curried_two_arg_function() {
    let src = "
add :: Int -> Int -> Int
  = x y => x + y;
add 2 3
";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn partial_application_returns_closure() {
    let src = "
add :: Int -> Int -> Int
  = x y => x + y;
add_two ::= add 2
add_two 3
";
    assert_eq!(run(src), Value::Int(5));
}

// ─── recursion ────────────────────────────────────────────────────────────────

#[test]
fn factorial_recursion() {
    // Integer literal patterns in clauses: `| 0 => 1;`
    let src = "
fac :: Int -> Int
  = 0 => 1;
  = n => n * fac (n - 1);
fac 5
";
    assert_eq!(run(src), Value::Int(120));
}

// ─── black-hole detection ─────────────────────────────────────────────────────

#[test]
fn black_hole_detected() {
    // `x :: Int = x` type-checks (both sides are Int) but diverges at runtime.
    let src = "
x :: Int = x
x
";
    assert_eq!(run_err(src), EvalError::BlackHole);
}

#[test]
fn strict_tlc_black_hole_detected() {
    let src = "
x :: Int = x
x
";
    assert_eq!(eval_tlc_file(src), Err(EvalError::BlackHole));
}

#[test]
fn tlc_lazy_record_projection_skips_unselected_black_hole() {
    let src = "bad :: Int = bad\n{ ok = 1; bad = bad; }.ok";
    assert_eq!(eval_file(src).unwrap(), Value::Int(1));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(1));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Int(1));
}

// ─── gate refusal — type errors must never produce a value ────────────────────

#[test]
fn gate_refuses_type_error() {
    let src = "
x :: Int = \"bad\"
x
";
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::TypeCheckFailed(_)),
        "expected TypeCheckFailed, got {err:?}"
    );
}

#[test]
fn gate_refuses_parse_error() {
    // Lists without trailing `;` fail to parse in general mode.
    let src = "[1; 2]";
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::NotRunnable(_)),
        "expected NotRunnable, got {err:?}"
    );
}

// ─── coalesce (??) ────────────────────────────────────────────────────────────

#[test]
fn coalesce_absent_optional_field() {
    // Optional record field `port?` is absent → #absent → ?? returns default.
    let src = "
RawServer :: type {
  port? : Int;
}
server :: RawServer = {}
server.port ?? 8080
";
    assert_eq!(run(src), Value::Int(8080));
}

#[test]
fn coalesce_present_optional_field() {
    // Optional record field is present → #present wraps the value → ?? unwraps it.
    let src = "
RawServer :: type {
  port? : Int;
}
server :: RawServer = {
  port = 9000;
}
server.port ?? 8080
";
    assert_eq!(run(src), Value::Int(9000));
}

#[test]
fn coalesce_explicit_none_takes_default() {
    // Regression: an explicit `#none` optional value must default, not pass
    // through. `??` unwraps one Optional or Maybe wrapper.
    let src = "x :: Int? = #none\nx ?? 5";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn coalesce_explicit_some_unwraps_value() {
    // Regression: an explicit `#some (x)` must unwrap to `x`.
    let src = "x :: Int? = #some (9)\nx ?? 5";
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn coalesce_explicit_some_text_unwraps() {
    let src = "x :: Text? = #some (\"hi\")\nx ?? \"def\"";
    assert_eq!(run(src), Value::Text("hi".into()));
}

#[test]
fn coalesce_maybe_absent_takes_default() {
    let src = "
S :: type { p? : Int; }
s :: S = {}
s.p ?? 5
";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn coalesce_maybe_present_unwraps_value() {
    let src = "
S :: type { p? : Int; }
s :: S = { p = 9; }
s.p ?? 5
";
    assert_eq!(run(src), Value::Int(9));
}

// ─── atom patterns in function clauses ───────────────────────────────────────

#[test]
fn atom_literal_pattern_in_clause() {
    let src = "
Profile :: type {
  #dev;
  #prod;
}
isProd :: Profile -> Bool
  = #prod => true;
  = #dev => false;
isProd #prod
";
    assert_eq!(run(src), Value::Bool(true));
}

// ─── mutual recursion ─────────────────────────────────────────────────────────

#[test]
fn mutual_recursion() {
    let src = "
isEven :: Int -> Bool
  = 0 => true;
  = n => isOdd (n - 1);
isOdd :: Int -> Bool
  = 0 => false;
  = n => isEven (n - 1);
isEven 4
";
    assert_eq!(run(src), Value::Bool(true));
}

// ─── function with guard ──────────────────────────────────────────────────────

#[test]
fn function_with_guard() {
    let src = "
classify :: Int -> Int
  = n if n > 0 => 1;
  = 0 => 0;
  = _ => -1;
classify 5
";
    assert_eq!(run(src), Value::Int(1));
}

// ─── regression: NaN comparisons (IEEE 754 unordered) ─────────────────────────

#[test]
fn nan_ordered_comparisons_are_false() {
    // NaN is unordered: `<`, `<=`, `>`, `>=` against NaN must all be false, in
    // BOTH the default TLC evaluator and the THIR oracle. Regression for the
    // `partial_cmp(..).unwrap_or(Ordering::{Less,Equal})` NaN bug that returned
    // `true` for `NaN <= x` / `NaN >= x`.
    for op in ["<", "<=", ">", ">="] {
        let src = format!("0.0 / 0.0 {op} 1.0");
        assert_eq!(run(&src), Value::Bool(false), "TLC: NaN {op} 1.0");
        assert_eq!(
            eval_thir_file(&src).unwrap(),
            Value::Bool(false),
            "THIR oracle: NaN {op} 1.0"
        );
    }
}

// ─── regression: unit argument against an inferred parameter type ─────────────

#[test]
fn unit_argument_against_inferred_param_type_checks() {
    // `(\x. 5) ()` — the lambda parameter type is an unsolved infer var; passing
    // `()` must unify it with the unit tuple, not be rejected as "expected tuple".
    assert_eq!(run("(\\x. 5) ()"), Value::Int(5));
}
