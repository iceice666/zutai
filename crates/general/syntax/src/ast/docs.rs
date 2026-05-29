use crate::{SyntaxKind, SyntaxNode};

/// Collect the doc-comment block for `node` by scanning the leading trivia of
/// its first non-trivia child for consecutive `DOC_COMMENT` tokens.
///
/// Each `--|` line's prefix (`--|` plus one optional space) is stripped; the
/// resulting lines are joined with `\n`. Returns `None` when no doc comment is
/// present. The body is a Markdown subset — consumers are responsible for
/// rendering it.
pub fn doc_block(node: &SyntaxNode) -> Option<String> {
    // Collect DOC_COMMENT tokens that appear before the first non-trivia child.
    let mut lines: Vec<String> = Vec::new();

    for elem in node.children_with_tokens() {
        match elem {
            rowan::NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::DOC_COMMENT => {
                let raw = tok.text();
                // Strip the `--|` marker (2 dashes + pipe) and one optional space.
                let body = raw
                    .strip_prefix("--|")
                    .unwrap_or(raw)
                    .strip_prefix(' ')
                    .unwrap_or_else(|| raw.strip_prefix("--|").unwrap_or(raw));
                lines.push(body.to_owned());
            }
            rowan::NodeOrToken::Token(tok) if tok.kind().is_trivia() => {
                // WHITESPACE between doc lines — skip, keep accumulating.
            }
            _ => break, // first non-trivia element reached
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}
