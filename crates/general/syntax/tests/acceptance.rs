/// M12 acceptance tests: the complete fixture corpus parsed and validated.
///
/// **Parse-clean** fixtures: must yield zero diagnostics + lossless round-trip.
/// **Parse-error** fixtures: must yield ≥1 diagnostic + lossless round-trip + no panic.
use zutai_syntax::parse;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn assert_parses_clean(src: &str, name: &str) {
    let p = parse(src);
    assert_eq!(
        p.syntax().text().to_string(),
        src,
        "{name}: lossless round-trip failed"
    );
    assert!(
        p.diagnostics.is_empty(),
        "{name}: expected zero diagnostics, got {}: {:?}",
        p.diagnostics.len(),
        p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

fn assert_parses_with_error(src: &str, name: &str) {
    let p = parse(src);
    assert_eq!(
        p.syntax().text().to_string(),
        src,
        "{name}: lossless round-trip failed"
    );
    assert!(
        !p.diagnostics.is_empty(),
        "{name}: expected ≥1 diagnostic, got zero"
    );
}

// ── Parse-clean fixtures ──────────────────────────────────────────────────────

#[test]
fn acceptance_cursed_zt() {
    assert_parses_clean(include_str!("../../fixtures/cursed.zt"), "cursed.zt");
}

#[test]
fn acceptance_valid_deep_nesting() {
    assert_parses_clean(
        include_str!("../../fixtures/valid/deep_nesting.zt"),
        "valid/deep_nesting.zt",
    );
}

#[test]
fn acceptance_valid_optional_chains() {
    assert_parses_clean(
        include_str!("../../fixtures/valid/optional_chains.zt"),
        "valid/optional_chains.zt",
    );
}

#[test]
fn acceptance_valid_lexical_torture() {
    assert_parses_clean(
        include_str!("../../fixtures/valid/lexical_torture.zt"),
        "valid/lexical_torture.zt",
    );
}

// ── Parse-error fixtures ──────────────────────────────────────────────────────

#[test]
fn acceptance_invalid_comparison_chaining() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/comparison_chaining.zt"),
        "invalid/comparison_chaining.zt",
    );
}

#[test]
fn acceptance_invalid_pipeline_ambiguity() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/pipeline_ambiguity.zt"),
        "invalid/pipeline_ambiguity.zt",
    );
}

#[test]
fn acceptance_invalid_sigil_swaps() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/sigil_swaps.zt"),
        "invalid/sigil_swaps.zt",
    );
}

#[test]
fn acceptance_invalid_separator_swaps() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/separator_swaps.zt"),
        "invalid/separator_swaps.zt",
    );
}

#[test]
fn acceptance_invalid_keyword_misuse() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/keyword_misuse.zt"),
        "invalid/keyword_misuse.zt",
    );
}

#[test]
fn acceptance_invalid_no_unary_operator() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/no_unary_operator.zt"),
        "invalid/no_unary_operator.zt",
    );
}

#[test]
fn acceptance_invalid_atom_and_comment_traps() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/atom_and_comment_traps.zt"),
        "invalid/atom_and_comment_traps.zt",
    );
}

#[test]
fn acceptance_invalid_string_number_lexical() {
    assert_parses_with_error(
        include_str!("../../fixtures/invalid/string_number_lexical.zt"),
        "invalid/string_number_lexical.zt",
    );
}

// ── Never-panic property: random bytes ───────────────────────────────────────

#[test]
fn never_panic_single_bytes() {
    for b in 0u8..=127u8 {
        if let Ok(s) = std::str::from_utf8(&[b]) {
            let p = parse(s);
            assert_eq!(
                p.syntax().text().to_string(),
                s,
                "round-trip failed for byte {b:#04x}"
            );
        }
    }
}

#[test]
fn never_panic_two_byte_pairs() {
    let pairs: &[(&str, &str)] = &[
        ("a", "b"),
        ("\n", "a"),
        ("a", "\n"),
        ("::", "::"),
        (":=", ":="),
        ("->", "->"),
        ("|>", "|>"),
        ("<|", "<|"),
        ("??", "??"),
        (".", "."),
        ("{", "}"),
        ("[", "]"),
        ("(", ")"),
    ];
    for &(a, b) in pairs {
        let src = format!("{a}{b}");
        let p = parse(&src);
        assert_eq!(
            p.syntax().text().to_string(),
            src,
            "round-trip failed for {src:?}"
        );
    }
}

#[test]
fn never_panic_adversarial_sequences() {
    let inputs = [
        "",
        "   ",
        "\n\n\n",
        ":= := :=",
        ":: :: ::",
        "-> -> ->",
        "|> |> |>",
        "<| <| <|",
        "?? ?? ??",
        "{ { { } } }",
        "[ [ [ ] ] ]",
        "( ( ( ) ) )",
        "a b c d e",
        ":: a { } :: b { } :: c { }",
        "x : Int = y : Bool = true",
        "type type type",
        "match match { _ => _ }",
        "if if if then then then else else else",
        "#",
        "##",
        "###",
        "; ; ; ; ;",
        ",,,",
        "...",
        "=====",
        "-----",
        "?????",
    ];
    for src in &inputs {
        let p = parse(src);
        assert_eq!(
            p.syntax().text().to_string(),
            *src,
            "round-trip failed for {src:?}"
        );
    }
}
