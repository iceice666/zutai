use super::*;

// ─── arithmetic ───────────────────────────────────────────────────────────────

#[test]
fn int_add() {
    assert_eq!(run("1 + 2"), Value::Int(3));
}

#[test]
fn typed_code_direct_splice_runs() {
    assert_eq!(run("splice(quote(1 + 2))"), Value::Int(3));
}

#[test]
fn typed_code_bound_splice_runs() {
    assert_eq!(run("c ::= quote(40 + 2); splice(c)"), Value::Int(42));
}

#[test]
fn typed_code_pure_helper_expands_with_lexical_substitution() {
    let src = r#"
make :: Int -> Code Int = value => quote(value + 1);
value ::= 100;
splice(make 41)
"#;
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn typed_code_nested_splice_retains_original_bindings() {
    let src = r#"
wrap :: Code Int -> Code Int = code => quote(splice(code) + 1);
value ::= 40;
splice(wrap(quote(value + 1)))
"#;
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn typed_code_compile_time_conditional_is_reduced() {
    let src = r#"
choose :: Bool -> Code Int = yes => if yes then quote(42) else quote(0);
splice(choose true)
"#;
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn typed_code_result_is_rejected() {
    let err = run_err("quote(1)");
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Code value"))
    );
}

#[test]
fn quoted_generic_recipe_supports_arbitrary_method_names() {
    let src = r#"
Const :: <A> @A { constant :: A -> Int; } derive = <T> => quote({ constant = \value. 7; })
Const @Text :: derive
constant "ignored"
"#;
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn quoted_generic_recipe_executes_pure_code_helper() {
    let src = r#"
makeConst :: Unit -> Code { constant : Text -> Int; }
  = _ => quote({ constant = \value. 9; });
Const :: <A> @A { constant :: A -> Int; } derive = <T> => makeConst ()
Const @Text :: derive
constant "ignored"
"#;
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn quoted_generic_recipe_result_is_checked_at_derive_request() {
    let src = r#"
makeConst :: Unit -> Code { constant : Text -> Int; }
  = _ => quote({ constant = \value. 9; });
Const :: <A> @A { constant :: A -> Int; } derive = <T> => makeConst ()
Const @Int :: derive
constant 1
"#;
    let EvalError::TypeCheckFailed(messages) = run_err(src) else {
        panic!("expected type-check failure");
    };
    assert!(
        messages
            .iter()
            .any(|message| { message.contains("derive recipe") && message.contains("constant") })
    );
}

#[test]
fn quoted_recipe_reduces_pattern_match_on_config() {
    // A recipe that inspects a compile-time config via `match` must reduce the
    // pattern to select the matching arm and bind the field into the expansion.
    let src = r#"
pick :: { constant : Int; } -> Code { constant : Text -> Int; }
  = cfg => match cfg { | { constant = n; } => quote({ constant = \value. n; }); };
Const :: <A> @A { constant :: A -> Int; } derive = <T> => pick { constant = 5; }
Const @Text :: derive
constant "ignored"
"#;
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn quoted_recipe_fuel_exhaustion_is_reported() {
    // A recipe whose pure reducer never terminates must surface as a source
    // diagnostic and refuse evaluation, not silently degrade to a bad witness.
    let src = r#"
loop :: Int -> Code { constant : Text -> Int; }
  = n => loop (n + 1);
Const :: <A> @A { constant :: A -> Int; } derive = <T> => loop 0
Const @Text :: derive
constant "ignored"
"#;
    let EvalError::TypeCheckFailed(messages) = run_err(src) else {
        panic!("expected type-check failure");
    };
    assert!(
        messages
            .iter()
            .any(|message| message.contains("exhausted type-level fuel")),
        "expected fuel-exhaustion diagnostic, got {messages:?}"
    );
}

#[test]
fn quoted_recipe_reduces_structural_recursion() {
    // A recipe that recurses over a nullary/payload union at compile time must
    // reduce through the recursion, selecting the terminating arm. This exercises
    // decisive cross-constructor arm ordering: arm 1 tests a nullary `#zero`
    // (atom) pattern against a `#succ` (payload) value — a decisive non-match that
    // must fall through to arm 2 rather than stalling the whole reduction.
    let src = r#"
Nat :: type { #zero; #succ : { n : Nat; }; };
pick :: Nat -> Code { constant : Text -> Int; }
  = cfg => match cfg { | #zero => quote({ constant = \value. 0; }); | #succ { n = m; } => pick m; };
Const :: <A> @A { constant :: A -> Int; } derive = <T> => pick (#succ { n = #succ { n = #zero; }; })
Const @Text :: derive
constant "ignored"
"#;
    assert_eq!(run(src), Value::Int(0));
}

#[test]
fn quoted_recipe_selects_first_arm_over_payload_variant() {
    // The dual arm-ordering case: a `#succ` value tested against the nullary
    // `#zero` arm first, then matched by the `#succ` arm. The `#succ` arm here is
    // terminal (no recursion), so the witness comes from arm 2.
    let src = r#"
Nat :: type { #zero; #succ : { n : Nat; }; };
pick :: Nat -> Code { constant : Text -> Int; }
  = cfg => match cfg { | #zero => quote({ constant = \value. 0; }); | #succ { n = m; } => quote({ constant = \value. 1; }); };
Const :: <A> @A { constant :: A -> Int; } derive = <T> => pick (#succ { n = #zero; })
Const @Text :: derive
constant "ignored"
"#;
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn quoted_recipe_irreducible_body_is_refused() {
    // A `Code`-typed recipe whose body stalls on a comparison the pure
    // compile-time reducer does not evaluate (`n == 0`) must be refused, not
    // silently fall through to an empty witness that crashes at dispatch. A
    // refused evaluation beats a wrong (missing) witness.
    let src = r#"
pick :: Int -> Code { constant : Text -> Int; }
  = n => if n == 0 then quote({ constant = \value. 0; }) else quote({ constant = \value. 1; });
Const :: <A> @A { constant :: A -> Int; } derive = <T> => pick 0
Const @Text :: derive
constant "ignored"
"#;
    let EvalError::TypeCheckFailed(messages) = run_err(src) else {
        panic!("expected type-check failure");
    };
    assert!(
        messages
            .iter()
            .any(|message| message.contains("did not reduce to a witness record")),
        "expected irreducible-recipe diagnostic, got {messages:?}"
    );
}

#[test]
fn quoted_recipe_with_effect_body_is_refused() {
    // A recipe body is a pure `Code`-returning computation. Performing an effect
    // inside it escapes the (empty) ambient effect row and must be refused by the
    // type system before any witness is synthesized.
    let src = r#"
pick :: Int -> Code { constant : Text -> Int; }
  = n => perform io.print "hi";
Const :: <A> @A { constant :: A -> Int; } derive = <T> => pick 0
Const @Text :: derive
constant "ignored"
"#;
    let EvalError::TypeCheckFailed(messages) = run_err(src) else {
        panic!("expected type-check failure");
    };
    assert!(
        messages
            .iter()
            .any(|message| message.contains("io.print") && message.contains("effect row")),
        "expected effect-escape diagnostic, got {messages:?}"
    );
}

#[test]
fn derived_from_data_decodes_primitive() {
    let src = r#"
result :: Validation DecodeIssue Int = decode (#int { value = 42; });
result
"#;
    let Value::TaggedValue { tag, payload } = run(src) else {
        panic!("expected validation result");
    };
    assert_eq!(tag.as_ref(), "valid");
    assert_eq!(payload[0].1.peek(), Some(Value::Int(42)));
}

/// The prelude `FromData` recipe body (`<T> => deriveFromData`) lands in every
/// referencing program's THIR arena. It must not poison the default path: a bare
/// builder marker is not a `TypeValue`, so a plain decode still runs on the
/// strict TLC evaluator, which rejects runtime Type values and reflection.
#[test]
fn from_data_recipe_body_does_not_poison_tlc_path() {
    let src = r#"
result :: Validation DecodeIssue Int = decode (#int { value = 7; });
result
"#;
    let Value::TaggedValue { tag, payload } = eval_tlc_file(src).unwrap() else {
        panic!("expected validation result on the strict TLC path");
    };
    assert_eq!(tag.as_ref(), "valid");
    assert_eq!(payload[0].1.peek(), Some(Value::Int(7)));
}

#[test]
fn derived_from_data_decodes_record_and_accumulates_errors() {
    let src = r#"
Point :: type { x : Int; label : Text; };
FromData @Point :: derive
good :: Validation DecodeIssue Point = fromData (#record { fields = {
  { name = "x"; value = #int { value = 3; }; };
  { name = "label"; value = #text { value = "ok"; }; };
}; });
good
"#;
    let Value::TaggedValue { tag, payload } = run(src) else {
        panic!("expected validation result");
    };
    assert_eq!(tag.as_ref(), "valid");
    let Some(Value::Record(fields)) = payload[0].1.peek() else {
        panic!("expected decoded record");
    };
    assert_eq!(fields[0].1.peek(), Some(Value::Int(3)));

    let bad = r#"
Point :: type { x : Int; label : Text; };
FromData @Point :: derive
bad :: Validation DecodeIssue Point = fromData (#record { fields = {
  { name = "x"; value = #text { value = "wrong"; }; };
  { name = "extra"; value = #int { value = 9; }; };
}; });
bad
"#;
    let Value::TaggedValue { tag, payload } = run(bad) else {
        panic!("expected validation result");
    };
    assert_eq!(tag.as_ref(), "invalid");
    let Some(Value::List(errors)) = payload[0].1.peek() else {
        panic!("expected accumulated errors");
    };
    assert_eq!(errors.len(), 2);
    let Some(Value::Record(issue)) = errors[0].peek() else {
        panic!()
    };
    let Some(Value::List(path)) = issue[0].1.peek() else {
        panic!()
    };
    assert!(
        matches!(path[0].peek(), Some(Value::TaggedValue { tag, .. }) if tag.as_ref() == "field")
    );
}

#[test]
fn derived_from_data_treats_missing_optional_record_field_as_absent() {
    let src = r#"
Config :: type { name : Text; note? : Text; };
FromData @Config :: derive
result :: Validation DecodeIssue Config = decode (#record { fields = {
  { name = "name"; value = #text { value = "zutai"; }; };
}; });
result
"#;
    let Value::TaggedValue { tag, payload } = run(src) else {
        panic!("expected validation result");
    };
    assert_eq!(tag.as_ref(), "valid");
    let value = payload
        .iter()
        .find(|(name, _)| name.as_ref() == "value")
        .and_then(|(_, value)| value.peek())
        .expect("valid payload value");
    let Value::Record(fields) = value else {
        panic!("expected decoded record");
    };
    assert!(fields.iter().any(|(name, _)| name.as_ref() == "name"));
    assert!(!fields.iter().any(|(name, _)| name.as_ref() == "note"));
}

#[test]
fn derived_from_data_decodes_present_optional_record_field() {
    let src = r#"
Config :: type { name : Text; note? : Text; };
FromData @Config :: derive
result :: Validation DecodeIssue Config = decode (#record { fields = {
  { name = "name"; value = #text { value = "zutai"; }; };
  { name = "note"; value = #text { value = "typed"; }; };
}; });
result
"#;
    let Value::TaggedValue { tag, payload } = run(src) else {
        panic!("expected validation result");
    };
    assert_eq!(tag.as_ref(), "valid");
    let value = payload
        .iter()
        .find(|(name, _)| name.as_ref() == "value")
        .and_then(|(_, value)| value.peek())
        .expect("valid payload value");
    assert_eq!(
        record_field_value(&value, "note"),
        Value::Text("typed".into())
    );
}

#[test]
fn derived_from_data_decodes_lists_optionals_and_unions() {
    let list_src = r#"
value :: Validation DecodeIssue (List Int) = fromData (#list { items = {
  #int { value = 1; };
  #int { value = 2; };
}; });
value
"#;
    let Value::TaggedValue { tag, payload } = run(list_src) else {
        panic!()
    };
    assert_eq!(tag.as_ref(), "valid");
    assert!(matches!(payload[0].1.peek(), Some(Value::List(items)) if items.len() == 2));

    let optional_src = r#"
value :: Validation DecodeIssue (Int?) = fromData (#tagged {
  tag = "some";
  payload = #int { value = 7; };
});
value
"#;
    let Value::TaggedValue { tag, .. } = run(optional_src) else {
        panic!()
    };
    assert_eq!(tag.as_ref(), "valid");

    let union_src = r#"
Choice :: type { #off; #count : { value : Int; }; };
FromData @Choice :: derive
value :: Validation DecodeIssue Choice = fromData (#tagged {
  tag = "count";
  payload = #record { fields = {
    { name = "value"; value = #int { value = 9; }; };
  }; };
});
value
"#;
    let Value::TaggedValue { tag, payload } = run(union_src) else {
        panic!()
    };
    assert_eq!(tag.as_ref(), "valid");
    assert!(
        matches!(payload[0].1.peek(), Some(Value::TaggedValue { tag, .. }) if tag.as_ref() == "count")
    );
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
fn int_remainder() {
    assert_eq!(run("17 % 5"), Value::Int(2));
    assert_eq!(run("1 + 5 % 2 * 3"), Value::Int(4));
}

#[test]
fn int_remainder_by_zero() {
    assert_eq!(run_err("1 % 0"), EvalError::RemByZero);
}

#[test]
fn thir_int_arithmetic_overflow_reports_operator() {
    assert_eq!(
        eval_thir_file("9223372036854775807 + 1").unwrap_err(),
        EvalError::IntOverflow("+")
    );
    assert_eq!(
        eval_thir_file("3037000500 * 3037000500").unwrap_err(),
        EvalError::IntOverflow("*")
    );
    assert_eq!(
        eval_thir_file("(-9223372036854775807 - 1) / -1").unwrap_err(),
        EvalError::IntOverflow("/")
    );
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

#[test]
fn cond_selects_first_true_branch() {
    assert_eq!(
        run("cond { false => 1; true => 2; _ => 3; }"),
        Value::Int(2)
    );
}

#[test]
fn cond_uses_default_branch() {
    assert_eq!(run("cond { false => 1; _ => 2; }"), Value::Int(2));
}

#[test]
fn cond_short_circuits_branches() {
    assert_eq!(run("cond { true => 1; _ => 1 / 0; }"), Value::Int(1));
    assert_eq!(run("cond { false => 1 / 0; _ => 2; }"), Value::Int(2));
}

// ─── let blocks ───────────────────────────────────────────────────────────────

#[test]
fn block_single_binding() {
    assert_eq!(run("[ a := 42; a ]"), Value::Int(42));
}

#[test]
fn block_typed_binding() {
    assert_eq!(run("[ a : Int = 42; a ]"), Value::Int(42));
}

#[test]
fn block_sequential_bindings() {
    assert_eq!(run("[ a := 1; b := a + 1; b ]"), Value::Int(2));
}

// ─── records and field access ─────────────────────────────────────────────────

#[test]
fn record_field_access() {
    // Records require a trailing `;` after each field.
    assert_eq!(run("{ x = 10; y = 20; }.x"), Value::Int(10));
}

#[test]
fn uninferred_record_field_access_reports_receiver_annotation_hint() {
    let err = run_err("f x = x.host;\nf");
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages.iter().any(|message| {
            message.contains("field access `.host` needs a known record type")
                && message.contains("typed helper")
        }),
        "expected row annotation hint for `.host`, got {messages:?}"
    );
}

#[test]
fn record_equality() {
    assert_eq!(run("{ x = 1; } == { x = 1; }"), Value::Bool(true));
    assert_eq!(run("{ x = 1; } == { x = 2; }"), Value::Bool(false));
}

/// Record equality is order-independent AND deterministic. The TLC evaluator's
/// `PartialEq` once sorted fields by the field-name string's POINTER ADDRESS,
/// which is nondeterministic across runs (ASLR / allocation order) and made
/// `==` on equal records flip between `true` and `false`. Fields are now sorted
/// by name CONTENT, so permuted-order records compare equal every time.
#[test]
fn record_equality_is_order_independent() {
    assert_eq!(
        run("{ a = 1; b = 2; } == { b = 2; a = 1; }"),
        Value::Bool(true)
    );
    assert_eq!(
        run("{ p = { x = 1; y = 2; }; q = 3; } == { q = 3; p = { y = 2; x = 1; }; }"),
        Value::Bool(true)
    );
    assert_eq!(
        run("{ p = { x = 1; y = 2; }; } == { p = { x = 1; y = 9; }; }"),
        Value::Bool(false)
    );
}

#[test]
fn record_update_replaces_only_named_field() {
    let v = run(r#"{ host = "h"; port = 80; } with { port = 8080; }"#);
    assert_eq!(record_field_value(&v, "host"), Value::Text("h".into()));
    assert_eq!(record_field_value(&v, "port"), Value::Int(8080));
}

#[test]
fn record_value_spread_merges_left_to_right() {
    let v = run(r#"
base ::= { host = "h"; port = 80; };
extra ::= { debug = true; port = 443; };
{ * base; * extra; port = 8080; }
"#);
    assert_eq!(record_field_value(&v, "host"), Value::Text("h".into()));
    assert_eq!(record_field_value(&v, "debug"), Value::Bool(true));
    assert_eq!(record_field_value(&v, "port"), Value::Int(8080));
}

#[test]
fn record_update_does_not_force_unchanged_fields() {
    let src = r#"
bad :: Int = bad;
({ host = "h"; port = bad; } with { host = "new"; }).host
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Text("new".into()));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Text("new".into()));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Text("new".into()));
}

#[test]
fn record_update_preserves_present_optional_field_in_both_evaluators() {
    let src = r#"
S :: type { x : Int; y? : Int; };
s :: S = { x = 1; y = 2; };
(s with { x = 3; }).y
"#;
    assert_eq!(eval_file(src).unwrap().to_string(), "#present (2)");
    assert_eq!(eval_tlc_file(src).unwrap().to_string(), "#present (2)");
    assert_eq!(eval_thir_file(src).unwrap().to_string(), "#present (2)");
}

#[test]
fn record_update_preserves_absent_optional_field_in_both_evaluators() {
    let src = r#"
S :: type { x : Int; y? : Int; };
s :: S = { x = 1; };
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
};
base :: Config = { host = "localhost"; port = 80; };
patch :: Patch Config = { port = 8080; };
(overlay patch base).port
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
};
bad :: Int = bad;
base :: Config = { host = "localhost"; port = bad; };
patch :: Patch Config = { host = "patched"; };
(overlay patch base).host
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
};
Config :: type {
  server : Server;
  name : Text;
};
base :: Config = {
  server = { host = "localhost"; port = 80; };
  name = "dev";
};

patch :: DeepPatch Config = {
  server = { port = 8080; };
};
(overlayDeep patch base).server.host
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
};
base :: Config = { host = "localhost"; port = 80; };
patch :: Patch Config = { port = 8080; };
applyPatch ::= overlay patch;
(applyPatch base).port
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Int(8080));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(8080));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Int(8080));
}

// ─── record field-pun shorthand ──────────────────────────────────────────────

#[test]
fn record_field_pun_desugars_to_same_value() {
    // `{ port =; }` is sugar for `{ port = port; }`.
    let punned = run("port ::= 8080;\n({ port =; }).port");
    let explicit = run("port ::= 8080;\n({ port = port; }).port");
    assert_eq!(punned, Value::Int(8080));
    assert_eq!(punned, explicit);
}

#[test]
fn record_update_field_pun_uses_binding_in_scope() {
    // `cfg with { port =; }` is sugar for `cfg with { port = port; }`, so the
    // updated field takes the in-scope `port`, not the receiver's old value.
    let v =
        run("cfg ::= { host = \"h\"; port = 1; };\nport ::= 8080;\n(cfg with { port =; }).port");
    assert_eq!(v, Value::Int(8080));
}

// ─── select projection ─────────────────────────────────────────────────────────

#[test]
fn select_projects_fields_in_requested_order() {
    // `select` projects exactly the named fields, in the requested order,
    // dropping the unselected `name`.
    let v = run("s ::= { host = \"h\"; port = 8080; name = \"n\"; };\nselect s { port; host; }");
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
        run("s ::= { host = \"h\"; port = 8080; };\n(select s { port; }).port"),
        Value::Int(8080)
    );
}

#[test]
fn select_unknown_field_is_type_check_failure() {
    // An unknown selected field is a type error, so the interpreter refuses to
    // evaluate (evaluation is gated on complete typed IR).
    let err = run_err("s ::= { host = \"h\"; };\nselect s { missing; }");
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
    assert_eq!(run("{1; 2; 3;} == {1; 2; 3;}"), Value::Bool(true));
    assert_eq!(run("{1; 2; 3;} == {1; 2; 4;}"), Value::Bool(false));
}

#[test]
fn list_value_spread_concatenates_in_place() {
    let src = "xs ::= {2; 3;};\n{1; * xs; 4;} == {1; 2; 3; 4;}";
    assert_eq!(run(src), Value::Bool(true));
}

#[test]
fn stream_generator_evaluates_as_codata_stream() {
    // `Stream A` is demand-driven codata (`Unit -> StreamCell A`), so a generator
    // is observed by forcing/folding it, not as a list. Summing the two yielded
    // elements exercises the desugared thunk + `#cons`/`#nil` cell.
    let src = "sumS :: Stream Int -> Int\n  = s => match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + sumS t;\n  };\nsumS (stream { yield 1; yield 2; })\n";
    assert_eq!(run(src), Value::Int(3));
}

#[test]
fn stream_tolist_materializes_finite_stream() {
    // V3-G2 List interop: `toList` walks a codata stream into a builtin `List`
    // via the `listEmpty`/`listCons` bridge primitives. The result equals the
    // corresponding list literal.
    let src = "toList (cons 1 (cons 2 (singleton 3)))\n";
    assert_eq!(run(src), run("{1; 2; 3;}"));
}

#[test]
fn stream_fromlist_roundtrips_through_tolist() {
    // `fromList` adapts a `List` into a codata stream via `listIsNil`/`listHead`/
    // `listTail`; `toList` materializes it back, yielding the original list.
    assert_eq!(run("toList (fromList {4; 5; 6;})"), run("{4; 5; 6;}"));
}

#[test]
fn stream_fromlist_empty_is_empty_stream() {
    // The empty builtin `List` adapts to the empty stream; `toList` yields `[]`.
    let src = "e :: List Int = {;};\ntoList (fromList e)\n";
    assert_eq!(run(src), run("e :: List Int = {;};\ne"));
}

#[test]
fn stream_takelist_bounds_infinite_generator() {
    // `takeList` = `toList ∘ take`: a bounded prefix of an infinite generator.
    let src = "countFrom :: Int -> Stream Int\n  = n _ => #cons { head = n; tail = countFrom (n + 1); };\ntakeList 3 (countFrom 1)\n";
    assert_eq!(run(src), run("{1; 2; 3;}"));
}

#[test]
fn bare_stream_constructor_is_rejected_as_value_type() {
    let err = run_err("bad :: Stream = {;};\nbad");
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages.iter().any(|msg| msg.contains("Stream")),
        "expected Stream type diagnostic, got {messages:?}"
    );
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
answer :: Int = 42;
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
add_two ::= add 2;
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

// ─── algebraic effects ────────────────────────────────────────────────────────

#[test]
fn handler_clause_may_return_directly_without_resuming() {
    let src = r#"
result ::= handle [ perform fail "bad"; "unreachable" ] with {
  fail = \e. "fallback";
};
result
"#;
    assert_eq!(run(src), Value::Text("fallback".into()));
}
// ─── black-hole detection ─────────────────────────────────────────────────────

#[test]
fn black_hole_detected() {
    // `x :: Int = x` type-checks (both sides are Int) but diverges at runtime.
    let src = "
x :: Int = x;
x
";
    assert_eq!(run_err(src), EvalError::BlackHole);
}

#[test]
fn strict_tlc_black_hole_detected() {
    let src = "
x :: Int = x;
x
";
    assert_eq!(eval_tlc_file(src), Err(EvalError::BlackHole));
}

#[test]
fn tlc_lazy_record_projection_skips_unselected_black_hole() {
    let src = "bad :: Int = bad;\n{ ok = 1; bad = bad; }.ok";
    assert_eq!(eval_file(src).unwrap(), Value::Int(1));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(1));
    assert_eq!(eval_thir_file(src).unwrap(), Value::Int(1));
}

