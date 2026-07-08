use super::*;

// Fixture smoke test (M1)
// ---------------------------------------------------------------------------

const EXPR_CORE: &str = include_str!("../../../fixtures/expr_core.zt");
const VALID_CURSED_DISAMBIGUATION: &str =
    include_str!("../../../fixtures/valid/cursed_disambiguation.zt");
const VALID_CURSED_OPERATORS: &str = include_str!("../../../fixtures/valid/cursed_operators.zt");
const VALID_CURSED_PATTERNS: &str = include_str!("../../../fixtures/valid/cursed_patterns.zt");
const VALID_HIGHER_ORDER_FUNCTIONS: &str =
    include_str!("../../../fixtures/valid/higher_order_functions.zt");
const VALID_DEEP_OPTIONALS: &str = include_str!("../../../fixtures/valid/deep_optionals.zt");
const VALID_GENERIC_ALIASES: &str = include_str!("../../../fixtures/valid/generic_aliases.zt");
const VALID_DOC_STALE_SYNTAX: &str = include_str!("../../../fixtures/valid/doc_stale_syntax.zt");
const VALID_NESTED_MATCH: &str = include_str!("../../../fixtures/valid/nested_match.zt");
const VALID_LARGE_PROGRAM: &str = include_str!("../../../fixtures/valid/large_program.zt");
const INVALID_CHAINED_COMPARISON: &str =
    include_str!("../../../fixtures/invalid/chained_comparison.zt");
const INVALID_LAMBDA_ARROW: &str = include_str!("../../../fixtures/invalid/lambda_arrow.zt");
const INVALID_LAMBDA_TIGHT_DOT: &str =
    include_str!("../../../fixtures/invalid/lambda_tight_dot.zt");
const INVALID_LIST_MISSING_SEMICOLON: &str =
    include_str!("../../../fixtures/invalid/list_missing_semicolon.zt");
const INVALID_LOCAL_BINDING_DOUBLE_COLON: &str =
    include_str!("../../../fixtures/invalid/local_binding_double_colon.zt");
const INVALID_MISSING_FIELD_AFTER_ACCESS: &str =
    include_str!("../../../fixtures/invalid/missing_field_after_access.zt");
const INVALID_MIXED_PIPELINE: &str = include_str!("../../../fixtures/invalid/mixed_pipeline.zt");
const INVALID_RECORD_FIELD_COLON: &str =
    include_str!("../../../fixtures/invalid/record_field_colon.zt");
const INVALID_RECORD_PATTERN_MISSING_SEMICOLON: &str =
    include_str!("../../../fixtures/invalid/record_pattern_missing_semicolon.zt");
const INVALID_TOP_LEVEL_SINGLE_COLON: &str =
    include_str!("../../../fixtures/invalid/top_level_single_colon.zt");
const INVALID_STALE_TYPE_DECLARATION: &str =
    include_str!("../../../fixtures/invalid/stale_type_declaration.zt");
const INVALID_TAGGED_VALUE_PAYLOAD_COLON: &str =
    include_str!("../../../fixtures/invalid/tagged_value_payload_colon.zt");
const INVALID_TYPE_FIELD_EQUALS: &str =
    include_str!("../../../fixtures/invalid/type_field_equals.zt");
const INVALID_TYPE_UNION_PAYLOAD_EQUALS: &str =
    include_str!("../../../fixtures/invalid/type_union_payload_equals.zt");
const INVALID_UNCLOSED_RECORD: &str = include_str!("../../../fixtures/invalid/unclosed_record.zt");
const INVALID_UNCLOSED_LIST: &str = include_str!("../../../fixtures/invalid/unclosed_list.zt");
const INVALID_TRAILING_OPERATOR: &str =
    include_str!("../../../fixtures/invalid/trailing_operator.zt");

#[test]
fn parse_expr_core_fixture() {
    parse_str(EXPR_CORE);
}

