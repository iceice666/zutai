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
        "{ host = \"localhost\"; port = 8080; enabled = true; tags = [#a; #b; none; -2.5e-3;]; nested = { x = false; }; }",
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
