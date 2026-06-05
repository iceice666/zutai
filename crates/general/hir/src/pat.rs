use text_size::TextRange;

use crate::arena::Idx;
use crate::symbol::SymbolId;
use crate::ty::LitVal;

// ── HirPat identifiers ────────────────────────────────────────────────────────

pub type HirPatId = Idx<HirPat>;

// ── HirPat ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HirPat {
    pub kind: HirPatKind,
    pub range: TextRange,
}

#[derive(Debug, Clone)]
pub enum HirPatKind {
    /// `_`
    Wildcard,
    /// Identifier in pattern position — introduces a binding.
    Bind(SymbolId),
    /// Literal value pattern: `none`, `true`, `42`, `"hello"`, `#ok`.
    Literal(LitVal),
    /// Parenthesized pattern; semantically identical to its inner pattern.
    Paren(HirPatId),
    /// Closed record pattern: `{ field = pat; ... }`.
    Record { fields: Vec<(String, HirPatId)> },
    /// Tuple pattern: positional items and optional named fields.
    Tuple { items: Vec<HirTuplePatElem> },
    /// Placeholder when pattern lowering fails.
    Error,
}

#[derive(Debug, Clone)]
pub enum HirTuplePatElem {
    Positional(HirPatId),
    Named(String, HirPatId),
}
