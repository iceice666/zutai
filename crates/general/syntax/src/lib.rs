//! Syntax support for Zutai general mode (`.zt`): lossless CST with error recovery.

pub mod ast;
pub mod diag;
pub mod lexer;
mod parser;
mod syntax_kind;
pub(crate) mod token_set;
pub mod validation;

pub use diag::Diagnostic;
pub use syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, ZutaiLanguage};

/// The result of parsing a `.zt` source file.
pub struct Parse {
    pub green: rowan::GreenNode,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// Root syntax node for tree navigation.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// Parse a `.zt` source string into a lossless green tree, then run the
/// validation pass to collect capitalization / reserved-name / duplicate lints.
pub fn parse(src: &str) -> Parse {
    let (green, mut diagnostics) = parser::parse(src);
    let root = SyntaxNode::new_root(green.clone());
    validation::validate(&root, &mut diagnostics);
    Parse { green, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    fn assert_round_trips(src: &str) {
        let p = parse(src);
        assert_eq!(
            p.syntax().text().to_string(),
            src,
            "round-trip failed for {src:?}"
        );
    }

    #[test]
    fn empty_input_round_trips() {
        assert_round_trips("");
    }

    #[test]
    fn token_soup_round_trips() {
        let cases = [
            "foo bar",
            "  leading",
            "trailing  ",
            "  both  ",
            "   ",
            "42 + 1",
            r#""hello world""#,
            "a := b + c; d",
            "::= -> ?? |> <|",
        ];
        for src in cases {
            assert_round_trips(src);
        }
    }

    #[test]
    fn markers_and_precede() {
        // Drive a synthetic parse: wrap `b` in an inner node, then precede with outer.
        // Expected shape: FILE > OUTER_NODE > INNER_NODE > IDENT("b"), with trivia in place.
        // We test through the public `parse` API but with a known input.
        // Input: "a b" — two idents, whitespace trivia between.
        let p = parse("a b");
        let root = p.syntax();
        // The placeholder file() bumps everything flat, so all three raw tokens are direct
        // children of FILE. Confirm round-trip and that the root text is correct.
        assert_eq!(root.text().to_string(), "a b");

        // Snapshot the debug representation to pin tree shape.
        let debug = format!("{root:#?}");
        expect![[r#"
            FILE@0..3
              CALL_EXPR@0..3
                LITERAL@0..1
                  IDENT@0..1 "a"
                LITERAL@1..3
                  WHITESPACE@1..2 " "
                  IDENT@2..3 "b"
        "#]]
        .assert_eq(&debug);
    }

    // ── M3 precedence snapshot tests ─────────────────────────────────────────

    fn tree(src: &str) -> String {
        format!("{:#?}", parse(src).syntax())
    }

    fn diagnostics(src: &str) -> Vec<String> {
        parse(src)
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect()
    }

    #[test]
    fn prec_field_access_tighter_than_app() {
        // `f x.y` means `f (x.y)` — field access binds tighter than application
        expect![[r#"
            FILE@0..5
              CALL_EXPR@0..5
                LITERAL@0..1
                  IDENT@0..1 "f"
                ACCESS_EXPR@1..5
                  LITERAL@1..3
                    WHITESPACE@1..2 " "
                    IDENT@2..3 "x"
                  DOT@3..4 "."
                  FIELD_NAME@4..5
                    IDENT@4..5 "y"
        "#]]
        .assert_eq(&tree("f x.y"));
    }

    #[test]
    fn prec_optional_chain_and_defaulting() {
        // `raw.server?.port ?? 8080` — access/chain binds tighter than ??
        assert_round_trips("raw.server?.port ?? 8080");
        let t = tree("raw.server?.port ?? 8080");
        // Root of expr is BINARY_EXPR(??) whose LHS is OPTIONAL_ACCESS_EXPR
        assert!(t.contains("BINARY_EXPR"), "expected BINARY_EXPR at root");
        assert!(
            t.contains("OPTIONAL_ACCESS_EXPR"),
            "expected optional chain"
        );
        assert!(t.contains("ACCESS_EXPR"), "expected field access");
    }

    #[test]
    fn prec_pipeline_with_app() {
        // `x |> f a` means `x |> (f a)` — app tighter than pipeline
        assert_round_trips("x |> f a");
        let t = tree("x |> f a");
        // Root is PIPELINE_EXPR; its RHS is CALL_EXPR
        assert!(
            t.contains("PIPELINE_EXPR"),
            "expected PIPELINE_EXPR at root"
        );
        assert!(t.contains("CALL_EXPR"), "expected application in RHS");
    }

    #[test]
    fn prec_backward_pipeline_with_defaulting() {
        // `f <| x ?? y` means `f <| (x ?? y)` — ?? tighter than <|
        assert_round_trips("f <| x ?? y");
        let t = tree("f <| x ?? y");
        assert!(t.contains("PIPELINE_EXPR"), "expected PIPELINE_EXPR");
        assert!(t.contains("BINARY_EXPR"), "expected ?? node in RHS");
    }

    #[test]
    fn prec_right_assoc_defaulting() {
        // `a ?? b ?? c` = `a ?? (b ?? c)` — right-associative
        assert_round_trips("a ?? b ?? c");
        let t = tree("a ?? b ?? c");
        // Outer node is BINARY_EXPR; inner (nested) is also BINARY_EXPR
        let count = t.matches("BINARY_EXPR").count();
        assert_eq!(
            count, 2,
            "expected two BINARY_EXPR nodes for right-assoc ??"
        );
    }

    #[test]
    fn prec_left_assoc_pipeline() {
        // `a |> b |> c` = `(a |> b) |> c` — left-associative
        assert_round_trips("a |> b |> c");
        let t = tree("a |> b |> c");
        let count = t.matches("PIPELINE_EXPR").count();
        assert_eq!(count, 2, "expected two PIPELINE_EXPR for left-assoc |>");
    }

    #[test]
    fn prec_negative_literal_in_mul() {
        // `x * -1` — MINUS adjacent to INT folds into a negative LITERAL
        assert_round_trips("x * -1");
        let t = tree("x * -1");
        assert!(t.contains("BINARY_EXPR"), "expected BINARY_EXPR for *");
        // -1 is a LITERAL (wraps MINUS + INT); no separate subtraction BINARY_EXPR
        let binary_count = t.matches("BINARY_EXPR").count();
        assert_eq!(
            binary_count, 1,
            "negative literal should not produce a second BINARY_EXPR"
        );
    }

    #[test]
    fn prec_non_assoc_comparison_error() {
        // `a == b == c` — must emit a diagnostic
        let diags = diagnostics("a == b == c");
        assert!(
            diags.iter().any(|d| d.contains("non-associative")),
            "expected non-associative comparison diagnostic, got: {diags:?}"
        );
        // Tree must still be complete (round-trips)
        assert_round_trips("a == b == c");
    }

    #[test]
    fn prec_pipeline_mix_error() {
        // `x |> f <| y` — must emit a diagnostic
        let diags = diagnostics("x |> f <| y");
        assert!(
            diags.iter().any(|d| d.contains("mixing")),
            "expected pipeline-mix diagnostic, got: {diags:?}"
        );
        assert_round_trips("x |> f <| y");
    }

    // ── M4 composite expression tests ────────────────────────────────────────

    #[test]
    fn m4_empty_record() {
        assert_round_trips("{}");
        let t = tree("{}");
        assert!(t.contains("RECORD_EXPR"), "expected RECORD_EXPR");
    }

    #[test]
    fn m4_record_expr() {
        assert_round_trips("{ x = 1; y = 2; }");
        let t = tree("{ x = 1; y = 2; }");
        assert!(t.contains("RECORD_EXPR"), "expected RECORD_EXPR");
        assert_eq!(t.matches("VALUE_FIELD").count(), 2);
        assert!(t.contains("FIELD_NAME"), "expected FIELD_NAME nodes");
    }

    #[test]
    fn m4_hyphenated_field_record() {
        // Hyphenated field names: IDENT-IDENT when raw-adjacent
        assert_round_trips("{ target-triple = \"aarch64\"; }");
        let t = tree("{ target-triple = \"aarch64\"; }");
        assert!(t.contains("RECORD_EXPR"));
        assert!(t.contains("FIELD_NAME"));
    }

    #[test]
    fn m4_list_expr() {
        assert_round_trips("[1; 2; 3;]");
        let t = tree("[1; 2; 3;]");
        assert!(t.contains("LIST_EXPR"));
        assert_eq!(t.matches("LIST_ITEM").count(), 3);
    }

    #[test]
    fn m4_list_empty() {
        assert_round_trips("[]");
        let t = tree("[]");
        assert!(t.contains("LIST_EXPR"));
    }

    #[test]
    fn m4_lambda_arrow() {
        assert_round_trips("\\x => x + 1");
        let t = tree("\\x => x + 1");
        assert!(t.contains("LAMBDA_EXPR"));
        assert!(t.contains("BINARY_EXPR"), "expected body expr");
    }

    #[test]
    fn m4_lambda_multi_arg() {
        assert_round_trips("\\x y => x + y");
        let t = tree("\\x y => x + y");
        assert!(t.contains("LAMBDA_EXPR"));
    }

    #[test]
    fn m4_lambda_block() {
        assert_round_trips("\\x { x + 1 }");
        let t = tree("\\x { x + 1 }");
        assert!(t.contains("LAMBDA_EXPR"));
        assert!(t.contains("BLOCK"));
    }

    #[test]
    fn m4_lambda_block_with_binding() {
        assert_round_trips("\\x { y := x + 1; y }");
        let t = tree("\\x { y := x + 1; y }");
        assert!(t.contains("LAMBDA_EXPR"));
        assert!(t.contains("BLOCK"));
        assert!(t.contains("LOCAL_BINDING"));
    }

    #[test]
    fn m4_if_expr() {
        assert_round_trips("if true then 1 else 0");
        let t = tree("if true then 1 else 0");
        assert!(t.contains("IF_EXPR"));
    }

    #[test]
    fn m4_match_expr() {
        assert_round_trips("match x { true => 1; false => 0; _ => -1; }");
        let t = tree("match x { true => 1; false => 0; _ => -1; }");
        assert!(t.contains("MATCH_EXPR"));
        assert_eq!(t.matches("MATCH_CASE").count(), 3);
        assert!(t.contains("WILDCARD_PATTERN"));
    }

    #[test]
    fn m4_match_nested() {
        assert_round_trips("match a { none => match b { true => 1; false => 0; }; _ => -1; }");
        let t = tree("match a { none => match b { true => 1; false => 0; }; _ => -1; }");
        assert_eq!(t.matches("MATCH_EXPR").count(), 2, "outer and inner match");
    }

    #[test]
    fn m4_import_expr() {
        assert_round_trips("import \"lib.zt\"");
        let t = tree("import \"lib.zt\"");
        assert!(t.contains("IMPORT_EXPR"));
        assert!(t.contains("IMPORT_PATH"));
    }

    #[test]
    fn m4_tuple_single() {
        // Single element in parens → PAREN_EXPR, not tuple
        assert_round_trips("(#just)");
        let t = tree("(#just)");
        assert!(
            t.contains("PAREN_EXPR"),
            "single-element parens should be PAREN_EXPR"
        );
    }

    #[test]
    fn m4_tuple_multi() {
        assert_round_trips("(1, 2, 3)");
        let t = tree("(1, 2, 3)");
        assert!(t.contains("TUPLE_EXPR"));
        assert_eq!(t.matches("TUPLE_ITEM").count(), 3);
    }

    #[test]
    fn m4_tuple_named_fields() {
        // Tuple variant value: (#tag, field = value)
        assert_round_trips("(#just-value, value = 42)");
        let t = tree("(#just-value, value = 42)");
        assert!(t.contains("TUPLE_EXPR"));
        assert!(t.contains("VALUE_FIELD"), "expected named field in tuple");
    }

    #[test]
    fn m4_block_expr() {
        assert_round_trips("{ x := 1; x }");
        let t = tree("{ x := 1; x }");
        assert!(t.contains("BLOCK"));
        assert!(t.contains("LOCAL_BINDING"));
    }

    #[test]
    fn m4_lambda_as_arg() {
        // Lambda used as juxtaposition argument
        assert_round_trips("map \\x => x + 1");
        let t = tree("map \\x => x + 1");
        assert!(t.contains("CALL_EXPR"), "expected application");
        assert!(t.contains("LAMBDA_EXPR"));
    }

    #[test]
    fn m4_if_as_arg() {
        assert_round_trips("f (if true then a else b)");
        let t = tree("f (if true then a else b)");
        assert!(t.contains("CALL_EXPR"));
        assert!(t.contains("IF_EXPR"));
    }

    #[test]
    fn m4_curried_lambdas() {
        // Chained lambdas: \f => \g => \x => f (g x)
        assert_round_trips("\\f => \\g => \\x => f (g x)");
        let t = tree("\\f => \\g => \\x => f (g x)");
        assert_eq!(t.matches("LAMBDA_EXPR").count(), 3);
    }

    // ── M6 pattern tests ─────────────────────────────────────────────────────

    #[test]
    fn m6_pattern_tuple_variant_snapshot() {
        // Pin key structural nodes for a single-field tuple variant pattern.
        // (A raw-string snapshot would be terminated early by "#atom" tokens.)
        let src = "match x { (#just-value, value = v) => v; }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("MATCH_EXPR"));
        assert!(t.contains("MATCH_CASE"));
        assert!(t.contains("TUPLE_PATTERN"));
        assert!(t.contains("PATTERN_FIELD"));
        assert!(t.contains("FIELD_NAME"));
        assert_eq!(t.matches("MATCH_CASE").count(), 1);
        assert_eq!(t.matches("PATTERN_FIELD").count(), 1);
    }

    #[test]
    fn m6_pattern_wildcard() {
        assert_round_trips("match x { _ => 0; }");
        let t = tree("match x { _ => 0; }");
        assert!(t.contains("WILDCARD_PATTERN"));
    }

    #[test]
    fn m6_pattern_literal_keywords() {
        let src = "match x { none => 0; true => 1; false => 2; }";
        assert_round_trips(src);
        let t = tree(src);
        assert_eq!(t.matches("MATCH_CASE").count(), 3);
        assert_eq!(
            t.matches("LITERAL").count(),
            7,
            "scrutinee + 3 pattern literals + 3 body literals"
        );
    }

    #[test]
    fn m6_pattern_int_literal() {
        assert_round_trips("match x { 0 => #zero; 1 => #one; }");
        let t = tree("match x { 0 => #zero; 1 => #one; }");
        assert_eq!(t.matches("MATCH_CASE").count(), 2);
    }

    #[test]
    fn m6_pattern_negative_literal() {
        assert_round_trips("match x { -1 => #neg; 0 => #zero; }");
        let t = tree("match x { -1 => #neg; 0 => #zero; }");
        assert!(t.contains("MINUS"), "negative literal contains MINUS token");
        assert_eq!(t.matches("MATCH_CASE").count(), 2);
    }

    #[test]
    fn m6_pattern_binding() {
        assert_round_trips("match x { n => n; }");
        let t = tree("match x { n => n; }");
        assert!(t.contains("LITERAL"), "binding pattern is a LITERAL node");
    }

    #[test]
    fn m6_pattern_atom() {
        assert_round_trips("match x { #nothing => 0; }");
        let t = tree("match x { #nothing => 0; }");
        assert!(t.contains("LITERAL"), "atom pattern is a LITERAL node");
    }

    #[test]
    fn m6_pattern_parenthesized_atom() {
        assert_round_trips("match x { (#just) => 0; }");
        let t = tree("match x { (#just) => 0; }");
        assert!(t.contains("PAREN_PATTERN"));
        assert!(!t.contains("TUPLE_PATTERN"));
    }

    #[test]
    fn m6_pattern_tuple_empty_paren() {
        // `()` — empty tuple pattern.
        assert_round_trips("match x { () => 0; }");
        let t = tree("match x { () => 0; }");
        assert!(t.contains("TUPLE_PATTERN"));
    }

    #[test]
    fn m6_pattern_tuple_variant_hyphenated_field() {
        // Hyphenated field name in pattern field.
        let src = "match x { (#just-maybe, maybe-value = none) => -1; }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TUPLE_PATTERN"));
        assert!(
            t.contains("FIELD_NAME"),
            "hyphenated field name produces FIELD_NAME node"
        );
        assert!(t.contains("PATTERN_FIELD"));
    }

    #[test]
    fn m6_pattern_tuple_variant_multi_field() {
        let src = "match x { (#both-things, a = none, b = none) => -4; }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TUPLE_PATTERN"));
        assert_eq!(t.matches("PATTERN_FIELD").count(), 2);
    }

    #[test]
    fn m6_pattern_tuple_variant_wildcard_fields() {
        let src = "match x { (#both-things, a = _, b = _) => -5; }";
        assert_round_trips(src);
        let t = tree(src);
        assert_eq!(t.matches("WILDCARD_PATTERN").count(), 2);
        assert_eq!(t.matches("PATTERN_FIELD").count(), 2);
    }

    #[test]
    fn m6_pattern_tuple_variant_nested() {
        // Tuple variant inside a tuple field: (#ok, body = (#circle, radius = r))
        let src = "match x { (#ok, body = (#circle, radius = r)) => r; }";
        assert_round_trips(src);
        let t = tree(src);
        assert_eq!(
            t.matches("TUPLE_PATTERN").count(),
            2,
            "outer and inner tuple variant pattern"
        );
        assert_eq!(t.matches("PATTERN_FIELD").count(), 2);
    }

    #[test]
    fn m6_pattern_record() {
        // Record pattern: { profile = #prod; }
        let src = "match x { { profile = #prod; } => 1; { profile = _; } => 0; }";
        assert_round_trips(src);
        let t = tree(src);
        assert_eq!(t.matches("RECORD_PATTERN").count(), 2);
        assert_eq!(t.matches("PATTERN_FIELD").count(), 2);
    }

    #[test]
    fn m6_lambda_wildcard_param() {
        assert_round_trips("\\_ => 0");
        let t = tree("\\_ => 0");
        assert!(t.contains("LAMBDA_EXPR"));
        assert!(t.contains("WILDCARD_PATTERN"));
    }

    #[test]
    fn m6_lambda_multi_params() {
        assert_round_trips("\\a b => a + b");
        let t = tree("\\a b => a + b");
        assert!(t.contains("LAMBDA_EXPR"));
        assert!(t.contains("BINARY_EXPR"));
    }

    #[test]
    fn m6_unholy_match_patterns() {
        // All 12 patterns from unholy_match (cursed.zt:113-124) in a synthesized match.
        let src = concat!(
            "match u {\n",
            "  #just                             => 0;\n",
            "  (#just-value, value = v)          => v;\n",
            "  (#just-maybe, maybe-value = none) => -1;\n",
            "  (#just-maybe, maybe-value = v)    => v;\n",
            "  (#just-abyss, deep = none)        => -3;\n",
            "  (#just-abyss, deep = a)           => a;\n",
            "  (#both-things, a = none, b = none) => -4;\n",
            "  (#both-things, a = _, b = _)       => -5;\n",
            "  #nothing                          => -6;\n",
            "  none                              => -7;\n",
            "  true                              => 1;\n",
            "  false                             => 0;\n",
            "}"
        );
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("MATCH_EXPR"));
        assert_eq!(t.matches("MATCH_CASE").count(), 12);
        assert!(
            t.contains("TUPLE_PATTERN"),
            "tuple variant patterns present"
        );
        assert!(t.contains("WILDCARD_PATTERN"), "wildcard pattern present");
        assert!(t.contains("PATTERN_FIELD"), "pattern fields present");
        assert!(t.contains("FIELD_NAME"), "hyphenated field names present");
    }

    // ── M5 type expression tests ──────────────────────────────────────────────

    #[test]
    fn m5_type_record_snapshot() {
        // Pin exact tree shape for a simple type record.
        assert_round_trips("type { x : Int; }");
        let t = tree("type { x : Int; }");
        expect![[r#"
            FILE@0..17
              TYPE_FORM@0..17
                KW_TYPE@0..4 "type"
                TYPE_RECORD@4..17
                  WHITESPACE@4..5 " "
                  L_BRACE@5..6 "{"
                  TYPE_FIELD@6..15
                    FIELD_NAME@6..8
                      WHITESPACE@6..7 " "
                      IDENT@7..8 "x"
                    WHITESPACE@8..9 " "
                    COLON@9..10 ":"
                    LITERAL@10..14
                      WHITESPACE@10..11 " "
                      IDENT@11..14 "Int"
                    SEMI@14..15 ";"
                  WHITESPACE@15..16 " "
                  R_BRACE@16..17 "}"
        "#]]
        .assert_eq(&t);
    }

    #[test]
    fn m5_type_record_optional_field() {
        // `host? : Text` — `?` on the field name marks it as optional-presence.
        assert_round_trips("type { host? : Text; }");
        let t = tree("type { host? : Text; }");
        assert!(t.contains("TYPE_RECORD"));
        assert!(t.contains("TYPE_FIELD"));
        assert!(t.contains("FIELD_NAME"));
        assert!(t.contains("QUESTION"));
    }

    #[test]
    fn m5_type_record_hyphenated_field() {
        assert_round_trips("type { target-triple : Text; }");
        let t = tree("type { target-triple : Text; }");
        assert!(t.contains("TYPE_RECORD"));
        assert!(t.contains("FIELD_NAME"));
    }

    #[test]
    fn m5_type_record_empty() {
        assert_round_trips("type {}");
        let t = tree("type {}");
        assert!(t.contains("TYPE_FORM"));
        assert!(t.contains("TYPE_RECORD"));
    }

    #[test]
    fn m5_optional_type() {
        assert_round_trips("type { x : Int?; }");
        let t = tree("type { x : Int?; }");
        assert!(t.contains("OPTIONAL_TYPE"));
    }

    #[test]
    fn m5_double_optional_type() {
        // `Abyss??` — lexes as QUESTION_QUESTION, treated as double-optional in type position.
        assert_round_trips("type { deep : Abyss??; }");
        let t = tree("type { deep : Abyss??; }");
        assert!(t.contains("OPTIONAL_TYPE"));
        assert!(
            t.contains("QUESTION_QUESTION"),
            "expected ?? token in double-optional"
        );
    }

    #[test]
    fn m5_function_type_right_assoc() {
        // `Int -> Int -> Int` = `Int -> (Int -> Int)`.
        assert_round_trips("type { f : Int -> Int -> Int; }");
        let t = tree("type { f : Int -> Int -> Int; }");
        assert_eq!(
            t.matches("FUNCTION_TYPE").count(),
            2,
            "expected two FUNCTION_TYPE nodes for right-assoc ->"
        );
    }

    #[test]
    fn m5_paren_optional_fn_type() {
        // `(Int -> Bool)?` — parens group the fn type, then postfix `?`.
        assert_round_trips("type { f? : (Int -> Bool)?; }");
        let t = tree("type { f? : (Int -> Bool)?; }");
        assert!(t.contains("OPTIONAL_TYPE"));
        assert!(t.contains("FUNCTION_TYPE"));
        assert!(t.contains("PAREN_EXPR"), "paren group wraps fn type");
    }

    #[test]
    fn m5_type_union_empty() {
        assert_round_trips("type []");
        let t = tree("type []");
        assert!(t.contains("TYPE_FORM"));
        assert!(t.contains("TYPE_UNION"));
    }

    #[test]
    fn m5_type_union_singletons() {
        // Singleton union from the spec — reserved literals and atoms.
        let src = "type [ none; true; false; #none; #true; #false; ]";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TYPE_UNION"));
        assert_eq!(t.matches("TYPE_UNION_ITEM").count(), 6);
    }

    #[test]
    fn m5_type_union_tag_only_atom() {
        // Tag-only alternatives are bare atom singleton types.
        assert_round_trips("type [ #just; ]");
        let t = tree("type [ #just; ]");
        assert!(t.contains("TYPE_UNION"));
        assert!(!t.contains("TUPLE_EXPR"));
    }

    #[test]
    fn m5_type_union_tuple_variant_with_fields() {
        assert_round_trips("type [ (#just-value, value : Int); ]");
        let t = tree("type [ (#just-value, value : Int); ]");
        assert!(t.contains("TUPLE_EXPR"));
        assert!(t.contains("TYPE_TUPLE_FIELD"));
        assert!(t.contains("FIELD_NAME"));
    }

    #[test]
    fn m5_tuple_variant_type_multiple_fields() {
        assert_round_trips("type [ (#both, a : Bool, b : Int); ]");
        let t = tree("type [ (#both, a : Bool, b : Int); ]");
        assert_eq!(t.matches("TYPE_TUPLE_FIELD").count(), 2);
    }

    #[test]
    fn m5_inline_union_in_record_field() {
        // Inline union as the type of a record field (no outer `type` keyword).
        assert_round_trips("type { env : [#a; #b; #c;]; }");
        let t = tree("type { env : [#a; #b; #c;]; }");
        assert!(t.contains("TYPE_RECORD"));
        assert!(t.contains("TYPE_UNION"));
        assert_eq!(t.matches("TYPE_UNION_ITEM").count(), 3);
    }

    #[test]
    fn m5_optional_union_in_record_field() {
        // `[#x; #y; none;]?` — union literal with postfix optional.
        assert_round_trips("type { env? : [#x; #y; none;]?; }");
        let t = tree("type { env? : [#x; #y; none;]?; }");
        assert!(t.contains("OPTIONAL_TYPE"));
        assert!(t.contains("TYPE_UNION"));
    }

    #[test]
    fn m5_type_application() {
        // `List Int` in type position — reuses CALL_EXPR (no separate type-application node).
        assert_round_trips("type { xs : List Int; }");
        let t = tree("type { xs : List Int; }");
        assert!(
            t.contains("CALL_EXPR"),
            "type application produces CALL_EXPR"
        );
    }

    #[test]
    fn m5_tuple_type() {
        // `(A, B)` in type position — reuses TUPLE_EXPR.
        assert_round_trips("type { pair : (Bool, Int); }");
        let t = tree("type { pair : (Bool, Int); }");
        assert!(t.contains("TUPLE_EXPR"));
    }

    #[test]
    fn m5_nested_type_record() {
        // Nested `{ host : Text; }` inside a field (no repeated `type` keyword).
        assert_round_trips("type { server : { host : Text; port : Int; }; }");
        let t = tree("type { server : { host : Text; port : Int; }; }");
        assert_eq!(t.matches("TYPE_RECORD").count(), 2);
        assert_eq!(t.matches("TYPE_FIELD").count(), 3);
    }

    // ── M5 acceptance: cursed.zt type bodies ─────────────────────────────────

    #[test]
    fn m5_abyss_round_trip() {
        let src = "type {\n  into? : Abyss?;\n  depth : Int;\n}";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TYPE_RECORD"));
        assert!(t.contains("OPTIONAL_TYPE"));
        assert_eq!(t.matches("TYPE_FIELD").count(), 2);
    }

    #[test]
    fn m5_shadows_round_trip() {
        let src = "type [\n  none;\n  true;\n  false;\n  #none;\n  #true;\n  #false;\n]";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TYPE_UNION"));
        assert_eq!(t.matches("TYPE_UNION_ITEM").count(), 6);
    }

    #[test]
    fn m5_unholy_round_trip() {
        let src = concat!(
            "type [\n",
            "  #just;\n",
            "  (#just-value, value : Int);\n",
            "  (#just-maybe, maybe-value : Int?);\n",
            "  (#just-abyss, deep : Abyss??);\n",
            "  (#both-things, a : Bool, b : Bool);\n",
            "  #nothing;\n",
            "  none;\n",
            "  true;\n",
            "  false;\n",
            "]"
        );
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TYPE_UNION"));
        assert_eq!(
            t.matches("TUPLE_EXPR").count(),
            4,
            "expected 4 tuple variant items"
        );
        assert!(
            t.contains("OPTIONAL_TYPE"),
            "should have optional field types"
        );
    }

    #[test]
    fn m5_nightmare_record_round_trip() {
        let src = concat!(
            "type {\n",
            "  required           : Bool;\n",
            "  optional-value     : Bool?;\n",
            "  optional-field?    : Bool;\n",
            "  both-optional?     : Bool?;\n",
            "  deep-optional?     : Abyss??;\n",
            "  inline-union       : [#a; #b; #c;];\n",
            "  inline-fn          : Int -> Int -> Int;\n",
            "  optional-fn?       : (Int -> Bool)?;\n",
            "  optional-union?    : [#x; #y; none;]?;\n",
            "}"
        );
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TYPE_RECORD"));
        assert_eq!(t.matches("TYPE_FIELD").count(), 9);
        assert!(t.contains("TYPE_UNION"), "should have inline union types");
        assert_eq!(
            t.matches("FUNCTION_TYPE").count(),
            3,
            "inline-fn needs 2 FUNCTION_TYPE, optional-fn needs 1"
        );
        assert!(t.contains("OPTIONAL_TYPE"));
    }

    #[test]
    fn no_panic_round_trip() {
        for b in 0u8..=127u8 {
            if let Ok(s) = std::str::from_utf8(&[b]) {
                assert_round_trips(s);
            }
        }
        for pair in [[b' ', b'a'], [b'\n', b'b'], [b'a', b'\n']] {
            if let Ok(s) = std::str::from_utf8(&pair) {
                assert_round_trips(s);
            }
        }
    }

    // ── M7 top-level declaration tests ───────────────────────────────────────

    #[test]
    fn m7_inferred_binding() {
        let src = "x := 42";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("INFERRED_BINDING"));
        assert!(t.contains("LITERAL"));
    }

    #[test]
    fn m7_annotated_binding() {
        let src = "x : Int = 42";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("ANNOTATED_BINDING"));
    }

    #[test]
    fn m7_annotated_binding_complex_type() {
        let src = "items : List Int = []";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("ANNOTATED_BINDING"));
    }

