use super::*;

fn assert_same_as_winnow(input: &str) {
    let parsed = parse(input).unwrap();
    let mut oracle_input = input;
    let oracle = zutai_im_syntax::parser::parse(&mut oracle_input).unwrap();

    assert_eq!(oracle_input, "");
    assert_eq!(parsed, oracle);
}

#[test]
fn scan_indexes_structural_and_pseudo_starts() {
    let index = scan("{ name = \"not;structural\"; list = [#a; 2;]; }").unwrap();

    assert!(index.structural.contains(&0));
    assert!(index.structural.contains(&7));
    assert!(index.structural.contains(&25));
    assert!(!index.structural.contains(&12));
    assert!(index.pseudo_structural.contains(&2));
    assert!(index.pseudo_structural.contains(&9));
    assert!(index.pseudo_structural.contains(&35));
    assert_eq!(index.chunks[0].base, 0);
    assert_ne!(index.chunks[0].quote_mask, 0);
    assert_ne!(index.chunks[0].pseudo_structural_mask, 0);
}

#[test]
fn scan_keeps_escaped_quote_inside_string() {
    let input = "{ s = \"quote: \\\" } ;\"; next = true; }";
    let index = scan(input).unwrap();
    let inside_brace = input.find('}').unwrap();

    assert!(!index.structural.contains(&inside_brace));
    assert_same_as_winnow(input);
}

#[test]
fn parse_empty_block() {
    assert_same_as_winnow("{}");
}

#[test]
fn parse_blocks_arrays_and_scalars() {
    assert_same_as_winnow(
        "{ host = \"localhost\"; port = 8080; enabled = true; tags = [#a; #b; #none; -2.5e-3;]; nested = { x = false; }; }",
    );
}

#[test]
fn parse_complex_fixture_matches_winnow() {
    assert_same_as_winnow(include_str!("../../fixtures/complex.zti"));
}

#[test]
fn rejects_top_level_non_block() {
    assert!(matches!(
        parse("[1;]").unwrap_err().kind,
        ParseErrorKind::Expected { expected: "`{`" }
    ));
}

#[test]
fn rejects_duplicate_fields() {
    assert!(matches!(
        parse("{ a = 1; a = 2; }").unwrap_err().kind,
        ParseErrorKind::DuplicateField(name) if name == "a"
    ));
}

#[test]
fn rejects_comments() {
    assert!(parse("{ // comment\n a = 1; }").is_err());
}

#[test]
fn rejects_missing_semicolon() {
    assert!(parse("{ a = 1 }").is_err());
    assert!(parse("{ a = [1] ; }").is_err());
}

#[test]
fn rejects_trailing_data() {
    assert!(matches!(
        parse("{} {}").unwrap_err().kind,
        ParseErrorKind::TrailingData
    ));
}

#[test]
fn rejects_invalid_numbers() {
    assert!(matches!(
        parse("{ a = 01; }").unwrap_err().kind,
        ParseErrorKind::InvalidNumber
    ));
    assert!(matches!(
        parse("{ a = 1.; }").unwrap_err().kind,
        ParseErrorKind::InvalidNumber
    ));
    assert!(matches!(
        parse("{ a = 1e9999; }").unwrap_err().kind,
        ParseErrorKind::InvalidNumber
    ));
}

#[test]
fn rejects_invalid_strings() {
    assert!(matches!(
        parse("{ s = \"\\x\"; }").unwrap_err().kind,
        ParseErrorKind::InvalidEscape
    ));
    assert!(matches!(
        parse("{ s = \"\\ud83d\"; }").unwrap_err().kind,
        ParseErrorKind::InvalidEscape
    ));
    assert!(matches!(
        parse("{ s = \"\n\"; }").unwrap_err().kind,
        ParseErrorKind::InvalidString
    ));
}

#[test]
fn rejects_mismatched_delimiters() {
    assert!(parse("{ a = [1; };").is_err());
    assert!(parse("{ a = { b = 1; ]; }").is_err());
}

// ── Additional error-path coverage ───────────────────────────────────────────

/// Unterminated block — EOF before the closing `}` (L60-63 of parser.rs).
#[test]
fn rejects_unterminated_block() {
    assert!(matches!(
        parse("{ a = 1;").unwrap_err().kind,
        ParseErrorKind::Expected { expected: "`}`" }
    ));
}

/// Unterminated array — EOF before the closing `]` (L99-102 of parser.rs).
#[test]
fn rejects_unterminated_array() {
    assert!(matches!(
        parse("{ a = [1;").unwrap_err().kind,
        ParseErrorKind::Expected { expected: "`]`" }
    ));
}

/// Missing field name: `=` before any name bytes — L136-142 of parser.rs.
#[test]
fn rejects_missing_field_name() {
    assert!(parse("{ = 1; }").is_err());
}

