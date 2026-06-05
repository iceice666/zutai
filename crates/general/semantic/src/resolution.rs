use rustc_hash::FxHashMap;
use text_size::TextRange;

// ── ResolutionMap ─────────────────────────────────────────────────────────────

/// Maps each resolved use-site (the `TextRange` of a `NameRef` token) to the
/// `TextRange` of the definition it refers to.
///
/// This is retained for editor/tooling consumers that want "go to definition"
/// information keyed by source ranges. The active semantic pipeline resolves
/// names during `zutai_hir` lowering and type checking reads `HirExprKind::Var`
/// plus the HIR `SymbolTable`; this map is not currently populated by
/// `analyze`.
#[derive(Default)]
pub struct ResolutionMap {
    refs: FxHashMap<TextRange, TextRange>,
}

impl ResolutionMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the identifier at `use_site` resolves to the binding at `def_site`.
    pub fn insert(&mut self, use_site: TextRange, def_site: TextRange) {
        self.refs.insert(use_site, def_site);
    }

    /// Look up what `use_site` resolves to.
    pub fn get(&self, use_site: TextRange) -> Option<TextRange> {
        self.refs.get(&use_site).copied()
    }

    pub fn len(&self) -> usize {
        self.refs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }
}