#[test]
fn parse_cursed_fixture_variants() {
    for (name, src) in [
        (
            "valid/cursed_disambiguation.zt",
            VALID_CURSED_DISAMBIGUATION,
        ),
        ("valid/cursed_operators.zt", VALID_CURSED_OPERATORS),
        ("valid/cursed_patterns.zt", VALID_CURSED_PATTERNS),
        (
            "valid/higher_order_functions.zt",
            VALID_HIGHER_ORDER_FUNCTIONS,
        ),
        ("valid/deep_optionals.zt", VALID_DEEP_OPTIONALS),
        ("valid/generic_aliases.zt", VALID_GENERIC_ALIASES),
        ("valid/doc_stale_syntax.zt", VALID_DOC_STALE_SYNTAX),
        ("valid/nested_match.zt", VALID_NESTED_MATCH),
        ("valid/large_program.zt", VALID_LARGE_PROGRAM),
    ] {
        let parsed = parse(src);
        if parsed.ast().is_none() {
            let msgs: Vec<_> = parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect();
            panic!("parse({name}) failed:\n{}", msgs.join("\n"))
        }
    }
}

#[test]
fn reject_invalid_fixture_variants() {
    for (name, src) in [
        ("invalid/chained_comparison.zt", INVALID_CHAINED_COMPARISON),
        ("invalid/lambda_arrow.zt", INVALID_LAMBDA_ARROW),
        ("invalid/lambda_tight_dot.zt", INVALID_LAMBDA_TIGHT_DOT),
        (
            "invalid/list_missing_semicolon.zt",
            INVALID_LIST_MISSING_SEMICOLON,
        ),
        (
            "invalid/local_binding_double_colon.zt",
            INVALID_LOCAL_BINDING_DOUBLE_COLON,
        ),
        (
            "invalid/missing_field_after_access.zt",
            INVALID_MISSING_FIELD_AFTER_ACCESS,
        ),
        ("invalid/mixed_pipeline.zt", INVALID_MIXED_PIPELINE),
        ("invalid/record_field_colon.zt", INVALID_RECORD_FIELD_COLON),
        (
            "invalid/record_pattern_missing_semicolon.zt",
            INVALID_RECORD_PATTERN_MISSING_SEMICOLON,
        ),
        (
            "invalid/top_level_single_colon.zt",
            INVALID_TOP_LEVEL_SINGLE_COLON,
        ),
        (
            "invalid/stale_type_declaration.zt",
            INVALID_STALE_TYPE_DECLARATION,
        ),
        (
            "invalid/tagged_value_payload_colon.zt",
            INVALID_TAGGED_VALUE_PAYLOAD_COLON,
        ),
        ("invalid/type_field_equals.zt", INVALID_TYPE_FIELD_EQUALS),
        (
            "invalid/type_union_payload_equals.zt",
            INVALID_TYPE_UNION_PAYLOAD_EQUALS,
        ),
        ("invalid/unclosed_record.zt", INVALID_UNCLOSED_RECORD),
        ("invalid/unclosed_list.zt", INVALID_UNCLOSED_LIST),
        ("invalid/trailing_operator.zt", INVALID_TRAILING_OPERATOR),
    ] {
        assert!(parse(src).has_errors(), "{name} parsed successfully");
    }
}

