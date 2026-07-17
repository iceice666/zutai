use crate::{Diagnostic, SyntaxKind, parse, tokenize};

const INDENT: &str = "  ";

/// Format a general-mode source file without rewriting its syntax.
///
/// The formatter keeps every non-trivia token and comment byte-for-byte. It
/// normalizes line endings, leading indentation from delimiter depth, trailing
/// whitespace, and the final newline. Keeping newlines and token spellings
/// intact preserves top-level application boundaries and compatibility forms.
pub fn format_source(input: &str) -> Result<String, Vec<Diagnostic>> {
    let parsed = parse(input);
    let blocking: Vec<_> = parsed
        .diagnostics()
        .iter()
        .filter(|diagnostic| !is_compatibility_diagnostic(diagnostic))
        .cloned()
        .collect();
    if !blocking.is_empty() || parsed.ast().is_none() {
        return Err(if blocking.is_empty() {
            parsed.diagnostics().to_vec()
        } else {
            blocking
        });
    }

    Ok(format_tokens(input))
}

fn is_compatibility_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(diagnostic.kind, crate::ParseErrorKind::LambdaArrow)
}

fn format_tokens(input: &str) -> String {
    let tokens = tokenize(input);
    let mut output = String::with_capacity(input.len().saturating_add(1));
    let mut depth = 0usize;
    let mut at_line_start = true;
    let mut last_content_end = 0usize;

    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            SyntaxKind::Whitespace => {
                if at_line_start
                    || tokens
                        .get(index + 1)
                        .is_some_and(|next| next.kind == SyntaxKind::Newline)
                {
                    continue;
                }
                push_normalized_text(&mut output, token.text);
            }
            SyntaxKind::Newline => {
                output.push('\n');
                at_line_start = true;
            }
            kind => {
                if at_line_start {
                    let line_depth = if is_closing(kind) {
                        depth.saturating_sub(1)
                    } else {
                        depth
                    };
                    for _ in 0..line_depth {
                        output.push_str(INDENT);
                    }
                }

                if is_closing(kind) {
                    depth = depth.saturating_sub(1);
                }
                if matches!(
                    kind,
                    SyntaxKind::LineComment | SyntaxKind::DocComment | SyntaxKind::BlockComment
                ) {
                    push_normalized_text(&mut output, token.text);
                } else {
                    output.push_str(token.text);
                }
                if is_opening(kind) {
                    depth += 1;
                }

                at_line_start = token.text.ends_with('\n');
                last_content_end = output.len();
            }
        }
    }

    output.truncate(last_content_end);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn push_normalized_text(output: &mut String, text: &str) {
    for segment in text.split('\r') {
        output.push_str(segment);
    }
}

fn is_opening(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LBrace | SyntaxKind::LBracket | SyntaxKind::LParen
    )
}

fn is_closing(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::RBrace | SyntaxKind::RBracket | SyntaxKind::RParen
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn significant_tokens(source: &str) -> Vec<(SyntaxKind, &str)> {
        tokenize(source)
            .into_iter()
            .filter(|token| {
                !matches!(
                    token.kind,
                    SyntaxKind::Whitespace
                        | SyntaxKind::Newline
                        | SyntaxKind::LineComment
                        | SyntaxKind::DocComment
                        | SyntaxKind::BlockComment
                )
            })
            .map(|token| (token.kind, token.text))
            .collect()
    }

    fn comments(source: &str) -> Vec<(SyntaxKind, String)> {
        tokenize(source)
            .into_iter()
            .filter(|token| {
                matches!(
                    token.kind,
                    SyntaxKind::LineComment | SyntaxKind::DocComment | SyntaxKind::BlockComment
                )
            })
            .map(|token| (token.kind, token.text.replace('\r', "")))
            .collect()
    }

    #[test]
    fn format_indents_delimiters_and_is_idempotent() {
        let source = "value ::= {\na = {\n1;\n};\n};\nvalue";
        let expected = "value ::= {\n  a = {\n    1;\n  };\n};\nvalue\n";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn format_preserves_comments_tokens_newlines_and_compatibility_spellings() {
        let source = concat!(
            "--| docs\r\n",
            "choice ::= if true then { -- keep\n",
            "1;\n",
            "} else { --[ nested --[ block ]--\r\n",
            "comment ]--\n",
            "2;\n",
            "};\n",
            "apply ::= \\x => x;\n",
            "choice\n",
        );
        let formatted = format_source(source).unwrap();
        assert_eq!(significant_tokens(&formatted), significant_tokens(source));
        assert!(!formatted.contains('\r'));
        assert_eq!(comments(&formatted), comments(source));
        assert_eq!(formatted.lines().count(), source.lines().count());
        assert!(formatted.contains("if true then"));
        assert!(formatted.contains("\\x => x"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        assert!(parse(&formatted).ast().is_some());
    }

    #[test]
    fn format_rejects_invalid_source() {
        let diagnostics = format_source("value ::= ;\nvalue").unwrap_err();
        assert!(!diagnostics.is_empty());
    }
}
