mod classify;
mod cursor;
mod scalars;

use classify::{is_atom_continue, is_ident_continue, is_ident_start, is_whitespace, keyword_kind};
use cursor::Cursor;
use scalars::{scan_number, scan_string};
pub(crate) use scalars::{validate_number, validate_string};

use crate::SyntaxKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    /// Byte length in the source string.
    pub len: u32,
}

/// Tokenize `src` into a lossless token sequence. All bytes are covered:
/// the sum of `token.len` for all returned tokens equals `src.len()`.
/// The lexer never fails; unrecognised runs produce [`SyntaxKind::ERROR`] tokens.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut cursor = Cursor::new(src);
    let mut tokens = Vec::new();

    while !cursor.is_eof() {
        let start = cursor.pos();
        let kind = next_token(&mut cursor);
        let len = (cursor.pos() - start) as u32;
        debug_assert!(len > 0, "next_token must always advance the cursor");
        tokens.push(Token { kind, len });
    }

    tokens
}

fn next_token(cursor: &mut Cursor<'_>) -> SyntaxKind {
    match cursor.peek().unwrap() {
        b if is_whitespace(b) => {
            cursor.eat_while(is_whitespace);
            SyntaxKind::WHITESPACE
        }

        b'"' => {
            scan_string(cursor);
            SyntaxKind::STRING
        }

        b'#' => scan_atom(cursor),

        b if is_ident_start(b) => scan_ident(cursor),

        b'0'..=b'9' => scan_number(cursor),

        b':' => match cursor.peek_at(1) {
            Some(b':') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::COLON_COLON
            }
            Some(b'=') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::COLON_EQ
            }
            _ => {
                cursor.bump();
                SyntaxKind::COLON
            }
        },

        b'=' => match cursor.peek_at(1) {
            Some(b'=') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::EQ_EQ
            }
            Some(b'>') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::FAT_ARROW
            }
            _ => {
                cursor.bump();
                SyntaxKind::EQ
            }
        },

        b'!' => match cursor.peek_at(1) {
            Some(b'=') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::BANG_EQ
            }
            _ => {
                cursor.bump();
                SyntaxKind::ERROR
            }
        },

        b'-' => match cursor.peek_at(1) {
            Some(b'-') => scan_comment(cursor),
            Some(b'>') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::ARROW
            }
            _ => {
                cursor.bump();
                SyntaxKind::MINUS
            }
        },

        b'?' => match cursor.peek_at(1) {
            Some(b'.') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::OPTIONAL_DOT
            }
            Some(b'?') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::QUESTION_QUESTION
            }
            _ => {
                cursor.bump();
                SyntaxKind::QUESTION
            }
        },

        b'<' => match cursor.peek_at(1) {
            Some(b'=') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::LT_EQ
            }
            Some(b'|') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::ARROW_PIPE
            }
            _ => {
                cursor.bump();
                SyntaxKind::LT
            }
        },

        b'>' => match cursor.peek_at(1) {
            Some(b'=') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::GT_EQ
            }
            _ => {
                cursor.bump();
                SyntaxKind::GT
            }
        },

        b'|' => match cursor.peek_at(1) {
            Some(b'>') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::PIPE_ARROW
            }
            Some(b'|') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::PIPE_PIPE
            }
            _ => {
                cursor.bump();
                SyntaxKind::ERROR
            }
        },

        b'&' => match cursor.peek_at(1) {
            Some(b'&') => {
                cursor.bump();
                cursor.bump();
                SyntaxKind::AMP_AMP
            }
            _ => {
                cursor.bump();
                SyntaxKind::ERROR
            }
        },

        b'.' => {
            if cursor.peek_at(1) == Some(b'.') && cursor.peek_at(2) == Some(b'.') {
                cursor.bump();
                cursor.bump();
                cursor.bump();
                SyntaxKind::ELLIPSIS
            } else {
                cursor.bump();
                SyntaxKind::DOT
            }
        }

        b'\\' => {
            cursor.bump();
            SyntaxKind::BACKSLASH
        }
        b'+' => {
            cursor.bump();
            SyntaxKind::PLUS
        }
        b'*' => {
            cursor.bump();
            SyntaxKind::STAR
        }
        b'/' => {
            cursor.bump();
            SyntaxKind::SLASH
        }
        b';' => {
            cursor.bump();
            SyntaxKind::SEMI
        }
        b',' => {
            cursor.bump();
            SyntaxKind::COMMA
        }
        b'{' => {
            cursor.bump();
            SyntaxKind::L_BRACE
        }
        b'}' => {
            cursor.bump();
            SyntaxKind::R_BRACE
        }
        b'[' => {
            cursor.bump();
            SyntaxKind::L_BRACK
        }
        b']' => {
            cursor.bump();
            SyntaxKind::R_BRACK
        }
        b'(' => {
            cursor.bump();
            SyntaxKind::L_PAREN
        }
        b')' => {
            cursor.bump();
            SyntaxKind::R_PAREN
        }

        _ => {
            // Unrecognised byte or non-ASCII outside a string: advance one code point.
            cursor.bump_char();
            SyntaxKind::ERROR
        }
    }
}

