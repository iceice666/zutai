use zutai_semantic::analyze;
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
    // Passes are stubs — we just verify no panic and no semantic diagnostics.
    assert_no_semantic_diags(src, label);
}

// ── Smoke: valid fixtures ─────────────────────────────────────────────────────
//
// All valid fixtures must pass through the (currently stubbed) semantic pass
// with zero diagnostics and no panic. As passes are implemented, these tests
// stay green (valid code → no errors).

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

// ── Semantic-gap fixtures ─────────────────────────────────────────────────────
//
// These are spec-invalid per v0 but have no semantic pass to catch them yet.
// They must pass through the stub passes without panic or false-positive diagnostics.
//
// When a pass is implemented that catches each case, move the fixture to
// `crates/general/fixtures/invalid/`, update `fixtures/EXPECTATIONS.md`, and
// flip the test below to `assert_has_semantic_error` (add that helper when needed).

#[test]
fn semantic_gap_closed_records() {
    assert_no_panic(
        include_str!("../../fixtures/semantic_invalid/closed_records.zt"),
        "semantic_invalid/closed_records.zt",
    );
}

#[test]
fn semantic_gap_exhaustiveness() {
    assert_no_panic(
        include_str!("../../fixtures/semantic_invalid/exhaustiveness.zt"),
        "semantic_invalid/exhaustiveness.zt",
    );
}

#[test]
fn semantic_gap_union_membership() {
    assert_no_panic(
        include_str!("../../fixtures/semantic_invalid/union_membership.zt"),
        "semantic_invalid/union_membership.zt",
    );
}

#[test]
fn semantic_gap_reserved_tag() {
    assert_no_panic(
        include_str!("../../fixtures/semantic_invalid/reserved_tag.zt"),
        "semantic_invalid/reserved_tag.zt",
    );
}
