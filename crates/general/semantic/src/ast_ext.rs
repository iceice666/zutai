//! Helpers that bridge the gaps between the CST and semantic intent.
//!
//! The `zutai-syntax` CST is lossless but not 1:1 with semantics. The most
//! important gap: **there is no `NameRef` or `Wildcard` node**. A variable
//! reference, `_`, `42`, `"x"`, `true`, `none`, and `#ok` are all
//! `LITERAL` nodes. Semantic passes must classify them by looking at the
//! inner token. This module centralises that classifier so every pass
//! shares the same logic.
//!
//! ## Other CST gaps (do NOT forget these in passes)
//!
//! * **Type vs. expression position** is not encoded in the tree.
//!   `List Int` is a `CALL_EXPR`; `(A, B)` in a type annotation is a
//!   `TUPLE_EXPR`. Reconstruct position from the parent node kind:
//!   type child of `ANNOTATED_BINDING`, `TYPE_FIELD`, `VARIANT_FIELD`,
//!   or the RHS of `FUNCTION_TYPE`.
//!
//! * **Patterns overlap `LITERAL`** in clause/match-arm position. A binding
//!   pattern (`n`), atom pattern (`#ok`), and literal pattern (`0`, `true`)
//!   are all `LITERAL` nodes. Only `WILDCARD_PATTERN`, `TUPLE_PATTERN`, and
//!   `RECORD_PATTERN` are distinct node kinds. A `NameRef`-classified `LITERAL`
//!   inside a pattern *introduces* a name; the same kind in expression position
//!   *references* one.
//!
//! * **Type definitions are `FUNC_DECL` nodes** with a `TYPE_FORM` child.
//!   Check `node.children().any(|c| c.kind() == SyntaxKind::TYPE_FORM)` to
//!   distinguish a type def from a function def.
//!
//! * **`_tag` is reserved/implicit** for tagged-union desugaring (§17.5).
//!   `(#circle, radius : Float)` desugars to `{ _tag : #circle; radius : Float; }`.
//!   Users must never write `_tag` explicitly; the `_tag` structural check pass
//!   (M4) enforces this.

use zutai_syntax::{SyntaxKind, SyntaxNode};

// ── LitClass ──────────────────────────────────────────────────────────────────

/// The semantic class of a `LITERAL` CST node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LitClass {
    /// `IDENT` child — a variable reference or (in pattern position) a binding.
    NameRef,
    /// `UNDERSCORE` child — wildcard (`_`).
    Wildcard,
    /// `INT` child (or `MINUS INT` for a negative literal).
    Int,
    /// `FLOAT` child (or `MINUS FLOAT` for a negative literal).
    Float,
    /// `STRING` child.
    Str,
    /// `ATOM` child (`#name`).
    Atom,
    /// `KW_TRUE` or `KW_FALSE` child.
    Bool,
    /// `KW_NONE` child.
    NoneLit,
}

/// Classify a `LITERAL` CST node into its semantic kind.
///
/// Returns `None` if `node` is not a `LITERAL` or if its token structure is
/// unrecognised (e.g. an `ERROR` token inside a recovery node).
pub fn classify_literal(node: &SyntaxNode) -> Option<LitClass> {
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
        // Negative literal: MINUS immediately followed by a number (no trivia
        // between them — enforced by the parser's raw-adjacency check).
        SyntaxKind::MINUS => match tokens.next().map(|t| t.kind()) {
            Some(SyntaxKind::INT) => Some(LitClass::Int),
            Some(SyntaxKind::FLOAT) => Some(LitClass::Float),
            _ => None,
        },
        _ => None,
    }
}
