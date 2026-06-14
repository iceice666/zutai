//! Golden-semantics test suite for the Zutai THIR reference interpreter.
//!
//! These tests double as the differential-testing oracle for future LLVM
//! codegen: any LLVM output that disagrees with these is a codegen bug.

use crate::{EvalError, Value, eval_file};

// ─── helpers ──────────────────────────────────────────────────────────────────

fn run(src: &str) -> Value {
    eval_file(src).unwrap_or_else(|e| panic!("eval_file failed for:\n{src}\nerror: {e}"))
}

fn run_err(src: &str) -> EvalError {
    eval_file(src).expect_err(&format!("expected error for:\n{src}"))
}

// ─── arithmetic ───────────────────────────────────────────────────────────────

#[test]
fn int_add() {
    assert_eq!(run("1 + 2"), Value::Int(3));
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
    // Function declarations use curly-brace clause syntax: `{ | params => body; }`
    let src = "
inc :: Int -> Int {
  | x => x + 1;
}
inc 41
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn curried_two_arg_function() {
    let src = "
add :: Int -> Int -> Int {
  | x y => x + y;
}
add 2 3
";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn partial_application_returns_closure() {
    let src = "
add :: Int -> Int -> Int {
  | x y => x + y;
}
add_two := add 2
add_two 3
";
    assert_eq!(run(src), Value::Int(5));
}

// ─── recursion ────────────────────────────────────────────────────────────────

#[test]
fn factorial_recursion() {
    // Integer literal patterns in clauses: `| 0 => 1;`
    let src = "
fac :: Int -> Int {
  | 0 => 1;
  | n => n * fac (n - 1);
}
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
    // Optional record field `port?` is absent → evaluates to Nothing → ?? returns default.
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
    // Optional record field is present → evaluates to the value → ?? passes it through.
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

// ─── atom patterns in function clauses ───────────────────────────────────────

#[test]
fn atom_literal_pattern_in_clause() {
    let src = "
Profile :: type [
  #dev;
  #prod;
]
isProd :: Profile -> Bool {
  | #prod => true;
  | #dev => false;
}
isProd #prod
";
    assert_eq!(run(src), Value::Bool(true));
}

// ─── mutual recursion ─────────────────────────────────────────────────────────

#[test]
fn mutual_recursion() {
    let src = "
isEven :: Int -> Bool {
  | 0 => true;
  | n => isOdd (n - 1);
}
isOdd :: Int -> Bool {
  | 0 => false;
  | n => isEven (n - 1);
}
isEven 4
";
    assert_eq!(run(src), Value::Bool(true));
}

// ─── function with guard ──────────────────────────────────────────────────────

#[test]
fn function_with_guard() {
    let src = "
classify :: Int -> Int {
  | n if n > 0 => 1;
  | 0 => 0;
  | _ => -1;
}
classify 5
";
    assert_eq!(run(src), Value::Int(1));
}

// ─── `.zti` imports ───────────────────────────────────────────────────────────

fn imports_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports")
}

/// Evaluate `src` with the shared fixtures directory as the import base.
fn run_import(src: &str) -> Value {
    crate::eval_with_base(src, Some(&imports_dir()))
        .unwrap_or_else(|e| panic!("eval failed for:\n{src}\nerror: {e}"))
}

fn run_import_err(src: &str) -> EvalError {
    crate::eval_with_base(src, Some(&imports_dir()))
        .expect_err(&format!("expected error for:\n{src}"))
}

#[test]
fn import_zti_field_access_int() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.port"),
        Value::Int(8080)
    );
}

#[test]
fn import_zti_field_access_text() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zti_field_access_bool() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.debug"),
        Value::Bool(true)
    );
}

#[test]
fn import_zti_field_access_atom() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.env"),
        Value::Atom("prod".into())
    );
}

#[test]
fn import_zti_nested_field() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.limits.max"),
        Value::Int(100)
    );
}

