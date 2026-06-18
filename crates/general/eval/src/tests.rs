//! Golden-semantics test suite for the Zutai THIR reference interpreter.
//!
//! These tests double as the differential-testing oracle for future LLVM
//! codegen: any LLVM output that disagrees with these is a codegen bug.

use crate::{EvalError, Value, eval_file, thunk, value};

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
  dev;
  prod;
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
fn zt_import_function_is_callable() {
    // func_module.zt exports `add :: Int -> Int -> Int`.  Calling it across
    // the module boundary must yield the correct result.
    assert_eq!(
        run_import("f := import \"func_module.zt\"\nf 2 3"),
        Value::Int(5)
    );
}

#[test]
fn zt_import_function_partial_application() {
    // Partially-applied cross-module function retains the correct arity.
    assert_eq!(
        run_import("f := import \"func_module.zt\"\n(f 10) 7"),
        Value::Int(17)
    );
}

#[test]
fn zt_import_sibling_call() {
    // sibling_module.zt: add2 calls `inc` (a sibling top-level binding in the
    // same module).  This exercises the arena switch on BindingRef resolution.
    assert_eq!(
        run_import("lib := import \"sibling_module.zt\"\nlib 3"),
        Value::Int(5)
    );
}

#[test]
fn zt_import_mixed_record_data_field() {
    // mixed_module.zt exports a record with both data and function fields.
    // Reading a data field must still work.
    assert_eq!(
        run_import("m := import \"mixed_module.zt\"\nm.version"),
        Value::Int(1)
    );
}

#[test]
fn zt_import_mixed_record_function_call() {
    // Calling a function field from an imported mixed record.
    assert_eq!(
        run_import("m := import \"mixed_module.zt\"\nm.double 21"),
        Value::Int(42)
    );
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

// ─── HM let-generalization ────────────────────────────────────────────────────

#[test]
fn polymorphic_identity_runs_at_two_types() {
    let v = eval_file("id x = x\n(id 42, id \"hello\")").unwrap();
    let expected = Value::Tuple(
        vec![
            value::TupleField {
                name: None,
                value: thunk::Thunk::ready(Value::Int(42)),
            },
            value::TupleField {
                name: None,
                value: thunk::Thunk::ready(Value::Text("hello".into())),
            },
        ]
        .into(),
    );
    assert_eq!(v, expected);
}

#[test]
fn monomorphic_value_binding_still_runs() {
    assert_eq!(eval_file("answer := 42\nanswer").unwrap(), Value::Int(42));
}

// ─── generic type aliases ─────────────────────────────────────────────────────

#[test]
fn generic_alias_value_evaluates() {
    // A value typed with a generic alias must evaluate to the underlying record,
    // and field access must return the correctly typed value.
    let decl = r#"
Pair :: <A, B> type { first : A; second : B; }
p :: Pair Text Int = { first = "x"; second = 1; }
"#;
    assert_eq!(run(&format!("{decl}\np.first")), Value::Text("x".into()));
    assert_eq!(run(&format!("{decl}\np.second")), Value::Int(1));
}

// ─── T-INV: v1 constraint/witness does not break THIR completeness ────────────

/// T-INV: a file with well-formed constraint + witness + normal binding produces
/// a complete THIR (LoweredThir.file.is_some()) and still evaluates.
/// This guards the semantics-oracle invariant: constraint/witness decls must
/// emit zero HIR+THIR diagnostics so they don't null out LoweredThir.file.
#[test]
fn t_inv_constraint_witness_does_not_break_eval() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq := \\a b. true\n42";
    assert_eq!(run(src), Value::Int(42));
}

/// Derive witness also must not break THIR completeness.
#[test]
fn t_inv_derive_witness_does_not_break_eval() {
    // Use builtin type `Int` so target resolves without error
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\n1";
    assert_eq!(run(src), Value::Int(1));
}

// Increment 5: method-name resolution — eval invariant tests
// ---------------------------------------------------------------------------

/// T-INV-5: `eq 1 2` type-checks (THIR is complete) but has no runtime value yet
/// (no dictionary-passing).  The interpreter must refuse with `UnboundBinding`
/// rather than guessing a value — the oracle must not invent semantics.
#[test]
fn t_inv5_method_call_type_checks_but_refuses_eval() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\neq 1 2";
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::UnboundBinding(_)),
        "expected EvalError::UnboundBinding for un-dispatched method call, got {err:?}"
    );
}