    #[test]
    fn m7_type_definition_record() {
        let src = "Point :: type { x : Int; y : Int; }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert!(t.contains("TYPE_FORM"));
        assert!(t.contains("TYPE_RECORD"));
        assert_eq!(t.matches("TYPE_FIELD").count(), 2);
    }

    #[test]
    fn m7_type_definition_union() {
        let src = "Color :: type [ #red; #green; #blue; ]";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert!(t.contains("TYPE_FORM"));
        assert!(t.contains("TYPE_UNION"));
    }

    #[test]
    fn m7_func_decl_with_sig_single_clause() {
        let src = "id :: Int -> Int :: x { x }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert!(t.contains("FUNCTION_TYPE"));
        assert_eq!(t.matches("CLAUSE").count(), 1);
        assert!(t.contains("BLOCK"));
    }

    #[test]
    fn m7_func_decl_multiple_clauses() {
        let src =
            "classify :: Int -> [#neg; #zero; #pos;]\n         :: 0 { #zero }\n         :: n { n }";
        assert_round_trips(src);
        let t = tree(src);
        assert_eq!(t.matches("CLAUSE").count(), 2);
    }

    #[test]
    fn m7_func_decl_type_params() {
        let src = "id :: [A] A -> A :: x { x }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert!(t.contains("TYPE_PARAM_LIST"));
        assert_eq!(t.matches("CLAUSE").count(), 1);
    }

