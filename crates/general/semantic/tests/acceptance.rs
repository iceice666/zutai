use zutai_semantic::analyze;
use zutai_syntax::diag::ErrorCode;
use zutai_syntax::parse;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn assert_no_semantic_diags(src: &str, label: &str) {
    let parsed = parse(src);
    let result = analyze(&parsed.syntax());
    assert!(
        result.diagnostics.is_empty(),
        "{label}: expected no semantic diagnostics, got {:#?}",
        result.diagnostics
    );
}

fn assert_no_panic(src: &str, label: &str) {
    let parsed = parse(src);
    let _result = analyze(&parsed.syntax());
    let _ = label;
}

fn assert_has_semantic_error(src: &str, label: &str, code: ErrorCode) {
    let parsed = parse(src);
    let result = analyze(&parsed.syntax());
    assert!(
        result.diagnostics.iter().any(|diag| diag.code == code),
        "{label}: expected semantic diagnostic {code:?}, got {:#?}",
        result.diagnostics
    );
}

// ── Smoke: valid fixtures ─────────────────────────────────────────────────────
//
// Fixture files are primarily parser/lowering stress tests. Keep them as
// non-panic coverage; focused semantic tests below assert type-check behavior.

#[test]
fn smoke_cursed() {
    assert_no_panic(include_str!("../../fixtures/cursed.zt"), "cursed.zt");
}

#[test]
fn smoke_deep_nesting() {
    assert_no_panic(
        include_str!("../../fixtures/valid/deep_nesting.zt"),
        "valid/deep_nesting.zt",
    );
}

#[test]
fn smoke_optional_chains() {
    assert_no_panic(
        include_str!("../../fixtures/valid/optional_chains.zt"),
        "valid/optional_chains.zt",
    );
}

#[test]
fn smoke_lexical_torture() {
    assert_no_panic(
        include_str!("../../fixtures/valid/lexical_torture.zt"),
        "valid/lexical_torture.zt",
    );
}

#[test]
fn smoke_bracket_disambiguation() {
    assert_no_panic(
        include_str!("../../fixtures/valid/bracket_disambiguation.zt"),
        "valid/bracket_disambiguation.zt",
    );
}

#[test]
fn smoke_guards_and_blocks() {
    assert_no_panic(
        include_str!("../../fixtures/valid/guards_and_blocks.zt"),
        "valid/guards_and_blocks.zt",
    );
}

#[test]
fn smoke_type_position_torture() {
    assert_no_panic(
        include_str!("../../fixtures/valid/type_position_torture.zt"),
        "valid/type_position_torture.zt",
    );
}

#[test]
fn smoke_comments() {
    assert_no_panic(
        include_str!("../../fixtures/valid/comments.zt"),
        "valid/comments.zt",
    );
}

// ── Semantic fixtures ─────────────────────────────────────────────────────────

#[test]
fn m2_closed_records_emit_errors() {
    let src = include_str!("../../fixtures/invalid/closed_records.zt");
    assert_has_semantic_error(src, "invalid/closed_records.zt", ErrorCode::UnknownField);
    assert_has_semantic_error(src, "invalid/closed_records.zt", ErrorCode::TypeMismatch);
}

#[test]
fn m3_exhaustiveness_emits_non_exhaustive_match() {
    assert_has_semantic_error(
        include_str!("../../fixtures/invalid/exhaustiveness.zt"),
        "invalid/exhaustiveness.zt",
        ErrorCode::NonExhaustiveMatch,
    );
}

#[test]
fn m2_union_membership_emits_type_mismatch() {
    assert_has_semantic_error(
        include_str!("../../fixtures/invalid/union_membership.zt"),
        "invalid/union_membership.zt",
        ErrorCode::TypeMismatch,
    );
}

#[test]
fn m2_valid_closed_record_and_union_members_pass() {
    assert_no_semantic_diags(
        r#"
Server :: type { host : Text; port : Int; tls? : Bool; }
Env :: type [#dev; #test; #prod;]

server : Server = { host = "localhost"; port = 8080; }
env : Env = #dev

{ server = server; env = env; }
"#,
        "m2 valid closed record and union members",
    );
}

#[test]
fn m2_tag_is_normal_name_and_field() {
    assert_no_semantic_diags(
        r#"
_tag := #top

Tagged :: type { _tag : #record; radius : Float; }

tagged : Tagged = { _tag = #record; radius = 5.0; }

{
  binding = _tag;
  field = tagged._tag;
}
"#,
        "m2 _tag is normal name and field",
    );
}

#[test]
fn m2_function_call_checks_union_argument() {
    assert_has_semantic_error(
        r#"
Env :: type [#dev; #test; #prod;]
greet :: Env -> Text
      :: #dev { "dev" }
      :: #test { "test" }
      :: #prod { "prod" }

greet #staging
"#,
        "m2 function call union argument",
        ErrorCode::TypeMismatch,
    );
}

#[test]
fn m3_wildcard_is_exhaustive() {
    assert_no_semantic_diags(
        r#"
  Status :: type [#ok; #err; #pending;]

  describe :: Status -> Text
           :: #ok { "ok" }
           :: _ { "other" }

  describe #ok
  "#,
        "m3 wildcard exhaustive",
    );
}

#[test]
fn m3_guarded_arm_does_not_count() {
    assert_has_semantic_error(
        r#"
  Status :: type [#ok; #err;]

  describe :: Status -> Text
           :: x if true { "some" }

  describe #ok
  "#,
        "m3 guarded catch-all not exhaustive",
        ErrorCode::NonExhaustiveMatch,
    );
}

#[test]
fn m3_match_field_access_scrutinee_is_checked() {
    assert_has_semantic_error(
        r#"
  Status :: type [#ok; #err; #pending;]
  Box :: type { status : Status; }

  box : Box = { status = #ok; }

  describe :: Box -> Text
           :: item {
             match item.status {
               (#ok)  => "ok";
               (#err) => "error";
             }
           }

  describe box
  "#,
        "m3 match field access scrutinee",
        ErrorCode::NonExhaustiveMatch,
    );
}
