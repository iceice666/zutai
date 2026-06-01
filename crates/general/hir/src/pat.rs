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
    /// Closed record pattern: `{ field = pat; ... }`.
    Record { fields: Vec<(String, HirPatId)> },
    /// Tagged variant pattern: `(#tag, field = pat, ...)`.
    /// Kept distinct from Record (see plan §"Variant vs Record").
    Variant {
        tag: String,
        fields: Vec<(String, HirPatId)>,
    },
    /// Placeholder when pattern lowering fails.
    Error,
}