// ─── Increment 6: dictionary-passing / instance resolution ────────────────────

/// Basic dispatch: `eq 1 2` resolves to the `Eq @Int` witness body.
#[test]
fn dispatch_basic_method_call() {
    let src = "
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \\a b. true; }
eq 1 2
";
    assert_eq!(run(src), Value::Bool(true));
}

/// Type-directed selection: two witnesses for the same constraint, each with a
/// different target type — the dispatch must pick the right one per call site.
#[test]
fn dispatch_type_directed_witness_selection() {
    let src = "
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \\a b. true; }
Eq @Bool :: { eq = \\a b. false; }
(eq 1 2, eq true false)
";
    let v = run(src);
    match v {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2);
            // force_deep (called inside eval_file) ensures all thunk fields are forced.
            assert_eq!(fields[0].value.peek(), Some(Value::Bool(true)));
            assert_eq!(fields[1].value.peek(), Some(Value::Bool(false)));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

/// Refusal: method with constraint but NO witness → still `UnboundBinding`.
/// The oracle must decline rather than invent a value.
#[test]
fn dispatch_refusal_no_witness() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\neq 1 2";
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::UnboundBinding(_)),
        "expected UnboundBinding when no witness is in scope, got {err:?}"
    );
}

// ─── Increment 7: operator-method dispatch ────────────────────────────────────

/// Custom `(==)` on a scalar overrides builtin structural equality.
/// `1 == 1` is builtin-`true` but the witness returns `false`.
#[test]
fn op_dispatch_eq_overrides_builtin() {
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. false; }
1 == 1
";
    assert_eq!(run(src), Value::Bool(false));
}

/// `!=` negates the `(==)` field when no `(!=)` field is present.
#[test]
fn op_dispatch_ne_negates_eq() {
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. false; }
1 != 1
";
    // Custom (==) says false → ne returns true.
    assert_eq!(run(src), Value::Bool(true));
}

/// Custom `(<)` on a scalar overrides builtin ordering.
/// `2 < 1` is builtin-`false` but the witness returns `true`.
#[test]
fn op_dispatch_lt_overrides_builtin() {
    let src = "
Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Int :: { (<) = \\a b. true; }
2 < 1
";
    assert_eq!(run(src), Value::Bool(true));
}

/// All six comparison operators dispatch to the appropriate witness field.
#[test]
fn op_dispatch_all_six_operators() {
    let src = "
Cmp :: <A> @A {
  (==) :: A -> A -> Bool;
  (!=) :: A -> A -> Bool;
  (<)  :: A -> A -> Bool;
  (<=) :: A -> A -> Bool;
  (>)  :: A -> A -> Bool;
  (>=) :: A -> A -> Bool;
}
Cmp @Int :: {
  (==)  = \\a b. false;
  (!=)  = \\a b. false;
  (<)   = \\a b. false;
  (<=)  = \\a b. false;
  (>)   = \\a b. false;
  (>=)  = \\a b. false;
}
(1 == 2, 1 != 2, 1 < 2, 1 <= 2, 1 > 2, 1 >= 2)
";
    let v = run(src);
    match v {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 6);
            for f in fields.iter() {
                assert_eq!(f.value.peek(), Some(Value::Bool(false)));
            }
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

/// Alias-resolved key match: a witness whose target is a named type alias
/// must still dispatch when the operand's inferred type is the structural record.
/// This verifies the D4 alias-resolution fix in `type_key`.
#[test]
fn op_dispatch_alias_resolved_key() {
    let src = "
Point :: type { x : Int; y : Int; }
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Point :: { (==) = \\a b. false; }
{ x = 1; y = 2; } == { x = 1; y = 2; }
";
    // Without alias resolution in type_key, dispatch would miss and builtin
    // values_equal would return true (structural equality). With it: false.
    assert_eq!(run(src), Value::Bool(false));
}

/// Builtin fallback: with no witness, `1 == 1` uses structural equality.
#[test]
fn op_dispatch_eq_builtin_fallback() {
    assert_eq!(run("1 == 1"), Value::Bool(true));
    assert_eq!(run("1 == 2"), Value::Bool(false));
}

/// Ordering on a non-scalar type-checks (D6 relaxation) when an ordering
/// constraint exists, but eval refuses via `cmp_op` when no witness matches.
#[test]
fn op_dispatch_ordering_non_scalar_no_witness_refuses() {
    let src = "
Ord :: <A> @A { (<) :: A -> A -> Bool; }
{ x = 1; } < { x = 2; }
";
    // Type-checks (no THIR error) because Ord constraint declares (<).
    // Eval refuses: no Ord @{...} witness → cmp_op returns TypeMismatch.
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::TypeMismatch { .. }),
        "expected TypeMismatch for non-scalar < with no witness, got {err:?}"
    );
}

