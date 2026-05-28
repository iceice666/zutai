use crate::{SyntaxKind, lexer::Token};

/// Trivia-skipping view over the raw token sequence produced by the lexer.
///
/// Trivia (whitespace, comments) are filtered out; each non-trivia token has its
/// original index into the raw `Vec<Token>` recorded so the builder can re-attach
/// trivia and the parser can query raw adjacency (M3 negative literal, M8 field names).
pub(crate) struct Tokens {
    /// Non-trivia kinds, in order.
    kinds: Vec<SyntaxKind>,
    /// Logical index → index in the original raw token vec.
    raw_index: Vec<usize>,
}

impl Tokens {
    pub(crate) fn from_raw(raw: &[Token]) -> Self {
        let mut kinds = Vec::new();
        let mut raw_index = Vec::new();
        for (i, tok) in raw.iter().enumerate() {
            if !tok.kind.is_trivia() {
                kinds.push(tok.kind);
                raw_index.push(i);
            }
        }
        Self { kinds, raw_index }
    }

    /// Kind at logical position `pos`; returns `EOF` when past the end.
    pub(crate) fn kind(&self, pos: usize) -> SyntaxKind {
        self.kinds.get(pos).copied().unwrap_or(SyntaxKind::EOF)
    }

    pub(crate) fn len(&self) -> usize {
        self.kinds.len()
    }

    /// True when logical tokens `pos` and `pos+1` are *raw-adjacent* — no trivia between them.
    /// Returns `false` when `pos+1` is out of bounds.
    pub(crate) fn is_raw_adjacent(&self, pos: usize) -> bool {
        match (self.raw_index.get(pos), self.raw_index.get(pos + 1)) {
            (Some(&a), Some(&b)) => b == a + 1,
            _ => false,
        }
    }
}
