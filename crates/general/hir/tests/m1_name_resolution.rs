use zutai_hir::lower_file;
use zutai_syntax::diag::{Diagnostic, ErrorCode};
use zutai_syntax::parse;

fn lower_diags(src: &str) -> Vec<Diagnostic> {
    let parsed = parse(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "test source should parse cleanly, got {:#?}",
        parsed.diagnostics
    );

    let (_, diags) = lower_file(&parsed.syntax());
    diags
}

fn assert_no_lowering_diags(src: &str) {
    let diags = lower_diags(src);
    assert!(
        diags.is_empty(),
        "expected no lowering diagnostics, got {diags:#?}"
    );
}

fn assert_has_lowering_error(src: &str, code: ErrorCode) {
    let diags = lower_diags(src);
    assert!(
        diags.iter().any(|diag| diag.code == code),
        "expected lowering diagnostic {code:?}, got {diags:#?}"
    );
}

#[test]
fn unknown_expression_identifier_emits_e0020() {
    assert_has_lowering_error("missing_name", ErrorCode::UnknownIdentifier);
}

#[test]
fn top_level_forward_references_are_accepted() {
    assert_no_lowering_diags(
        r#"
first := second
second := 42
first
"#,
    );
}

#[test]
fn top_level_mutual_references_are_accepted() {
    assert_no_lowering_diags(
        r#"
left := right
right := left
left
"#,
    );
}

#[test]
fn block_local_forward_reference_emits_e0020() {
    assert_has_lowering_error(
        r#"
{
  x := y;
  y := 1;
  x
}
"#,
        ErrorCode::UnknownIdentifier,
    );
}

#[test]
fn builtin_type_annotation_references_are_accepted() {
    assert_no_lowering_diags(
        r#"
count : Int = 1
label : Text = "ok"
flag : Bool = true
items : List Int = []
items
"#,
    );
}