#[test]
fn import_zti_list_field() {
    match run_import("cfg := import \"config.zti\"\ncfg.tags") {
        Value::List(items) => assert_eq!(items.len(), 2),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn import_zti_whole_record() {
    match run_import("cfg := import \"config.zti\"\ncfg") {
        Value::Record(fields) => assert_eq!(fields.len(), 6),
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn import_via_eval_path() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/importer.zt");
    assert_eq!(crate::eval_path(&path).unwrap(), Value::Int(8080));
}

#[test]
fn import_without_base_is_not_runnable() {
    // `eval_file` has no base directory, so the import cannot resolve.
    match eval_file("cfg := import \"config.zti\"\ncfg.port") {
        Err(EvalError::NotRunnable(_)) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

#[test]
fn import_missing_file_is_not_runnable() {
    match run_import_err("cfg := import \"nope.zti\"\ncfg") {
        EvalError::NotRunnable(_) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

// ─── `.zt` module imports ─────────────────────────────────────────────────────

fn imports_path(name: &str) -> std::path::PathBuf {
    imports_dir().join(name)
}

#[test]
fn zt_import_scalar_value() {
    // other.zt evaluates to the bare integer 42.
    assert_eq!(run_import("n := import \"other.zt\"\nn"), Value::Int(42));
}

#[test]
fn zt_import_record_field() {
    // data_module.zt returns a record whose `doubled` field is 21 * 2.
    assert_eq!(
        run_import("m := import \"data_module.zt\"\nm.doubled"),
        Value::Int(42)
    );
}

#[test]
fn zt_import_whole_record() {
    match run_import("m := import \"data_module.zt\"\nm") {
        Value::Record(fields) => assert_eq!(fields.len(), 3),
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn zt_import_transitive_through_zti() {
    // chain_top.zt imports chain_mid.zt which imports config.zti.
    assert_eq!(
        crate::eval_path(&imports_path("chain_top.zt")).unwrap(),
        Value::Int(8080)
    );
}

#[test]
fn zt_import_function_value_is_refused() {
    // A module whose final value is a function cannot cross the import boundary:
    // a clean refusal, not an internal error or panic.
    match run_import_err("f := import \"func_module.zt\"\nf") {
        EvalError::NotRunnable(_) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

#[test]
fn zt_import_cycle_is_refused() {
    match crate::eval_path(&imports_path("cycle_a.zt")) {
        Err(EvalError::NotRunnable(_)) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

// ─── lambda expressions ───────────────────────────────────────────────────────

#[test]
fn lambda_identity() {
    assert_eq!(run(r"(\x . x) 42"), Value::Int(42));
}

#[test]
fn lambda_add() {
    // Two-parameter lambda applied to two arguments (curried)
    assert_eq!(run(r"(\x y . x + y) 3 4"), Value::Int(7));
}

#[test]
fn lambda_captured_env() {
    // Lambda captures surrounding block binding
    assert_eq!(run(r"{ n := 10; (\x . x + n) 5 }"), Value::Int(15));
}

#[test]
fn lambda_as_value_binding() {
    // Lambda stored in a type-annotated value declaration, then applied
    let src = "
double :: Int -> Int = \\x . x + x
double 7
";
    assert_eq!(run(src), Value::Int(14));
}

#[test]
fn lambda_partial_application() {
    assert_eq!(
        run(r"{ add := \x y . x + y; add_two := add 2; add_two 3 }"),
        Value::Int(5)
    );
}

// ─── match expressions ────────────────────────────────────────────────────────

#[test]
fn match_int_literal() {
    // Matched arm returns Int so both arms have the same type.
    let src = r"
match 0 {
  | 0 => 1;
  | _ => 2;
}
";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn match_wildcard_fallthrough() {
    let src = r"
match 99 {
  | 0 => 1;
  | _ => 2;
}
";
    assert_eq!(run(src), Value::Int(2));
}

#[test]
fn match_bind_pattern() {
    // Binding pattern captures the matched value.
    let src = r"
match 7 {
  | n => n * 2;
}
";
    assert_eq!(run(src), Value::Int(14));
}

#[test]
fn match_with_guard() {
    // Guard filters to the correct arm.
    let src = r"
match 5 {
  | n if n > 3 => 1;
  | _ => 0;
}
";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn match_guard_falls_through() {
    // When the guard fails, the next arm is tried.
    let src = r"
match 2 {
  | n if n > 3 => 1;
  | _ => 0;
}
";
    assert_eq!(run(src), Value::Int(0));
}

#[test]
fn match_bool_patterns() {
    assert_eq!(
        run(r"match true { | true => 1; | false => 0; }"),
        Value::Int(1)
    );
    assert_eq!(
        run(r"match false { | true => 1; | false => 0; }"),
        Value::Int(0)
    );
}

#[test]
fn match_function_using_match_expr() {
    // match expression inside a lambda stored as a value binding
    let src = "
is_zero :: Int -> Bool = \\n . match n {
  | 0 => true;
  | _ => false;
}
is_zero 0
";
    assert_eq!(run(src), Value::Bool(true));
}

// ─── optional access ──────────────────────────────────────────────────────────

#[test]
fn optional_access_present() {
    // `?.` chains through an optional record field that is present.
    // outer.inner has type Optional(Inner); outer.inner?.val returns Int.
    let src = "
Inner :: type { val : Int; }
Outer :: type { inner? : Inner; }
outer :: Outer = { inner = { val = 42; }; }
outer.inner?.val
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn optional_access_absent() {
    // When the optional record field is absent, ?.field returns Nothing.
    let src = "
Inner :: type { val : Int; }
Outer :: type { inner? : Inner; }
outer :: Outer = {}
outer.inner?.val
";
    assert_eq!(run(src), Value::Nothing);
}
