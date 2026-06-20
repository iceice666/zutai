use super::*;

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
fn display_non_finite_floats_have_no_dot_zero() {
    // Regression: `inf`/`-inf`/`NaN` must not get a `.0` suffix (`inf.0` is
    // malformed and not valid float syntax).
    assert_eq!(run("1.0 / 0.0").to_string(), "inf");
    assert_eq!(run("(0.0 - 1.0) / 0.0").to_string(), "-inf");
    assert_eq!(run("0.0 / 0.0").to_string(), "NaN");
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
fn display_absent_optional_field_as_absent() {
    // Absent optional field access displays as #absent.
    let src = "
S :: type { port? : Int; }
s :: S = {}
s.port
";
    assert_eq!(run(src).to_string(), "#absent");
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
Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
}
s :: Status = #ok { code = 200; }
s
";
    assert_eq!(run(src).to_string(), "#ok { code = 200 }");
}

#[test]
fn display_closure_shows_remaining_arity() {
    // A function value displays as <function/N> where N = remaining arity
    let src = "
inc :: Int -> Int
  = x => x + 1;
inc
";
    assert_eq!(run(src).to_string(), "<function/1>");
}

#[test]
fn display_closure_partial_application() {
    // After partial application arity decreases: <function/2> → <function/1>
    let src = "
add :: Int -> Int -> Int
  = x y => x + y;
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
fn strict_tlc_rejects_runtime_type_values() {
    let src = "
MyInt :: type Int
MyInt
";
    match eval_tlc_file(src).unwrap_err() {
        EvalError::ReflectionUnsupported(message) => {
            assert!(message.contains("runtime Type values"));
        }
        other => panic!("expected ReflectionUnsupported, got {other:?}"),
    }
}

#[test]
fn default_eval_keeps_type_values_on_thir_boundary() {
    let src = "
MyInt :: type Int
MyInt
";
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

#[test]
fn fields_record_returns_field_metadata_with_type_values() {
    let value = run(r#"
Server :: type { host : Text; port? : Int; }
fields Server
"#);

    let host = list_item(&value, 0);
    assert_eq!(
        record_field_value(&host, "name"),
        Value::Text("host".into())
    );
    assert!(matches!(
        record_field_value(&host, "Type"),
        Value::TypeValue(_)
    ));
    assert_eq!(record_field_value(&host, "optional"), Value::Bool(false));

    let port = list_item(&value, 1);
    assert_eq!(
        record_field_value(&port, "name"),
        Value::Text("port".into())
    );
    assert!(matches!(
        record_field_value(&port, "Type"),
        Value::TypeValue(_)
    ));
    assert_eq!(record_field_value(&port, "optional"), Value::Bool(true));
}

#[test]
fn schema_record_returns_serializable_shape() {
    let value = run(r#"
Server :: type { host : Text; port? : Int; }
schema Server
"#);

    assert_eq!(
        record_field_value(&value, "kind"),
        Value::Atom("record".into())
    );
    let fields = record_field_value(&value, "fields");
    let host = list_item(&fields, 0);
    assert_eq!(
        record_field_value(&host, "name"),
        Value::Text("host".into())
    );
    assert_eq!(
        record_field_value(&host, "type"),
        Value::Text("Text".into())
    );
    assert_eq!(record_field_value(&host, "optional"), Value::Bool(false));

    let port = list_item(&fields, 1);
    assert_eq!(
        record_field_value(&port, "name"),
        Value::Text("port".into())
    );
    assert_eq!(record_field_value(&port, "type"), Value::Text("Int".into()));
    assert_eq!(record_field_value(&port, "optional"), Value::Bool(true));
}

#[test]
fn schema_generic_alias_substitutes_type_arguments() {
    let value = run(r#"
Box :: <A> type { value : A; }
schema (type Box Text)
"#);

    let fields = record_field_value(&value, "fields");
    let field = list_item(&fields, 0);
    assert_eq!(
        record_field_value(&field, "type"),
        Value::Text("Text".into())
    );
}

#[test]
fn schema_union_returns_variant_schema() {
    let value = run(r#"
Result :: type {
  #ok: { value : Text; };
  #err: { code : Int; };
  #done;
}
schema Result
"#);

    assert_eq!(
        record_field_value(&value, "kind"),
        Value::Atom("union".into())
    );
    let variants = record_field_value(&value, "variants");
    let ok = list_item(&variants, 0);
    assert_eq!(record_field_value(&ok, "name"), Value::Text("ok".into()));
    let ok_fields = record_field_value(&ok, "fields");
    let ok_value = list_item(&ok_fields, 0);
    assert_eq!(
        record_field_value(&ok_value, "type"),
        Value::Text("Text".into())
    );

    let done = list_item(&variants, 2);
    assert_eq!(
        record_field_value(&done, "name"),
        Value::Text("done".into())
    );
    let done_fields = record_field_value(&done, "fields");
    let Value::List(items) = done_fields else {
        panic!("expected fields list, got {done_fields:?}");
    };
    assert!(items.is_empty());
}

#[test]
fn reflection_rejects_open_rows() {
    let err = run_err(
        r#"
OpenServer :: type { host : Text; ...; }
schema OpenServer
"#,
    );
    assert!(
        matches!(err, EvalError::ReflectionUnsupported(ref message) if message.contains("open record rows")),
        "got {err:?}"
    );
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
