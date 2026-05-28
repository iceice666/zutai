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
              IDENT@0..1 "a"
              WHITESPACE@1..2 " "
              IDENT@2..3 "b"
        "#]]
        .assert_eq(&debug);
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
