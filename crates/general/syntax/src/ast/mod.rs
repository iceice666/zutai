pub mod nodes;
pub mod operators;
pub mod tokens;

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

// ── AstNode trait ─────────────────────────────────────────────────────────────

/// A typed wrapper over a green-tree `SyntaxNode`.
pub trait AstNode: Sized {
    fn can_cast(kind: SyntaxKind) -> bool;
    fn cast(node: SyntaxNode) -> Option<Self>;
    fn syntax(&self) -> &SyntaxNode;
}

// ── Support helpers ───────────────────────────────────────────────────────────

pub mod support {
    use super::{AstNode, SyntaxKind, SyntaxNode, SyntaxToken};

    /// First child node that casts to `N`.
    pub fn child<N: AstNode>(parent: &SyntaxNode) -> Option<N> {
        parent.children().find_map(N::cast)
    }

    /// All child nodes that cast to `N`.
    pub fn children<'a, N: AstNode + 'a>(parent: &'a SyntaxNode) -> impl Iterator<Item = N> + 'a {
        parent.children().filter_map(N::cast)
    }

    /// First child token of the given kind.
    pub fn token(parent: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
        parent
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == kind)
    }
}
