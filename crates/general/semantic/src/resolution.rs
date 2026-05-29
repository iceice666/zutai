use rustc_hash::FxHashMap;
use text_size::TextRange;

// ── ResolutionMap ─────────────────────────────────────────────────────────────

/// Maps each resolved use-site (the `TextRange` of a `NameRef` token) to the
/// `TextRange` of the definition it refers to.
///
/// Populated by the name-resolution pass (M1). Consumed by the type-checking
/// pass (M2) and any tooling that wants "go to definition" / hover information.
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