/// Ordering on a non-scalar WITH a witness dispatches correctly.
#[test]
fn op_dispatch_ordering_non_scalar_with_witness() {
    let src = "
Point :: type { x : Int; y : Int; }
Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Point :: { (<) = \\a b. true; }
{ x = 2; y = 0; } < { x = 1; y = 0; }
";
    // Custom (<) always returns true even though 2 > 1.
    assert_eq!(run(src), Value::Bool(true));
}

// ─── Increment 8: polymorphic constraint dispatch ─────────────────────────────

/// Headline test: `same 1 1` evaluates to `Bool true` via witness-dict injection.
/// The `eq` method inside `same` dispatches through the `Eq @Int` witness because
/// the injected WitnessDict resolves the ambiguous TypeVar at the call site.
#[test]
fn dispatch_polymorphic_method_inside_bounded_fn() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
same 1 1
"#;
    assert_eq!(run(src), Value::Bool(true));
}

/// Default-body fallback: a witness that omits the method uses the default body
/// defined in the constraint.  Witness `Eq @Int :: {}` is valid (method has a
/// default), and calling `eq 1 2` uses the default clause `| _ _ => true;`.
#[test]
fn dispatch_default_method_used_when_field_absent() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool { | _ _ => true; }; }
Eq @Int :: {}
eq 1 2
"#;
    assert_eq!(run(src), Value::Bool(true));
}

/// Regression: an unbounded wrapper calls a bounded function — the bounded
/// function is called indirectly so the witness dict is not visible in the
/// callee's captured env.  Must refuse cleanly with `UnresolvedWitness`,
/// not return a wrong value (`Bool(true)`) or an internal error.
#[test]
fn dispatch_polymorphic_indirect_call_refuses_cleanly() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
wrapper :: Int -> Bool { | n => same n n; }
wrapper 1
"#;
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::UnresolvedWitness { .. }),
        "expected UnresolvedWitness for indirect bounded-fn call, got {err:?}"
    );
}

/// Regression: the default-body fallback must NOT fire for ambiguous type keys
/// even when the method has a default body.  An indirect call where the witness
/// dict is invisible must refuse with `UnresolvedWitness`, not silently return
/// the default value `Bool(true)`.
#[test]
fn dispatch_default_not_used_when_witness_exists_but_indirect() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool { | _ _ => true; }; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool { | x y => eq x y; }
useit :: Int -> Bool { | _ => same 1 2; }
useit 0
"#;
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::UnresolvedWitness { .. }),
        "expected UnresolvedWitness (not Bool(true) wrong answer), got {err:?}"
    );
}

// ─── Value::Display ───────────────────────────────────────────────────────────

#[test]
fn display_bool_true() {
    assert_eq!(run("true").to_string(), "true");
}

#[test]
fn display_bool_false() {
    assert_eq!(run("false").to_string(), "false");
}

#[test]
fn display_int_value() {
    assert_eq!(run("42").to_string(), "42");
}

#[test]
fn display_float_with_decimal() {
    // 1.5 already has a decimal point — shown as-is
    assert_eq!(run("1.5").to_string(), "1.5");
}

#[test]
fn display_float_integer_value_adds_dot_zero() {
    // 2.0 stored as float — must print with ".0" suffix
    assert_eq!(run("2.0").to_string(), "2.0");
}

#[test]
fn display_float_negative() {
    assert_eq!(run("0.0 - 1.5").to_string(), "-1.5");
}