// ─── gate refusal — type errors must never produce a value ────────────────────

#[test]
fn gate_refuses_type_error() {
    let src = "
x :: Int = \"bad\";
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
    let src = "{1; 2}";
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
};
server :: RawServer = {};
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
};
server :: RawServer = {
  port = 9000;
};
server.port ?? 8080
";
    assert_eq!(run(src), Value::Int(9000));
}

#[test]
fn coalesce_explicit_none_takes_default() {
    // Regression: an explicit `#none` optional value must default, not pass
    // through. `??` unwraps one Optional or Maybe wrapper.
    let src = "x :: Int? = #none;\nx ?? 5";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn coalesce_explicit_some_unwraps_value() {
    // Regression: an explicit `#some (x)` must unwrap to `x`.
    let src = "x :: Int? = #some (9);\nx ?? 5";
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn coalesce_some_skips_fallback() {
    let src = "x :: Int? = #some (9);\nx ?? (1 / 0)";
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn coalesce_explicit_some_text_unwraps() {
    let src = "x :: Text? = #some (\"hi\");\nx ?? \"def\"";
    assert_eq!(run(src), Value::Text("hi".into()));
}

#[test]
fn coalesce_maybe_absent_takes_default() {
    let src = "
S :: type { p? : Int; };
s :: S = {};
s.p ?? 5
";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn coalesce_maybe_present_unwraps_value() {
    let src = "
S :: type { p? : Int; };
s :: S = { p = 9; };
s.p ?? 5
";
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn coalesce_present_skips_fallback() {
    let src = "
S :: type { p? : Int; };
s :: S = { p = 9; };
s.p ?? (1 / 0)
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
};
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

// ─── Fix #02: u64 literals above i64::MAX ────────────────────────────────────

/// u64::MAX evaluates to Value::Int(u64::MAX as i64) — the bit-faithful
/// representation under the untagged-i64 runtime ABI (D-0002).
/// NOTE: Value::Int renders as a signed decimal, so this prints as "-1".
/// A future unsigned-display fix should update the expected string intentionally.
#[test]
fn u64_max_evaluates_as_bit_faithful_int() {
    assert_eq!(run("18446744073709551615u64"), Value::Int(u64::MAX as i64));
}

/// 9223372036854775808u64 (first value above i64::MAX) evaluates as i64::MIN.
#[test]
fn u64_above_i64_max_evaluates_as_bit_faithful_int() {
    assert_eq!(run("9223372036854775808u64"), Value::Int(i64::MIN));
}