    #[test]
    fn m7_func_decl_multi_type_params() {
        let src = "const :: [A, B] A -> B -> A :: x -> _ { x }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("TYPE_PARAM_LIST"));
        assert_eq!(t.matches("CLAUSE").count(), 1);
    }

    #[test]
    fn m7_func_decl_multi_arg_clause() {
        let src = "flip :: [A, B, C] (A -> B -> C) -> B -> A -> C :: f -> b -> a { f a b }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert_eq!(t.matches("CLAUSE").count(), 1);
    }

    #[test]
    fn m7_func_decl_clauses_only() {
        // No type signature, clauses only (type inferred).
        let src = "succ :: n { n + 1 }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert_eq!(t.matches("CLAUSE").count(), 1);
        // No FUNCTION_TYPE since no signature.
        assert!(!t.contains("FUNCTION_TYPE"), "should have no type sig");
    }

    #[test]
    fn m7_func_decl_guard() {
        let src = "safe :: Int :: n if n > 0 { n }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("GUARD"));
        assert_eq!(t.matches("CLAUSE").count(), 1);
    }

    #[test]
    fn m7_multiple_decls() {
        let src = "x := 1\ny := 2\nz := x + y\nz";
        assert_round_trips(src);
        let t = tree(src);
        assert_eq!(t.matches("INFERRED_BINDING").count(), 3);
        assert!(t.contains("BINARY_EXPR"), "output expression present");
    }

