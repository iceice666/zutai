//! Syntax support for Zutai general mode (`.zt`): lossless CST with error recovery.

pub mod lexer;
mod parser;
mod syntax_kind;

pub use syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, ZutaiLanguage};

/// A parse error recorded at a byte offset in the source.
///
/// This is a minimal placeholder; the full `diag/` module (M10) will supersede it with
/// severity codes, multi-span labels, and ariadne rendering.
pub struct SyntaxError {
    pub message: String,
    pub offset: text_size::TextSize,
}

/// The result of parsing a `.zt` source file.
pub struct Parse {
    pub green: rowan::GreenNode,
    pub diagnostics: Vec<SyntaxError>,
}

impl Parse {
    /// Root syntax node for tree navigation.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// Parse a `.zt` source string into a lossless green tree.
pub fn parse(src: &str) -> Parse {
    let (green, diagnostics) = parser::parse(src);
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
            include_str!("../../fixtures/cursed.zt"),
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
        // Variant constructor: (#tag, field = value)
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
    fn m5_type_union_variant_tag_only() {
        // `(#just)` — variant with tag and no fields.
        assert_round_trips("type [ (#just); ]");
        let t = tree("type [ (#just); ]");
        assert!(t.contains("TYPE_UNION"));
        assert!(t.contains("VARIANT_TYPE"));
    }

    #[test]
    fn m5_type_union_variant_with_fields() {
        assert_round_trips("type [ (#just-value, value : Int); ]");
        let t = tree("type [ (#just-value, value : Int); ]");
        assert!(t.contains("VARIANT_TYPE"));
        assert!(t.contains("VARIANT_FIELD"));
        assert!(t.contains("FIELD_NAME"));
    }

    #[test]
    fn m5_variant_type_multiple_fields() {
        assert_round_trips("type [ (#both, a : Bool, b : Int); ]");
        let t = tree("type [ (#both, a : Bool, b : Int); ]");
        assert_eq!(t.matches("VARIANT_FIELD").count(), 2);
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
            "  (#just);\n",
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
            t.matches("VARIANT_TYPE").count(),
            5,
            "expected 5 variant items"
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
}
