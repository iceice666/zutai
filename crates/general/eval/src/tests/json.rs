use super::*;

// ─── natural JSON serialization ─────────────────────────────────────────────

#[test]
fn to_json_record_list_atom() {
    let value = run("{ host = \"localhost\"; port = 8080; mode = #prod; flags = [true; false;]; }");
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
}
s :: Status = #ok { code = 200; }
s";
    assert_eq!(
        run(src).to_json().unwrap(),
        serde_json::json!({ "tag": "ok", "payload": { "code": 200 } }),
    );
}
