use crate::{SyntaxKind, SyntaxToken};

// ── Typed token wrappers ──────────────────────────────────────────────────────

/// A `FIELD_NAME` token — one or more `IDENT (MINUS IDENT)*` leaves.
///
/// The node itself is a `FIELD_NAME` *composite node* (not a single leaf token),
/// so this type wraps the `SyntaxNode`, not `SyntaxToken`.
use crate::SyntaxNode;

pub struct FieldName(SyntaxNode);

impl FieldName {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SyntaxKind::FIELD_NAME {
            Some(Self(node))
        } else {
            None
        }
    }

    /// The concatenated text of the field name (e.g. `"target-triple"`).
    ///
    /// Skips trivia (whitespace) to return only the identifier text.
    pub fn text(&self) -> String {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia())
            .map(|t| t.text().to_owned())
            .collect()
    }
}

// ── Literal value decoders ────────────────────────────────────────────────────

/// Decode the integer value from an `INT` token.
pub fn decode_int(tok: &SyntaxToken) -> Option<i64> {
    debug_assert_eq!(tok.kind(), SyntaxKind::INT);
    tok.text().parse().ok()
}

/// Decode the float value from a `FLOAT` token.
pub fn decode_float(tok: &SyntaxToken) -> Option<f64> {
    debug_assert_eq!(tok.kind(), SyntaxKind::FLOAT);
    tok.text().parse().ok()
}

/// Decode a JSON-style string value from a `STRING` token (strips quotes, handles escapes).
///
/// Returns `None` if the token is malformed (e.g. unterminated string from lexer ERROR).
pub fn decode_string(tok: &SyntaxToken) -> Option<String> {
    debug_assert_eq!(tok.kind(), SyntaxKind::STRING);
    let text = tok.text();
    // Strip enclosing `"` characters.
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    // Simple JSON-escape processing.
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\x08'),
                'f' => out.push('\x0C'),
                'u' => {
                    // 4-hex-digit Unicode escape.
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    let ch = char::from_u32(code)?;
                    out.push(ch);
                }
                _ => return None, // invalid escape
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Return the atom body (without the `#` prefix) from an `ATOM` token.
pub fn decode_atom(tok: &SyntaxToken) -> &str {
    debug_assert_eq!(tok.kind(), SyntaxKind::ATOM);
    tok.text().strip_prefix('#').unwrap_or(tok.text())
}