#[test]
fn invalid_fixtures_report_specific_error_kinds() {
    for (name, src, kind) in [
        (
            "invalid/chained_comparison.zt",
            INVALID_CHAINED_COMPARISON,
            ParseErrorKind::ChainedComparison,
        ),
        (
            "invalid/lambda_arrow.zt",
            INVALID_LAMBDA_ARROW,
            ParseErrorKind::LambdaArrow,
        ),
        (
            "invalid/lambda_tight_dot.zt",
            INVALID_LAMBDA_TIGHT_DOT,
            ParseErrorKind::LambdaDotNeedsWhitespace,
        ),
        (
            "invalid/list_missing_semicolon.zt",
            INVALID_LIST_MISSING_SEMICOLON,
            ParseErrorKind::MissingListItemSemicolon,
        ),
        (
            "invalid/local_binding_double_colon.zt",
            INVALID_LOCAL_BINDING_DOUBLE_COLON,
            ParseErrorKind::LocalBindingDoubleColon,
        ),
        (
            "invalid/missing_field_after_access.zt",
            INVALID_MISSING_FIELD_AFTER_ACCESS,
            ParseErrorKind::MissingFieldAfterAccess,
        ),
        (
            "invalid/mixed_pipeline.zt",
            INVALID_MIXED_PIPELINE,
            ParseErrorKind::MixedPipeline,
        ),
        (
            "invalid/record_field_colon.zt",
            INVALID_RECORD_FIELD_COLON,
            ParseErrorKind::ValueRecordFieldUsesColon,
        ),
        (
            "invalid/record_pattern_missing_semicolon.zt",
            INVALID_RECORD_PATTERN_MISSING_SEMICOLON,
            ParseErrorKind::MissingListItemSemicolon,
        ),
        (
            "invalid/top_level_single_colon.zt",
            INVALID_TOP_LEVEL_SINGLE_COLON,
            ParseErrorKind::TopLevelSingleColon,
        ),
        (
            "invalid/stale_type_declaration.zt",
            INVALID_STALE_TYPE_DECLARATION,
            ParseErrorKind::StaleTypeDeclaration,
        ),
        (
            "invalid/tagged_value_payload_colon.zt",
            INVALID_TAGGED_VALUE_PAYLOAD_COLON,
            ParseErrorKind::TaggedValuePayloadUsesColon,
        ),
        (
            "invalid/type_field_equals.zt",
            INVALID_TYPE_FIELD_EQUALS,
            ParseErrorKind::TypeRecordFieldUsesEquals,
        ),
        (
            "invalid/type_union_payload_equals.zt",
            INVALID_TYPE_UNION_PAYLOAD_EQUALS,
            ParseErrorKind::TypeUnionPayloadUsesEquals,
        ),
        (
            "invalid/unclosed_record.zt",
            INVALID_UNCLOSED_RECORD,
            ParseErrorKind::UnclosedDelimiter('{'),
        ),
        (
            "invalid/unclosed_list.zt",
            INVALID_UNCLOSED_LIST,
            ParseErrorKind::UnclosedDelimiter('{'),
        ),
        (
            "invalid/trailing_operator.zt",
            INVALID_TRAILING_OPERATOR,
            ParseErrorKind::TrailingOperator,
        ),
    ] {
        let kinds = parse_kinds(src);
        assert_eq!(kinds.first(), Some(&kind), "{name}: {kinds:?}");
    }
}

#[test]
fn optional_access_without_field_reports_specific_error() {
    let kinds = parse_kinds("cfg ::= { server = #none; };\ncfg.server?.\n");
    assert_eq!(
        kinds.first(),
        Some(&ParseErrorKind::MissingFieldAfterAccess),
        "{kinds:?}"
    );
}

#[test]
fn ast_only_parse_matches_parse_diagnostics() {
    assert!(parse_ast_only("x ::= 1;\nx").ast().is_some());
    assert_eq!(
        parse_ast_only_kinds(INVALID_MIXED_PIPELINE),
        parse_kinds(INVALID_MIXED_PIPELINE)
    );
}

#[test]
fn lambda_string_boundary_does_not_capture_later_cond_arrow() {
    let src = r#"
[
  f := \path. "handler-mock";
  cond {
    value == "handler-mock" => false;
    _ => true;
  }
]
"#;
    let parsed = parse_ast_only(src);
    assert!(
        parsed.diagnostics().is_empty(),
        "{:?}",
        parsed.diagnostics()
    );
}

#[test]
fn reports_multiple_common_diagnostics_in_source_order() {
    let parsed = parse(
        r#"
[
  a := 1 < 2 < 3;
  b := \x => x;
  c := { 1; 2 }
]
"#,
    );
    assert!(parsed.has_errors(), "source should fail");

    let kinds: Vec<_> = parsed
        .diagnostics()
        .iter()
        .map(|err| err.kind.clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            ParseErrorKind::ChainedComparison,
            ParseErrorKind::LambdaArrow,
            ParseErrorKind::MissingListItemSemicolon,
        ]
    );
    assert!(
        parsed
            .diagnostics()
            .windows(2)
            .all(|pair| pair[0].primary_span().start <= pair[1].primary_span().start)
    );
}

#[test]
fn lossless_cst_round_trips_source_text() {
    let src = "--| doc\nanswer ::= --[ nested --[ inner ]-- ]-- 42\nanswer";
    let parsed = parse(src);
    assert_eq!(parsed.syntax().to_string(), src);
}

