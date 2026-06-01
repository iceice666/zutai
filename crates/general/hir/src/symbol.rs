use text_size::TextRange;

use crate::arena::{Arena, Idx};

// ── SymbolId ──────────────────────────────────────────────────────────────────

pub type SymbolId = Idx<Symbol>;

/// Sentinel SymbolId used when name resolution fails.
/// Safe to use as an index only after checking `is_error`.
pub const ERROR_SYM: SymbolId = Idx {
    raw: u32::MAX,
    _phantom: std::marker::PhantomData,
};

impl SymbolId {
    pub fn is_error(self) -> bool {
        self.raw == u32::MAX
    }
}

// ── SymbolKind ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// Top-level `name := expr` binding.
    Value,
    /// Top-level `name :: clauses` function definition.
    Function,
    /// Top-level `Name :: type ...` type definition.
    TypeDef,
    /// A type parameter inside `[A, B]` on a function/type declaration.
    TypeParam,
    /// A block-local `:=` binding or lambda/match-arm binding pattern.
    Local,
}

// ── Symbol ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Source location of the definition token (for error labels).
    pub def_range: TextRange,
    /// Resolved semantic type (filled by M2 type-check pass; `None` until then).
    pub ty: Option<crate::ty::HirTyRef>,
}

// ── SymbolTable ───────────────────────────────────────────────────────────────

/// Flat arena of all symbols in a file. Indexed by `SymbolId`.
#[derive(Debug)]
pub struct SymbolTable {
    syms: Arena<Symbol>,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        Self { syms: Arena::new() }
    }

    pub fn alloc(&mut self, sym: Symbol) -> SymbolId {
        self.syms.alloc(sym)
    }

    pub fn get(&self, id: SymbolId) -> &Symbol {
        self.syms.get(id)
    }

    pub fn get_mut(&mut self, id: SymbolId) -> &mut Symbol {
        self.syms.get_mut(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (SymbolId, &Symbol)> {
        self.syms.iter()
    }
}