/// Scan a comment starting at the first `-` (caller has verified peek(0)=peek(1)=`-`).
///
/// Dispatch on the character immediately after `--`:
///   `/`  → 3-char `--/` NODE_COMMENT marker (does not consume to EOL).
///   `{`  → nestable block comment `--{ … }--`; returns COMMENT.
///   `|`  → doc-comment to end of line; returns DOC_COMMENT.
///   else → plain line comment to end of line; returns COMMENT.
///
/// Note: a line comment whose text starts with `/`, `|`, or `{` must use a
/// leading space to avoid being parsed as node comment / doc / block (`-- /path`).
fn scan_comment(cursor: &mut Cursor<'_>) -> SyntaxKind {
    debug_assert_eq!(cursor.peek(), Some(b'-'));
    debug_assert_eq!(cursor.peek_at(1), Some(b'-'));
    cursor.bump(); // first `-`
    cursor.bump(); // second `-`

    match cursor.peek() {
        Some(b'/') => {
            cursor.bump();
            SyntaxKind::NODE_COMMENT
        }
        Some(b'{') => {
            scan_block_comment(cursor);
            SyntaxKind::COMMENT
        }
        Some(b'|') => {
            // consume to end of line (leave the newline for WHITESPACE)
            cursor.eat_while(|b| b != b'\n');
            SyntaxKind::DOC_COMMENT
        }
        _ => {
            cursor.eat_while(|b| b != b'\n');
            SyntaxKind::COMMENT
        }
    }
}

