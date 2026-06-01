mod ctx;
mod decl;
mod expr;
mod pat;
mod ty;

use zutai_syntax::SyntaxNode;
use zutai_syntax::diag::Diagnostic;

use crate::file::HirFile;

pub(crate) use ctx::LowerCtx;

// Re-export the LitClass classifier (adapted from zutai-semantic::ast_ext)
pub(crate) use classify::LitClass;
pub(crate) use classify::classify_literal;

/// Lower a parsed `.zt` syntax tree to an HIR file.
///
/// Returns the HIR and any lowering diagnostics (currently: E0020 unknown identifier).
pub fn lower_file(root: &SyntaxNode) -> (HirFile, Vec<Diagnostic>) {
    let mut ctx = LowerCtx::new();
    decl::lower_file_decls(&mut ctx, root);
    let final_expr = ctx.final_expr.unwrap_or_else(|| {
        let range = root.text_range();
        ctx.error_expr(range)
    });
    ctx.into_file(final_expr)
}

// ── LitClass classifier ───────────────────────────────────────────────────────
//
// Adapted from zutai-semantic::ast_ext. Kept here to avoid a circular dependency
// (hir → semantic would be circular since semantic → hir).

mod classify {
    use zutai_syntax::{SyntaxKind, SyntaxNode};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum LitClass {
        NameRef,
        Wildcard,
        Int,
        Float,
        Str,
        Atom,
        Bool,
        NoneLit,
    }

    pub(crate) fn classify_literal(node: &SyntaxNode) -> Option<LitClass> {
        if node.kind() != SyntaxKind::LITERAL {
            return None;
        }
        let mut tokens = node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.kind().is_trivia());
        let first = tokens.next()?;
        match first.kind() {
            SyntaxKind::IDENT => Some(LitClass::NameRef),
            SyntaxKind::UNDERSCORE => Some(LitClass::Wildcard),
            SyntaxKind::INT => Some(LitClass::Int),
            SyntaxKind::FLOAT => Some(LitClass::Float),
            SyntaxKind::STRING => Some(LitClass::Str),
            SyntaxKind::ATOM => Some(LitClass::Atom),
            SyntaxKind::KW_TRUE | SyntaxKind::KW_FALSE => Some(LitClass::Bool),
            SyntaxKind::KW_NONE => Some(LitClass::NoneLit),
            SyntaxKind::MINUS => match tokens.next().map(|t| t.kind()) {
                Some(SyntaxKind::INT) => Some(LitClass::Int),
                Some(SyntaxKind::FLOAT) => Some(LitClass::Float),
                _ => None,
            },
            _ => None,
        }
    }
}
