use crate::SyntaxKind;

/// A compact set of [`SyntaxKind`] values represented as a 128-bit bitmask.
///
/// Only kinds with discriminant < 128 are supported; the complete `SyntaxKind`
/// enum has 98 variants (0..=97), so the full set fits comfortably.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSet(u128);

impl TokenSet {
    pub(crate) const EMPTY: Self = Self(0);

    pub(crate) const fn new(kinds: &[SyntaxKind]) -> Self {
        let mut bits = 0u128;
        let mut i = 0;
        while i < kinds.len() {
            bits |= 1u128 << (kinds[i] as u16);
            i += 1;
        }
        Self(bits)
    }

    pub(crate) fn contains(self, kind: SyntaxKind) -> bool {
        let bit = kind as u16;
        if bit >= 128 {
            return false;
        }
        (self.0 >> bit) & 1 == 1
    }

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// ── Common recovery sets ──────────────────────────────────────────────────────

/// Tokens that delimit the end of a statement or nested construct.
pub(crate) const STMT_RECOVERY: TokenSet = TokenSet::new(&[
    SyntaxKind::SEMI,
    SyntaxKind::R_BRACE,
    SyntaxKind::R_BRACK,
    SyntaxKind::R_PAREN,
    SyntaxKind::EOF,
]);

/// Recovery tokens inside a match arm (skip to `=>`, `;`, `}`, or EOF).
pub(crate) const MATCH_CASE_RECOVERY: TokenSet = TokenSet::new(&[
    SyntaxKind::FAT_ARROW,
    SyntaxKind::SEMI,
    SyntaxKind::R_BRACE,
    SyntaxKind::EOF,
]);

/// Recovery tokens for a clause body (`{` signals the start of the block).
pub(crate) const CLAUSE_RECOVERY: TokenSet = TokenSet::new(&[
    SyntaxKind::L_BRACE,
    SyntaxKind::COLON_COLON,
    SyntaxKind::EOF,
]);