/// Scan a nestable block comment. Cursor is at `{` (the char after `--`).
///
/// Opening: `--{`  Closing: `}--`  Nesting: fully nestable.
/// On unterminated input (EOF before matching `}--`), consumes all remaining
/// bytes and returns (lossless; the caller emits COMMENT; diagnostics deferred).
fn scan_block_comment(cursor: &mut Cursor<'_>) {
    debug_assert_eq!(cursor.peek(), Some(b'{'));
    cursor.bump(); // opening `{`

    let mut depth: usize = 1;
    loop {
        match cursor.peek() {
            None => return, // unterminated — lossless, diagnostic deferred
            Some(b'-') if cursor.peek_at(1) == Some(b'-') && cursor.peek_at(2) == Some(b'{') => {
                cursor.bump();
                cursor.bump();
                cursor.bump();
                depth += 1;
            }
            Some(b'}') if cursor.peek_at(1) == Some(b'-') && cursor.peek_at(2) == Some(b'-') => {
                cursor.bump();
                cursor.bump();
                cursor.bump();
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            Some(_) => {
                cursor.bump_char();
            }
        }
    }
}

fn scan_ident(cursor: &mut Cursor<'_>) -> SyntaxKind {
    let start = cursor.pos();
    cursor.bump(); // start char already verified by caller
    cursor.eat_while(is_ident_continue);
    keyword_kind(cursor.slice(start))
}

fn scan_atom(cursor: &mut Cursor<'_>) -> SyntaxKind {
    debug_assert_eq!(cursor.peek(), Some(b'#'));
    cursor.bump(); // `#`
    match cursor.peek() {
        Some(b) if is_ident_start(b) => {
            cursor.bump();
            cursor.eat_while(is_atom_continue);
            SyntaxKind::ATOM
        }
        // `#` not followed by a valid atom start: emit ERROR for the `#` alone.
        _ => SyntaxKind::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SyntaxKind::*;

    fn lex(src: &str) -> Vec<(SyntaxKind, &str)> {
        let tokens = tokenize(src);
        let mut pos = 0usize;
        tokens
            .iter()
            .map(|t| {
                let end = pos + t.len as usize;
                let text = &src[pos..end];
                pos = end;
                (t.kind, text)
            })
            .collect()
    }

    fn kinds(src: &str) -> Vec<SyntaxKind> {
        lex(src).into_iter().map(|(k, _)| k).collect()
    }

    fn assert_lossless(src: &str) {
        let total: u32 = tokenize(src).iter().map(|t| t.len).sum();
        assert_eq!(
            total as usize,
            src.len(),
            "token lengths do not cover source"
        );
    }

    #[test]
    fn empty() {
        assert_eq!(tokenize(""), vec![]);
    }

    #[test]
    fn whitespace_collapses() {
        assert_eq!(kinds("  \t\n  "), vec![WHITESPACE]);
    }

    #[test]
    fn ident_and_underscore() {
        assert_eq!(lex("foo"), vec![(IDENT, "foo")]);
        assert_eq!(lex("_foo"), vec![(IDENT, "_foo")]);
        assert_eq!(lex("_"), vec![(UNDERSCORE, "_")]);
    }

    #[test]
    fn keywords() {
        let cases: &[(&str, SyntaxKind)] = &[
            ("type", KW_TYPE),
            ("match", KW_MATCH),
            ("if", KW_IF),
            ("then", KW_THEN),
            ("else", KW_ELSE),
            ("import", KW_IMPORT),
            ("true", KW_TRUE),
            ("false", KW_FALSE),
            ("none", KW_NONE),
            ("select", KW_SELECT),
        ];
        for (kw, expected) in cases {
            assert_eq!(kinds(kw), vec![*expected], "keyword: {kw}");
        }
    }

    #[test]
    fn keyword_prefix_is_ident() {
        // "types" should be IDENT, not KW_TYPE
        assert_eq!(kinds("types"), vec![IDENT]);
        assert_eq!(kinds("matches"), vec![IDENT]);
    }

    #[test]
    fn atoms() {
        assert_eq!(lex("#prod"), vec![(ATOM, "#prod")]);
        assert_eq!(lex("#x86_64-linux"), vec![(ATOM, "#x86_64-linux")]);
        assert_eq!(lex("#none"), vec![(ATOM, "#none")]);
        assert_eq!(lex("#true"), vec![(ATOM, "#true")]);
    }

    #[test]
    fn bare_hash_is_error() {
        assert_eq!(kinds("#"), vec![ERROR]);
        assert_eq!(kinds("# "), vec![ERROR, WHITESPACE]);
    }

    #[test]
    fn integers() {
        assert_eq!(kinds("0"), vec![INT]);
        assert_eq!(kinds("42"), vec![INT]);
        assert_eq!(kinds("1000"), vec![INT]);
    }

    #[test]
    fn floats() {
        assert_eq!(kinds("3.14"), vec![FLOAT]);
        assert_eq!(kinds("1e9"), vec![FLOAT]);
        assert_eq!(kinds("2.5e-3"), vec![FLOAT]);
        assert_eq!(kinds("1E+10"), vec![FLOAT]);
    }

    #[test]
    fn minus_is_separate_from_number() {
        // `-1` is MINUS + INT (parser handles negative literals)
        assert_eq!(kinds("-1"), vec![MINUS, INT]);
        assert_eq!(kinds("-3.14"), vec![MINUS, FLOAT]);
    }

    #[test]
    fn number_dot_field() {
        // `1.foo` → INT DOT IDENT  (not FLOAT)
        assert_eq!(kinds("1.foo"), vec![INT, DOT, IDENT]);
    }

    #[test]
    fn strings() {
        assert_eq!(kinds(r#""hello""#), vec![STRING]);
        assert_eq!(kinds(r#""a\nb""#), vec![STRING]);
        assert_eq!(kinds(r#""unicode A""#), vec![STRING]);
    }

    #[test]
    fn operators_maximal_munch() {
        let cases: &[(&str, SyntaxKind)] = &[
            ("::", COLON_COLON),
            (":=", COLON_EQ),
            (":", COLON),
            ("==", EQ_EQ),
            ("=>", FAT_ARROW),
            ("=", EQ),
            ("!=", BANG_EQ),
            ("->", ARROW),
            ("-", MINUS),
            ("?.", OPTIONAL_DOT),
            ("??", QUESTION_QUESTION),
            ("?", QUESTION),
            ("<=", LT_EQ),
            ("<|", ARROW_PIPE),
            ("<", LT),
            (">=", GT_EQ),
            (">", GT),
            ("|>", PIPE_ARROW),
            ("||", PIPE_PIPE),
            ("&&", AMP_AMP),
            ("...", ELLIPSIS),
            (".", DOT),
            ("\\", BACKSLASH),
            ("+", PLUS),
            ("*", STAR),
            ("/", SLASH),
            (";", SEMI),
            (",", COMMA),
            ("{", L_BRACE),
            ("}", R_BRACE),
            ("[", L_BRACK),
            ("]", R_BRACK),
            ("(", L_PAREN),
            (")", R_PAREN),
        ];
        for (src, expected) in cases {
            assert_eq!(kinds(src), vec![*expected], "operator: {src:?}");
        }
    }

    #[test]
    fn error_for_lone_punctuation() {
        assert_eq!(kinds("!"), vec![ERROR]);
        assert_eq!(kinds("|"), vec![ERROR]);
        assert_eq!(kinds("&"), vec![ERROR]);
    }

    #[test]
    fn colon_colon_then_eq() {
        // `::=` is COLON_COLON followed by EQ, not COLON_EQ
        assert_eq!(kinds("::="), vec![COLON_COLON, EQ]);
    }

    #[test]
    fn trivia_between_tokens() {
        assert_eq!(kinds("a b"), vec![IDENT, WHITESPACE, IDENT]);
        assert_eq!(
            kinds("42 + 1"),
            vec![INT, WHITESPACE, PLUS, WHITESPACE, INT]
        );
    }

    #[test]
    fn lossless_cursed_zt() {
        let src = include_str!("../../../fixtures/cursed.zt");
        assert_lossless(src);
    }

    #[test]
    fn lossless_final_boss_fixtures() {
        let fixtures = [
            include_str!("../../../fixtures/valid/deep_nesting.zt"),
            include_str!("../../../fixtures/valid/optional_chains.zt"),
            include_str!("../../../fixtures/valid/lexical_torture.zt"),
            include_str!("../../../fixtures/valid/comments.zt"),
            include_str!("../../../fixtures/invalid/sigil_swaps.zt"),
            include_str!("../../../fixtures/invalid/separator_swaps.zt"),
            include_str!("../../../fixtures/invalid/comparison_chaining.zt"),
            include_str!("../../../fixtures/invalid/pipeline_ambiguity.zt"),
            include_str!("../../../fixtures/invalid/keyword_misuse.zt"),
            include_str!("../../../fixtures/invalid/no_unary_operator.zt"),
            include_str!("../../../fixtures/invalid/atom_and_comment_traps.zt"),
            include_str!("../../../fixtures/invalid/string_number_lexical.zt"),
        ];
        for src in fixtures {
            assert_lossless(src);
        }
    }

    #[test]
    fn no_panic_on_arbitrary_ascii() {
        // The lexer must not panic on any input.
        for b in 0u8..=127 {
            let buf = [b];
            let s = std::str::from_utf8(&buf).unwrap_or("");
            if !s.is_empty() {
                assert_lossless(s);
            }
        }
    }

    // ── Comment lexing ────────────────────────────────────────────────────────

    #[test]
    fn line_comment() {
        assert_eq!(kinds("-- hello world"), vec![COMMENT]);
        assert_eq!(kinds("-- trailing"), vec![COMMENT]);
        // newline is separate WHITESPACE, not consumed by the comment
        assert_eq!(kinds("-- foo\nx"), vec![COMMENT, WHITESPACE, IDENT]);
    }

    #[test]
    fn doc_comment() {
        assert_eq!(kinds("--| hello"), vec![DOC_COMMENT]);
        assert_eq!(
            kinds("--| line1\n--| line2"),
            vec![DOC_COMMENT, WHITESPACE, DOC_COMMENT]
        );
    }

    #[test]
    fn node_comment_three_chars() {
        // --/ is exactly 3 chars; what follows is a separate token
        assert_eq!(kinds("--/"), vec![NODE_COMMENT]);
        assert_eq!(kinds("--/ x"), vec![NODE_COMMENT, WHITESPACE, IDENT]);
        assert_lossless("--/ x := 42");
    }

    #[test]
    fn block_comment() {
        assert_eq!(kinds("--{ body }--"), vec![COMMENT]);
        assert_lossless("--{ body }--");
    }

    #[test]
    fn block_comment_nested() {
        assert_eq!(kinds("--{ outer --{ inner }-- outer }--"), vec![COMMENT]);
        assert_lossless("--{ outer --{ inner }-- outer }--");
    }

    #[test]
    fn block_comment_unterminated_is_lossless() {
        // Unterminated block comment must not panic and must cover all bytes.
        assert_lossless("--{ never closed");
        assert_eq!(kinds("--{ never closed"), vec![COMMENT]);
    }

    #[test]
    fn comment_does_not_steal_arrow() {
        // `->` must still lex as ARROW, not a comment followed by `>`
        assert_eq!(kinds("->"), vec![ARROW]);
        assert_eq!(
            kinds("a -> b"),
            vec![IDENT, WHITESPACE, ARROW, WHITESPACE, IDENT]
        );
    }

    #[test]
    fn disambiguation_plain_line_with_slash() {
        // A line comment starting with a space + slash is NOT node comment
        assert_eq!(kinds("-- /usr/bin"), vec![COMMENT]);
        assert_lossless("-- /usr/bin");
    }

    #[test]
    fn node_comment_vs_comment() {
        // --/ (no space) is node comment; -- / (space) is a line comment
        assert_eq!(kinds("--/"), vec![NODE_COMMENT]);
        assert_eq!(kinds("-- /"), vec![COMMENT]);
    }
}