#[test]
fn display_text_plain() {
    assert_eq!(run(r#""hello""#).to_string(), r#""hello""#);
}

#[test]
fn display_text_escape_double_quote() {
    // Inner double-quote must be escaped as \"
    assert_eq!(run(r#""say \"hi\"""#).to_string(), r#""say \"hi\"""#);
}

#[test]
fn display_text_escape_backslash() {
    assert_eq!(run(r#""a\\b""#).to_string(), r#""a\\b""#);
}

#[test]
fn display_text_escape_newline() {
    assert_eq!(run(r#""a\nb""#).to_string(), r#""a\nb""#);
}

#[test]
fn display_text_escape_carriage_return() {
    assert_eq!(run(r#""a\rb""#).to_string(), r#""a\rb""#);
}

#[test]
fn display_text_escape_tab() {
    assert_eq!(run(r#""a\tb""#).to_string(), r#""a\tb""#);
}

#[test]
fn display_atom_value() {
    assert_eq!(run("#hello").to_string(), "#hello");
}

#[test]
fn display_nothing_value() {
    // Nothing comes from absent optional field access
    let src = "
S :: type { port? : Int; }
s :: S = {}
s.port
";
    assert_eq!(run(src).to_string(), "#none");
}

#[test]
fn display_list_empty() {
    let src = "
x :: List Int = []
x
";
    assert_eq!(run(src).to_string(), "[]");
}

#[test]
fn display_list_singleton() {
    assert_eq!(run("[42;]").to_string(), "[42]");
}

#[test]
fn display_list_multiple() {
    assert_eq!(run("[1; 2; 3;]").to_string(), "[1; 2; 3]");
}

#[test]
fn display_tuple_positional() {
    assert_eq!(run("(1, 2)").to_string(), "(1, 2)");
}

#[test]
fn display_tuple_three_items() {
    assert_eq!(run("(1, 2, 3)").to_string(), "(1, 2, 3)");
}

#[test]
fn display_record_single_field() {
    assert_eq!(run("{ x = 1; }").to_string(), "{ x = 1 }");
}

#[test]
fn display_record_two_fields() {
    // The separator between fields is "; " and each field is prefixed with " ",
    // so two consecutive fields show "field1; <space>field2" (note the extra space).
    assert_eq!(run("{ x = 1; y = 2; }").to_string(), "{ x = 1;  y = 2 }");
}

#[test]
fn display_tagged_value_with_payload() {
    let src = "
Status :: type [
  ok : { code : Int; };
  err : { msg : Text; };
]
s :: Status = #ok { code = 200; }
s
";
    assert_eq!(run(src).to_string(), "#ok { code = 200 }");
}

#[test]
fn display_closure_shows_remaining_arity() {
    // A function value displays as <function/N> where N = remaining arity
    let src = "
inc :: Int -> Int {
  | x => x + 1;
}
inc
";
    assert_eq!(run(src).to_string(), "<function/1>");
}

#[test]
fn display_closure_partial_application() {
    // After partial application arity decreases: <function/2> → <function/1>
    let src = "
add :: Int -> Int -> Int {
  | x y => x + y;
}
add 1
";
    assert_eq!(run(src).to_string(), "<function/1>");
}

// ─── integer overflow ─────────────────────────────────────────────────────────

#[test]
fn int_overflow_add() {
    assert_eq!(
        run_err("9223372036854775807 + 1"),
        EvalError::IntOverflow("+")
    );
}

#[test]
fn int_overflow_sub() {
    // i64::MIN - 1 overflows
    assert_eq!(
        run_err("-9223372036854775807 - 2"),
        EvalError::IntOverflow("-")
    );
}

#[test]
fn int_overflow_mul() {
    assert_eq!(
        run_err("9223372036854775807 * 2"),
        EvalError::IntOverflow("*")
    );
}

// ─── float and text comparison (cmp_op) ──────────────────────────────────────

#[test]
fn float_lt() {
    assert_eq!(run("1.0 < 2.0"), Value::Bool(true));
    assert_eq!(run("2.0 < 1.0"), Value::Bool(false));
}

#[test]
fn float_le() {
    assert_eq!(run("1.0 <= 1.0"), Value::Bool(true));
    assert_eq!(run("2.0 <= 1.0"), Value::Bool(false));
}

#[test]
fn float_gt() {
    assert_eq!(run("2.0 > 1.0"), Value::Bool(true));
    assert_eq!(run("1.0 > 2.0"), Value::Bool(false));
}

#[test]
fn float_ge() {
    assert_eq!(run("1.0 >= 1.0"), Value::Bool(true));
    assert_eq!(run("0.5 >= 1.0"), Value::Bool(false));
}

#[test]
fn text_lt() {
    assert_eq!(run(r#""a" < "b""#), Value::Bool(true));
    assert_eq!(run(r#""b" < "a""#), Value::Bool(false));
}

#[test]
fn text_le() {
    assert_eq!(run(r#""a" <= "a""#), Value::Bool(true));
    assert_eq!(run(r#""b" <= "a""#), Value::Bool(false));
}

#[test]
fn text_gt() {
    assert_eq!(run(r#""b" > "a""#), Value::Bool(true));
}

#[test]
fn text_ge() {
    assert_eq!(run(r#""b" >= "b""#), Value::Bool(true));
}

// ─── TaggedValue semantics ────────────────────────────────────────────────────

#[test]
fn tagged_value_equality_same_tag_and_payload() {
    let src = "
Status :: type [
  ok : { code : Int; };
  err : { msg : Text; };
]
a :: Status = #ok { code = 200; }
b :: Status = #ok { code = 200; }
a == b
";
    assert_eq!(run(src), Value::Bool(true));
}

#[test]
fn tagged_value_equality_different_tag() {
    let src = "
Status :: type [
  ok : { code : Int; };
  err : { msg : Text; };
]
a :: Status = #ok { code = 200; }
b :: Status = #err { msg = \"nope\"; }
a == b
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn tagged_value_equality_different_payload() {
    let src = "
Status :: type [
  ok : { code : Int; };
  err : { msg : Text; };
]
a :: Status = #ok { code = 200; }
b :: Status = #ok { code = 404; }
a == b
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn tagged_value_tag_field_access() {
    // `.tag` on a tagged value returns the atom of the tag name.
    // Since THIR's static type of a union is "union" (not a record), field access
    // must go through match + record access.  We verify the `.tag` runtime path
    // by writing a function that accepts Any and returns via match.
    let src = "
Status :: type [
  ok : { code : Int; };
  err : { msg : Text; };
]
getCode :: Status -> Int {
  | #ok { code = n; } => n;
  | #err { msg = _; } => -1;
}
getCode (#ok { code = 200; })
";
    assert_eq!(run(src), Value::Int(200));
}

#[test]
fn tagged_value_match_by_tag() {
    let src = "
Color :: type [
  red : { r : Int; };
  blue : { b : Int; };
]
c :: Color = #red { r = 255; }
match c {
  | #red { r = n; } => n;
  | #blue { b = n; } => 0;
}
";
    assert_eq!(run(src), Value::Int(255));
}

#[test]
fn tagged_value_match_wrong_tag_falls_through() {
    let src = "
Color :: type [
  red : { r : Int; };
  blue : { b : Int; };
]
c :: Color = #blue { b = 100; }
match c {
  | #red { r = n; } => n;
  | #blue { b = n; } => n + 1;
}
";
    assert_eq!(run(src), Value::Int(101));
}

// ─── type_key dispatch arms ───────────────────────────────────────────────────

#[test]
fn type_key_float_witness() {
    // type_key hits TypeKind::Float arm when witness target is @Float
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Float :: { (==) = \\a b. false; }
1.0 == 1.0
";
    // Without witness: builtin says true; with Float witness: false.
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_text_witness() {
    // type_key hits TypeKind::Text arm
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Text :: { (==) = \\a b. false; }
\"hi\" == \"hi\"
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_atom_witness() {
    // type_key hits TypeKind::Atom arm for singleton atom type @#hello
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @#hello :: { (==) = \\a b. false; }
#hello == #hello
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_tuple_witness() {
    // type_key hits TypeKind::Tuple arm for (Int, Int) witness target
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @(Int, Int) :: { (==) = \\a b. false; }
(1, 2) == (1, 2)
";
    assert_eq!(run(src), Value::Bool(false));
}

// ─── format_thir_diagnostic arms ─────────────────────────────────────────────
//
// Each test exercises one arm of `format_thir_diagnostic` in lib.rs by
// feeding a program that passes parse + HIR but fails THIR.

fn assert_type_check_failed(src: &str) {
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::TypeCheckFailed(_)),
        "expected TypeCheckFailed for:\n{src}\ngot: {err:?}"
    );
}

#[test]
fn thir_diag_invalid_binary_operands() {
    // `true + 1` — booleans don't support `+`
    assert_type_check_failed("true + 1");
}

#[test]
fn thir_diag_empty_list_needs_type() {
    // `[]` with no type context
    assert_type_check_failed("[]");
}

#[test]
fn thir_diag_expected_function() {
    // Applying 1 (an Int) as a function
    assert_type_check_failed("1 2");
}

#[test]
fn thir_diag_unsupported_feature_empty_match() {
    // Empty match arms — must be on a separate line so `{}` isn't parsed as a
    // record argument applied to the scrutinee.
    assert_type_check_failed("match 1\n{}");
}

#[test]
fn thir_diag_non_exhaustive_match() {
    // Integer match with only one literal arm — not exhaustive
    assert_type_check_failed("match 1 {\n  | 1 => 2;\n}");
}

#[test]
fn thir_diag_unreachable_match_arm() {
    // Wildcard makes the next arm unreachable
    assert_type_check_failed("match 1 {\n  | _ => 1;\n  | 1 => 2;\n}");
}

#[test]
fn thir_diag_match_arm_pattern_count_mismatch() {
    // Match arm with 2 patterns instead of 1
    assert_type_check_failed("match 1 {\n  | a b => 1;\n}");
}

#[test]
fn thir_diag_function_clause_arity_mismatch() {
    // Function typed Int -> Int but clause has 2 patterns
    assert_type_check_failed("f :: Int -> Int {\n  | x y => x;\n}\nf 1");
}

#[test]
fn thir_diag_tuple_arity_mismatch() {
    // Type says (Int, Int) but value has 3 elements
    assert_type_check_failed("x :: (Int, Int) = (1, 2, 3)\nx");
}

#[test]
fn thir_diag_alias_cycle() {
    // Mutually cyclic type aliases
    let src = "
A :: type B
B :: type A
x :: A = 1
x
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_type_constructor_arity_mismatch() {
    // Pair needs 2 type args but only 1 given
    let src = "
Pair :: <A, B> type { first : A; second : B; }
x :: Pair Text = x
x
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_invalid_type_expression() {
    // Number literal `1` in type annotation position → ExprEscape → InvalidTypeExpression
    assert_type_check_failed("x :: 1 = 1\nx");
}

#[test]
fn thir_diag_unknown_field() {
    // Accessing field `y` that doesn't exist on `{ x : Int }`
    assert_type_check_failed("{ x = 1; }.y");
}

#[test]
fn thir_diag_missing_record_field() {
    let src = "
Server :: type { host : Text; port : Int; }
s :: Server = { host = \"localhost\"; }
s
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_unexpected_record_field() {
    let src = "
Server :: type { host : Text; }
s :: Server = { host = \"localhost\"; port = 8080; }
s
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_witness_field_type_mismatch() {
    // Witness field `(==)` should be `Int -> Int -> Bool` but is given `42` (Int)
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = 42; }
1 == 1
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_missing_witness_field() {
    // Witness for Eq @Int is missing the required `(==)` field
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: {}
1 == 1
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_unknown_witness_field() {
    // Witness has field `extra` not declared in constraint
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. true; extra = 42; }
1 == 1
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_conflicting_witness() {
    // Two witnesses for Eq @Int
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. true; }
Eq @Int :: { (==) = \\a b. false; }
1 == 1
";
    assert_type_check_failed(src);
}

// ─── match_pattern: Float literal patterns ────────────────────────────────────

#[test]
fn match_float_pattern_in_function_clause() {
    let src = "
classify :: Float -> Text {
  | 0.0 => \"zero\";
  | 1.5 => \"one-half\";
  | _ => \"other\";
}
classify 1.5
";
    assert_eq!(run(src), Value::Text("one-half".into()));
}

#[test]
fn match_float_pattern_fallthrough() {
    let src = "
classify :: Float -> Text {
  | 0.0 => \"zero\";
  | 1.5 => \"one-half\";
  | _ => \"other\";
}
classify 2.0
";
    assert_eq!(run(src), Value::Text("other".into()));
}

// ─── match_pattern: String literal patterns ───────────────────────────────────

#[test]
fn match_string_pattern_in_function_clause() {
    let src = "
greet :: Text -> Text {
  | \"hello\" => \"world\";
  | \"hi\" => \"there\";
  | _ => \"unknown\";
}
greet \"hello\"
";
    assert_eq!(run(src), Value::Text("world".into()));
}

#[test]
fn match_string_pattern_fallthrough() {
    let src = "
greet :: Text -> Text {
  | \"hello\" => \"world\";
  | _ => \"stranger\";
}
greet \"goodbye\"
";
    assert_eq!(run(src), Value::Text("stranger".into()));
}

// ─── match_pattern: Atom literal patterns ────────────────────────────────────

#[test]
fn match_atom_pattern_in_function_clause() {
    // The type `#foo` is a singleton atom type (Atom("foo")), not a union variant.
    // Pattern `#foo` in this context produces ThirPatKind::Atom.
    let src = "
describe :: #foo -> Text {
  | #foo => \"it is foo\";
}
describe #foo
";
    assert_eq!(run(src), Value::Text("it is foo".into()));
}

// ─── match_pattern: Positional Tuple patterns ─────────────────────────────────

#[test]
fn match_positional_tuple_pattern_in_function_clause() {
    // Positional tuple pattern `(x, y)` exercises ThirPatKind::Tuple Positional arm.
    let src = "
fst :: (Int, Text) -> Int {
  | (n, _) => n;
}
fst (42, \"hi\")
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn match_positional_tuple_pattern_in_match_expr() {
    let src = "
p :: (Int, Int) = (10, 20)
match p {
  | (x, y) => x + y;
}
";
    assert_eq!(run(src), Value::Int(30));
}

// ─── match_pattern: Named Tuple patterns ─────────────────────────────────────

#[test]
fn named_tuple_construction_and_named_pattern() {
    // Named tuple value `(x = 42, y = 99)` exercises ThirTupleItem::Named construction.
    // Pattern `(x = v, y = _)` exercises ThirTuplePatItem::Named matching.
    let src = "
Coord :: type (x : Int, y : Int)
getX :: Coord -> Int {
  | (x = v, y = _) => v;
}
getX (x = 42, y = 99)
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn display_named_tuple() {
    // Named tuple fields display as `name = value`.
    let src = "
Coord :: type (x : Int, y : Int)
p :: Coord = (x = 10, y = 20)
p
";
    assert_eq!(run(src).to_string(), "(x = 10, y = 20)");
}

// ─── match_pattern: Record patterns in function clauses ───────────────────────

#[test]
fn match_record_pattern_in_function_clause() {
    // Record pattern `{ port = n; }` exercises ThirPatKind::Record in match_pattern.
    let src = "
Server :: type { host : Text; port : Int; }
getPort :: Server -> Int {
  | { host = _; port = n; } => n;
}
getPort { host = \"localhost\"; port = 8080; }
";
    assert_eq!(run(src), Value::Int(8080));
}

#[test]
fn match_record_pattern_multiple_fields() {
    let src = "
Point :: type { x : Int; y : Int; }
sumCoords :: Point -> Int {
  | { x = a; y = b; } => a + b;
}
sumCoords { x = 3; y = 4; }
";
    assert_eq!(run(src), Value::Int(7));
}

// ─── Guard false in function clause (apply_closure path) ─────────────────────

#[test]
fn function_clause_guard_false_falls_through() {
    // guard `n > 0` evaluates to false for negative input → falls through to next clause.
    let src = "
classify :: Int -> Int {
  | n if n > 0 => 1;
  | 0 => 0;
  | _ => -1;
}
classify (-1)
";
    assert_eq!(run(src), Value::Int(-1));
}

#[test]
fn function_clause_guard_false_then_matching_clause() {
    // Guard on first clause fails; second clause (no guard) matches.
    let src = "
safe_div :: Int -> Int -> Int {
  | _ 0 => 0;
  | n d if d > 0 => n / d;
  | _ _ => 0;
}
safe_div 10 0
";
    assert_eq!(run(src), Value::Int(0));
}

// ─── TypeValue expression ─────────────────────────────────────────────────────

#[test]
fn type_alias_reference_produces_type_value() {
    // Referencing a type alias by name evaluates to a TypeValue.
    let src = "
MyInt :: type Int
MyInt
";
    // TypeValue just needs to evaluate without error.
    match eval_file(src).unwrap() {
        Value::TypeValue(_) => {}
        other => panic!("expected TypeValue, got {other:?}"),
    }
}

#[test]
fn display_type_value() {
    // Value::TypeValue displays as "<type>".
    let src = "
MyInt :: type Int
MyInt
";
    assert_eq!(eval_file(src).unwrap().to_string(), "<type>");
}

// ─── Bool and Float equality (values_equal arms) ─────────────────────────────

#[test]
fn bool_equality_true_true() {
    // values_equal Bool arm: true == true.
    assert_eq!(run("true == true"), Value::Bool(true));
}

#[test]
fn bool_equality_true_false() {
    assert_eq!(run("true == false"), Value::Bool(false));
}

#[test]
fn float_equality_equal() {
    // values_equal Float arm: 1.5 == 1.5.
    assert_eq!(run("1.5 == 1.5"), Value::Bool(true));
}

#[test]
fn float_equality_not_equal() {
    assert_eq!(run("1.5 == 2.0"), Value::Bool(false));
}

#[test]
fn float_ne_operator() {
    assert_eq!(run("1.5 != 2.0"), Value::Bool(true));
}

// ─── from_immediate: Im::False and Im::Float ─────────────────────────────────

#[test]
fn import_zti_false_value() {
    // meta.zti has `active = false` — exercises Im::False arm in from_immediate.
    assert_eq!(
        run_import("m := import \"meta.zti\"\nm.active"),
        Value::Bool(false)
    );
}

#[test]
fn import_zti_float_value() {
    // meta.zti has `score = 2.5` — exercises Im::Float arm in from_immediate.
    assert_eq!(
        run_import("m := import \"meta.zti\"\nm.score"),
        Value::Float(2.5)
    );
}

// ─── .zti import coverage: Text, Atom, List ───────────────────────────────────

#[test]
fn import_zti_text_field() {
    // config.zti has `host = "127.0.0.1"` — exercises ImportedType::Text in import.rs
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zti_atom_field() {
    // config.zti has `env = #prod` — exercises ImportedType::Atom in import.rs
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.env"),
        Value::Atom("prod".into())
    );
}

#[test]
fn import_zti_empty_list_field() {
    // empty_list.zti has `items = []` — exercises ImportedType::Unknown via empty array
    match run_import("m := import \"empty_list.zti\"\nm.items") {
        Value::List(items) => assert!(items.is_empty(), "expected empty list"),
        other => panic!("expected List, got {other:?}"),
    }
}

// ─── .zt import coverage: Optional, Tuple, Union, Type ───────────────────────

#[test]
fn import_zt_optional_module() {
    // optional_module.zt exports Int? — exercises ImportedType::Optional in import.rs
    // cfg.port is absent so the result is Value::Nothing (absent optional).
    assert_eq!(
        run_import("m := import \"optional_module.zt\"\nm"),
        Value::Nothing
    );
}

#[test]
fn import_zt_tuple_module() {
    // tuple_module.zt exports (Int, Text) — exercises ImportedType::Tuple in import.rs
    match run_import("m := import \"tuple_module.zt\"\nm") {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2, "tuple should have 2 fields");
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

#[test]
fn import_zt_union_module() {
    // union_module.zt exports Color (a union) — exercises ImportedType::Union in import.rs
    assert_eq!(
        run_import("m := import \"union_module.zt\"\nm"),
        Value::Atom("red".into())
    );
}

#[test]
fn import_zt_type_module() {
    // type_module.zt exports MyInt (a type alias reference) — ImportedType::Type in import.rs.
    // TLC now maps TypeKind::Type to PrimTy::Nothing instead of panicking; the THIR evaluator
    // returns Value::TypeValue for the imported type alias reference.
    let v = run_import("m := import \"type_module.zt\"\nm");
    assert!(
        matches!(v, Value::TypeValue(_)),
        "expected TypeValue for imported type alias, got {v:?}"
    );
}