    #[test]
    fn m7_block_with_local_bindings() {
        let src = "f :: Int :: n {\n  a := n + 1;\n  b := a * 2;\n  b\n}";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("FUNC_DECL"));
        assert!(t.contains("BLOCK"));
        assert_eq!(t.matches("LOCAL_BINDING").count(), 2);
    }

    #[test]
    fn m7_hyphenated_field_access_in_decl() {
        let src = "get :: Cfg :: cfg { cfg.target-triple }";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("ACCESS_EXPR"));
        assert!(t.contains("FIELD_NAME"), "hyphenated field name in access");
    }

    // M8: hyphenated field names in access position
    #[test]
    fn m8_field_access_hyphenated() {
        let src = "cfg.target-triple";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("ACCESS_EXPR"));
        assert!(t.contains("FIELD_NAME"));
    }

    #[test]
    fn m8_optional_field_access_hyphenated() {
        let src = "n?.next-hop";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("OPTIONAL_ACCESS_EXPR"));
        assert!(t.contains("FIELD_NAME"));
    }

    #[test]
    fn m8_field_vs_subtraction() {
        // With parens, `-` becomes subtraction, not field separator.
        let src = "(cfg.opt) - triple";
        assert_round_trips(src);
        let t = tree(src);
        assert!(t.contains("BINARY_EXPR"), "should be subtraction");
        // The field name inside the paren is plain (single IDENT)
        assert!(t.contains("ACCESS_EXPR"));
    }

    // ── M11 typed AST tests ───────────────────────────────────────────────────

    use ast::{AstNode, nodes::*};

    #[test]
    fn m11_ast_inferred_binding_name() {
        let p = parse("add5 := 42");
        let file = File::cast(p.syntax()).expect("File node");
        let decl = file.decls().next().expect("one decl");
        match decl {
            TopDecl::Inferred(b) => assert_eq!(b.name().as_deref(), Some("add5")),
            _ => panic!("expected inferred binding"),
        }
    }

    #[test]
    fn m11_ast_func_decl_clauses() {
        let p = parse("id :: Int -> Int :: x { x }");
        let file = File::cast(p.syntax()).expect("File node");
        let decl = file.decls().next().expect("one decl");
        match decl {
            TopDecl::Func(f) => {
                assert_eq!(f.name().as_deref(), Some("id"));
                assert_eq!(f.clauses().count(), 1);
            }
            _ => panic!("expected func decl"),
        }
    }

    #[test]
    fn m11_ast_func_decl_type_params() {
        let p = parse("const :: [A, B] A -> B -> A :: x -> _ { x }");
        let file = File::cast(p.syntax()).expect("File node");
        let decl = file.decls().next().expect("one decl");
        match decl {
            TopDecl::Func(f) => {
                let params = f.type_params().expect("type params");
                let names: Vec<_> = params.params().map(|t| t.text().to_owned()).collect();
                assert_eq!(names, ["A", "B"]);
            }
            _ => panic!("expected func decl"),
        }
    }

    #[test]
    fn m11_ast_field_name_text() {
        // FieldName::text() should concatenate IDENT-MINUS-IDENT.
        let p = parse("{ target-triple = \"x86\"; }");
        let root = p.syntax();
        let field_name_node = root
            .descendants()
            .find(|n| n.kind() == SyntaxKind::FIELD_NAME)
            .expect("FIELD_NAME node");
        let fname = ast::tokens::FieldName::cast(field_name_node).expect("FieldName");
        assert_eq!(fname.text(), "target-triple");
    }

    #[test]
    fn m11_ast_token_decoders() {
        // Int decode
        let p = parse("x := 42");
        let root = p.syntax();
        let int_tok = root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::INT)
            .expect("INT token");
        assert_eq!(ast::tokens::decode_int(&int_tok), Some(42));

        // Atom decode
        let p = parse("#just-value");
        let root = p.syntax();
        let atom_tok = root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::ATOM)
            .expect("ATOM token");
        assert_eq!(ast::tokens::decode_atom(&atom_tok), "just-value");
    }

    // ── M11 validation tests ──────────────────────────────────────────────────

    #[test]
    fn m11_validation_reserved_name_error() {
        // `forall` is reserved but lexes as IDENT → validation catches it.
        let p = parse("forall := 1");
        assert!(
            p.diagnostics.iter().any(|d| d.message.contains("reserved")),
            "expected reserved-name diagnostic, got: {:?}",
            p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn m11_validation_duplicate_binding_error() {
        let p = parse("x := 1\nx := 2\nx");
        assert!(
            p.diagnostics
                .iter()
                .any(|d| d.message.contains("duplicate")),
            "expected duplicate-binding diagnostic"
        );
    }

    #[test]
    fn m11_validation_duplicate_field_error() {
        let p = parse("{ a = 1; a = 2; }");
        assert!(
            p.diagnostics
                .iter()
                .any(|d| d.message.contains("duplicate field")),
            "expected duplicate-field diagnostic"
        );
    }

    #[test]
    fn m11_validation_type_def_capitalization_warn() {
        // Lowercase type definition name should trigger a capitalization warning.
        let p = parse("myType :: type { x : Int; }");
        assert!(
            p.diagnostics.iter().any(|d| {
                d.message.contains("uppercase") || d.message.contains("capitalization")
            }),
            "expected capitalization warning"
        );
    }

    // ── Comment / node comment / doc tests ───────────────────────────────────────

    #[test]
    fn node_comment_decl_excluded_from_typed_ast() {
        // `--/ z := "commented out"` should not appear in File::decls()
        let src = "--/ z := \"commented out\"\nw := 1";
        let p = parse(src);
        assert!(
            p.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            p.diagnostics
        );
        let file = ast::nodes::File::cast(p.syntax()).unwrap();
        let names: Vec<_> = file
            .decls()
            .filter_map(|d| ast::nodes::decl_name(&d))
            .map(|t| t.text().to_owned())
            .collect();
        assert_eq!(
            names,
            vec!["w"],
            "node comment decl should be excluded; got {names:?}"
        );
    }

    #[test]
    fn doc_comment_single_line() {
        let src = "--| The value.\nx := 1";
        let p = parse(src);
        assert!(p.diagnostics.is_empty());
        let file = ast::nodes::File::cast(p.syntax()).unwrap();
        let decl = file.decls().next().unwrap();
        assert_eq!(decl.doc().as_deref(), Some("The value."));
    }

    #[test]
    fn doc_comment_stacked_lines() {
        let src = "--| First line.\n--| Second line.\nx := 1";
        let p = parse(src);
        assert!(p.diagnostics.is_empty());
        let file = ast::nodes::File::cast(p.syntax()).unwrap();
        let decl = file.decls().next().unwrap();
        assert_eq!(decl.doc().as_deref(), Some("First line.\nSecond line."));
    }

    #[test]
    fn plain_comment_does_not_become_doc() {
        let src = "-- plain comment\nx := 1";
        let p = parse(src);
        assert!(p.diagnostics.is_empty());
        let file = ast::nodes::File::cast(p.syntax()).unwrap();
        let decl = file.decls().next().unwrap();
        assert_eq!(decl.doc(), None, "plain comment should not be a doc");
    }

    #[test]
    fn node_comment_record_field_excluded() {
        let src = "r := { --/ hidden = 1; visible = 2; }";
        let p = parse(src);
        assert!(p.diagnostics.is_empty(), "unexpected: {:?}", p.diagnostics);
        assert_round_trips(src);
        // The tree contains a NODE_COMMENT_NODE node
        assert!(tree(src).contains("NODE_COMMENT_NODE"));
        // The tree does NOT contain RECORD_EXPR > VALUE_FIELD for "hidden" at top level
        // (it's inside NODE_COMMENT_NODE, so semantic accessors skip it)
    }

    #[test]
    fn node_comment_list_item_excluded() {
        let src = "lst := [1; --/ 2; 3;]";
        let p = parse(src);
        assert!(p.diagnostics.is_empty(), "unexpected: {:?}", p.diagnostics);
        assert_round_trips(src);
        assert!(tree(src).contains("NODE_COMMENT_NODE"));
    }
}
