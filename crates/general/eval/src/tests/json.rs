use super::*;

// ─── natural JSON serialization ─────────────────────────────────────────────

#[test]
fn to_json_record_list_atom() {
    let value = run("{ host = \"localhost\"; port = 8080; mode = #prod; flags = {true; false;}; }");
    assert_eq!(
        value.to_json().unwrap(),
        serde_json::json!({
            "host": "localhost",
            "port": 8080,
            "mode": "#prod",
            "flags": [true, false],
        })
    );
}

#[test]
fn to_json_rejects_non_finite_float() {
    assert_eq!(
        run("1.0 / 0.0").to_json().unwrap_err(),
        EvalError::Internal("cannot serialize non-finite float to JSON"),
    );
}

#[test]
fn to_json_tagged_union_named_payload() {
    let src = "Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
};
s :: Status = #ok { code = 200; };
s";
    assert_eq!(
        run(src).to_json().unwrap(),
        serde_json::json!({ "tag": "ok", "payload": { "code": 200 } }),
    );
}

// ─── immediate-mode (.zti) serialization ────────────────────────────────────

#[test]
fn to_immediate_document_round_trips_through_from_immediate() {
    let value = run(
        "{ host = \"localhost\"; port = 8080; ratio = 0.5; mode = #prod; flags = {true; false;}; nested = { ok = true; }; }",
    );
    let block = value.to_immediate_document().unwrap();
    let text = zutai_im::format_block(&block);
    assert_eq!(zutai_im::parse(&text).unwrap(), block);
    let reimported = Value::from_immediate(&zutai_im::Value::Block(block.clone()));
    assert_eq!(reimported.to_immediate_document().unwrap(), block);
}

#[test]
fn to_immediate_document_requires_a_record() {
    assert_eq!(
        run("{1; 2;}").to_immediate_document().unwrap_err(),
        EvalError::ImmediateUnsupported(
            "a .zti document must be a record, but the final value is List".to_string()
        ),
    );
}

#[test]
fn to_immediate_document_refuses_values_outside_the_data_model() {
    let cases = [
        ("{ t = (1, 2); }", "`t` is a tuple"),
        (
            "{ r = { xs = {1.0 / 0.0;}; }; }",
            "`r.xs[0]` is a non-finite float",
        ),
        (
            "Status :: type { #ok: { code : Int; }; #idle; };\ns :: Status = #ok { code = 1; };\n{ s = s; }",
            "`s` is the tagged value `#ok` with a payload",
        ),
    ];
    for (src, expected) in cases {
        let err = run(src).to_immediate_document().unwrap_err();
        let EvalError::ImmediateUnsupported(message) = &err else {
            panic!("expected ImmediateUnsupported for {src:?}, got {err:?}");
        };
        assert!(message.starts_with(expected), "{src:?}: {message}");
    }
}

#[test]
fn to_immediate_document_keeps_payload_free_tags_as_atoms() {
    let src = "Status :: type { #ok: { code : Int; }; #idle; };\ns :: Status = #idle;\n{ s = s; }";
    let block = run(src).to_immediate_document().unwrap();
    assert_eq!(block[0].value, zutai_im::Value::Atom("idle".to_string()));
}

// ─── runtime-ABI renderability (interpreter/native parity) ──────────────────

#[test]
fn runtime_abi_reason_accepts_first_order_data() {
    let value = run("{ host = \"localhost\"; ports = {80; 443;}; mode = #prod; }");
    assert_eq!(value.runtime_abi_reason(), None);
}

#[test]
fn runtime_abi_reason_rejects_function_entry() {
    // A program whose result is a function has no runtime-ABI representation,
    // matching the native backend's function-entry refusal.
    let value = run("\\x. x");
    assert_eq!(
        value.runtime_abi_reason(),
        Some("compiled entry point returns a function, which cannot be shown by the runtime ABI"),
    );
}

#[test]
fn runtime_abi_reason_rejects_type_value_nested_in_list() {
    // `fields T` yields records carrying runtime `Type` values; native refuses
    // the whole entry, so the interpreter must report the same Type reason even
    // though the outermost value is a plain list.
    let value = run("Server :: type { host : Text; };\nfields Server");
    assert_eq!(
        value.runtime_abi_reason(),
        Some("compiled entry point returns Type, which cannot be shown by the runtime ABI"),
    );
}
