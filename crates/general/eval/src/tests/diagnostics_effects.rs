use super::*;

// ─── format_thir_diagnostic arms ─────────────────────────────────────────────
//
// Each test exercises one arm of `format_thir_diagnostic` in lib.rs by
// feeding a program that passes parse + HIR but fails THIR.

fn type_check_messages(src: &str) -> Vec<String> {
    let err = run_err(src);
    let EvalError::TypeCheckFailed(msgs) = err else {
        panic!("expected TypeCheckFailed for:\n{src}\ngot: {err:?}");
    };
    msgs
}

fn assert_type_check_failed(src: &str) {
    let _ = type_check_messages(src);
}

#[test]
fn diagnostic_polish_eval_record_mismatch_message() {
    let src = r#"
S :: type { x : Int; y : Text; }
T :: type { x : Int; }
f :: S -> Int
  = _ => 0;
t :: T = { x = 1; }
f t
"#;
    let msgs = type_check_messages(src);
    assert!(
        msgs.iter()
            .any(|m| { m == "type mismatch: expected { x : Int; y : Text; }, found { x : Int; }" })
    );
}

#[test]
fn diagnostic_polish_eval_row_tail_overlap_message() {
    let src = r#"
Base :: type { host : Text; port : Int; }
Bad :: type { host : Int; ...Base; }
Bad
"#;
    let msgs = type_check_messages(src);
    assert!(msgs.iter().any(|m| {
        m == "record row tail `...Base` overlaps explicit field `host`: existing `host : Int`, incoming `host : Text`"
    }));
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
    assert_type_check_failed("f :: Int -> Int\n  = x y => x;\nf 1");
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
// ─── algebraic effects ────────────────────────────────────────────────────────

#[test]
fn handled_warn_resume_runs_rest_of_computation() {
    assert_eq!(
        run(r#"
result ::= handle { perform warn "diag"; "ok" } with { warn = \d. resume (); }
result
"#,),
        Value::Text("ok".into())
    );
}

#[test]
fn handled_effect_in_block_local_initializer_resumes() {
    assert_eq!(
        run(r#"
compute :: Text -> Int ! { query : Text -> Int }
  = _ => { x := perform query "q"; x + 1 };
handle compute "go" with { query = \u. resume 41; }
"#,),
        Value::Int(42)
    );
}

#[test]
fn repeated_handled_effects_resume_with_distinct_handler_bindings() {
    assert_eq!(
        run(r#"
result ::= handle (perform query 1) + (perform query 2) with { query = \n. resume n; }
result
"#,),
        Value::Int(3)
    );
}

#[test]
fn handled_effect_resume_value_can_reference_param_inside_tuple() {
    assert_eq!(
        run(r#"
result ::= handle perform query 1 with { query = \n. resume (n, n); }
result
"#,)
        .to_string(),
        "(1, 1)"
    );
}
#[test]
fn handled_fail_can_return_without_resuming() {
    assert_eq!(
        run(r#"
result ::= handle { perform fail "bad"; "unreachable" } with { fail = \e. "fallback"; }
result
"#,),
        Value::Text("fallback".into())
    );
}

#[test]
fn top_level_io_print_is_handled_by_host_boundary() {
    assert_eq!(
        run(r#"perform io.print "hello""#),
        Value::Text("hello".into())
    );
}

#[test]
fn source_handler_intercepts_repointed_print_builtin() {
    assert_eq!(
        run(r#"
result ::= handle print "x" with { io.print = \text. "handled"; }
result
"#,),
        Value::Text("handled".into())
    );
}

#[test]
fn non_tail_resume_reenters_suspended_expression() {
    assert_eq!(
        run(r#"
compute :: Text -> Int ! { query : Text -> Int }
  = _ => (perform query "question") + 1;
result ::= handle compute "go" with { query = \u. resume 41; }
result
"#,),
        Value::Int(42)
    );
}

#[test]
fn forwarded_effect_reaches_outer_handler() {
    assert_eq!(
        run(r#"
result ::= handle (handle { perform fail "bad"; "unreachable" } with { fail = \e. { perform log e; "fallback" }; }) with { log = \msg. resume (); }
result
"#,),
        Value::Text("fallback".into())
    );
}

#[test]
fn value_clause_runs_only_on_normal_completion() {
    assert_eq!(
        run(r#"
normal ::= handle "ok" with { value = \v. "done"; }
normal
"#,),
        Value::Text("done".into())
    );
    assert_eq!(
        run(r#"
abort ::= handle perform fail "bad" with { value = \v. "done"; fail = \e. "fallback"; }
abort
"#,),
        Value::Text("fallback".into())
    );
}

// ─── prelude `print` effect binding ───────────────────────────────────────────

#[test]
fn print_returns_its_argument() {
    // `print :: Text -> Text ! { io.print : Text -> Text }`; the host run
    // boundary handles `io.print` and resumes with the printed text.
    assert_eq!(run(r#"print "hello""#), Value::Text("hello".into()));
}

#[test]
fn print_via_forward_pipeline() {
    // `"x" |> print` desugars to `print "x"`.
    assert_eq!(run(r#""piped" |> print"#), Value::Text("piped".into()));
}

#[test]
fn print_in_list_returns_all_elements() {
    // force_deep forces every element, so each `print` fires and the list value
    // is the list of returned texts.
    match run(r#"[print "a"; print "b"; print "c";]"#) {
        Value::List(items) => {
            let texts: Vec<_> = items.iter().filter_map(|t| t.peek()).collect();
            assert_eq!(
                texts,
                vec![
                    Value::Text("a".into()),
                    Value::Text("b".into()),
                    Value::Text("c".into()),
                ]
            );
        }
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn print_result_is_usable_text() {
    // The returned text composes with ordinary text operations.
    assert_eq!(run(r#"print "z" < "a""#), Value::Bool(false));
}

#[test]
fn print_lambda_param_shadows_builtin() {
    // A lambda parameter named `print` shadows the prelude builtin; applying it
    // is ordinary variable application, not the builtin effect.
    let src = "apply :: (Text -> Text) -> Text -> Text\n  = f x => f x;\napply (\\print. print) \"shadowed\"";
    assert_eq!(run(src), Value::Text("shadowed".into()));
}

#[test]
fn print_unapplied_is_a_function_value() {
    // Referencing `print` without applying it yields the runtime-dispatching
    // function value used by both direct and higher-order calls.
    assert!(matches!(run("print"), Value::TlcClosure(_)));
}

#[test]
fn redefining_print_at_top_level_is_rejected() {
    // `print` is reserved in the root scope, so a top-level redefinition is a
    // DuplicateBinding error and the program refuses to run.
    let err = run_err("print ::= 5\nprint");
    assert!(matches!(err, EvalError::NotRunnable(_)), "got {err:?}");
}

#[test]
fn occurs_check_rejects_infinite_self_application() {
    // `(\x. x x) (\x. x x)` requires an infinite type. The occurs check must
    // reject it at THIR (clean diagnostic) rather than silently accepting and
    // overflowing the stack at runtime.
    let msgs = type_check_messages("(\\x. x x) (\\x. x x)");
    assert!(
        msgs.iter().any(|m| m.contains("infinite type")),
        "expected an infinite-type diagnostic, got: {msgs:?}"
    );
}