#[test]
fn tokenizer_preserves_comments_and_keywords() {
    let tokens = tokenize("-- hi\nif true then #ok else #no");
    let kinds: Vec<_> = tokens.iter().map(|token| token.kind).collect();
    assert!(kinds.contains(&SyntaxKind::LineComment));
    assert!(kinds.contains(&SyntaxKind::KeywordIf));
    assert!(kinds.contains(&SyntaxKind::KeywordTrue));
    assert!(kinds.contains(&SyntaxKind::KeywordThen));
    assert!(kinds.contains(&SyntaxKind::KeywordElse));
    assert!(kinds.contains(&SyntaxKind::Atom));
}

#[test]
fn diagnostic_exposes_structured_fix() {
    let parsed = parse("x : Int = 5\n5");
    let diagnostic = parsed.diagnostics().first().expect("expected diagnostic");
    assert_eq!(diagnostic.kind, ParseErrorKind::TopLevelSingleColon);
    assert_eq!(diagnostic.code, "zutai::parse::top_level_single_colon");
    assert_eq!(diagnostic.fixes.len(), 1);
    assert_eq!(diagnostic.fixes[0].edits[0].replacement, "::");
}

#[test]
fn unclosed_paren_reports_unclosed_delimiter() {
    let kinds = parse_kinds("x ::= (1 + 2\nx");
    assert!(
        kinds.contains(&ParseErrorKind::UnclosedDelimiter('(')),
        "expected unclosed `(` diagnostic, got {kinds:?}"
    );
}

#[test]
fn mismatched_delimiter_reports_matching_delimiter() {
    // `(` is closed by `]`: the scanner pops a Paren frame against a Bracket
    // close and emits an `ExpectedToken("matching delimiter")` diagnostic.
    let kinds = parse_kinds("x ::= (1 + 2]\nx");
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, ParseErrorKind::ExpectedToken(_))),
        "expected matching-delimiter diagnostic, got {kinds:?}"
    );
}

#[test]
fn parse_error_display_includes_span_and_expected() {
    use crate::error::ParseError;
    use crate::span::Span;
    let err = ParseError::from_kind(Span::new(3, 7), ParseErrorKind::ExpectedToken("then"))
        .with_expected(vec!["then", "else"]);
    let rendered = err.to_string();
    assert!(rendered.contains("3..7"), "span range shown: {rendered}");
    assert!(rendered.contains("then"), "message shown: {rendered}");
    assert!(
        rendered.contains("expected: then, else"),
        "expected list shown: {rendered}"
    );
}

#[test]
fn parse_error_from_kind_derives_dynamic_message() {
    use crate::error::ParseError;
    use crate::span::Span;
    let span = Span::new(0, 1);
    // `ExpectedToken` interpolates the token name into the message.
    let tok = ParseError::from_kind(span, ParseErrorKind::ExpectedToken("=>"));
    assert!(
        tok.message.contains("=>"),
        "token name interpolated: {}",
        tok.message
    );
    // `UnclosedDelimiter` interpolates the delimiter char.
    let delim = ParseError::from_kind(span, ParseErrorKind::UnclosedDelimiter('('));
    assert!(
        delim.message.contains('('),
        "delimiter interpolated: {}",
        delim.message
    );
}

#[test]
fn generic_fallback_names_offending_token() {
    // `@` mid-declaration is not caught by the heuristic scanner, so the
    // winnow fallback fires. It must name the stuck token and point a caret at
    // it, not emit a blank message at the construct's start.
    let parsed = parse("main ::= foo @ bar\nmain\n");
    let diag = parsed
        .diagnostics()
        .iter()
        .find(|d| d.kind == ParseErrorKind::Generic)
        .expect("expected a generic fallback diagnostic");
    assert_eq!(diag.message, "syntax error");
    let label = &diag.labels[0];
    assert_eq!(label.message, "unexpected `@`", "label names the token");
    assert_eq!(label.span.start, 13, "caret points at the `@`");
}

#[test]
fn generic_fallback_reports_end_of_input() {
    // A trailing binary operator with no following operand consumes through
    // EOF; the fallback should say so rather than pointing at a stale token.
    let parsed = parse("x ::= 1 +\nx\n");
    let diag = parsed
        .diagnostics()
        .iter()
        .find(|d| d.kind == ParseErrorKind::Generic)
        .expect("expected a generic fallback diagnostic");
    assert!(
        diag.labels[0].message.contains("end of input"),
        "label reports EOF: {}",
        diag.labels[0].message
    );
}