#[test]
fn rejects_hyphenated_field_name() {
    assert!(parse("{ bad-name = 1; }").is_err());
}

/// Bare `#` at EOF is an invalid atom — L161 of parser.rs.
#[test]
fn rejects_bare_hash_as_atom() {
    assert!(matches!(
        parse("{ a = #; }").unwrap_err().kind,
        ParseErrorKind::InvalidAtom
    ));
}

/// `#` followed by a digit is an invalid atom start — L163-164 of parser.rs.
#[test]
fn rejects_hash_digit_as_atom() {
    assert!(matches!(
        parse("{ a = #1foo; }").unwrap_err().kind,
        ParseErrorKind::InvalidAtom
    ));
}

/// `true` followed by a name-continue char is not a valid `true` — L191-195.
#[test]
fn rejects_keyword_with_trailing_ident_char() {
    assert!(parse("{ a = trueX; }").is_err());
    assert!(parse("{ a = false1; }").is_err());
}

/// String containing a multi-byte UTF-8 character — L229-233 of parser.rs.
#[test]
fn accepts_utf8_string_value() {
    // The simd parser must handle multi-byte UTF-8 characters in string values.
    assert_same_as_winnow("{ a = \"café\"; }");
}

/// Unclosed string literal — L237 of parser.rs.
#[test]
fn rejects_unclosed_string() {
    assert!(matches!(
        parse("{ a = \"unclosed").unwrap_err().kind,
        ParseErrorKind::UnclosedString
    ));
}

/// `\` at EOF inside a string — L242-243 (parse_escape: None arm).
/// The scanner may surface this as UnclosedString or InvalidEscape depending
/// on whether the quote-tracking sees the escape; either way the parse fails.
#[test]
fn rejects_backslash_at_eof_in_string() {
    // The exact error kind depends on the SIMD scanner's string-state tracking,
    // but the parse must fail.
    assert!(
        parse("{ a = \"\\").is_err(),
        "backslash at EOF must be rejected"
    );
}

/// Specific string escapes: `\/`, `\b`, `\f`, `\r`, `\t` — L250-255.
/// These are the less-common JSON escapes rarely covered by happy-path tests.
#[test]
fn accepts_various_string_escapes() {
    // `\/` → '/'
    assert_same_as_winnow("{ a = \"\\/\"; }");
    // `\b` → backspace (0x08)
    assert_same_as_winnow("{ a = \"\\b\"; }");
    // `\f` → form feed (0x0C)
    assert_same_as_winnow("{ a = \"\\f\"; }");
    // `\r` → carriage return
    assert_same_as_winnow("{ a = \"\\r\"; }");
    // `\t` → tab
    assert_same_as_winnow("{ a = \"\\t\"; }");
}

/// Minus followed by a non-digit is an invalid number — L320 (the `_` arm).
#[test]
fn rejects_minus_followed_by_non_digit() {
    assert!(matches!(
        parse("{ a = -x; }").unwrap_err().kind,
        ParseErrorKind::InvalidNumber
    ));
}

/// Number with exponent marker but no digits — L341-342.
#[test]
fn rejects_number_with_empty_exponent() {
    assert!(matches!(
        parse("{ a = 1e; }").unwrap_err().kind,
        ParseErrorKind::InvalidNumber
    ));
    // Also with sign but no digits
    assert!(matches!(
        parse("{ a = 2.5e+; }").unwrap_err().kind,
        ParseErrorKind::InvalidNumber
    ));
}

// ── SIMD string-scan path coverage ───────────────────────────────────────────

/// Long string with no escapes — exercises the SIMD special-byte finder across
/// multiple full lanes before reaching the closing quote.
#[test]
fn accepts_long_plain_string_value() {
    let value = "abcdefghijklmnopqrstuvwxyz".repeat(8);
    let input = format!("{{ s = \"{value}\"; }}");
    assert_same_as_winnow(&input);
}

/// Long string with an escaped quote in the middle — exercises SIMD scanning of
/// the literal spans both before and after the scalar escape decoder.
#[test]
fn accepts_long_escaped_string_value() {
    let prefix = "a".repeat(96);
    let suffix = "z".repeat(96);
    let input = format!("{{ s = \"{prefix}\\\"{suffix}\"; }}");
    assert_same_as_winnow(&input);
}

/// Raw NUL inside a string — verifies the unsigned `< 0x20` control mask pins the
/// low end of the control range (a signed compare would only misclassify high
/// UTF-8 bytes, so NUL specifically checks the bias trick).
#[test]
fn rejects_nul_control_in_string() {
    assert!(matches!(
        parse("{ s = \"a\0b\"; }").unwrap_err().kind,
        ParseErrorKind::InvalidString
    ));
}
