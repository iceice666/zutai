use rustc_hash::FxHashMap;

use crate::symbol::SymbolId;

// ── ScopeId ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

// ── Scope ─────────────────────────────────────────────────────────────────────

pub struct Scope {
    pub parent: Option<ScopeId>,
    names: FxHashMap<String, SymbolId>,
}

// ── ScopeStack ────────────────────────────────────────────────────────────────

/// Arena-allocated scope stack for name resolution during lowering.
///
/// Names map to `SymbolId`s. The `SymbolTable` holds the actual `Symbol` data.
pub struct ScopeStack {
    arena: Vec<Scope>,
    current: ScopeId,
}

impl ScopeStack {
    pub fn new() -> Self {
        Self {
            arena: vec![Scope {
                parent: None,
                names: FxHashMap::default(),
            }],
            current: ScopeId(0),
        }
    }

    pub fn current_id(&self) -> ScopeId {
        self.current
    }

    /// Open a new child scope, making it the current scope.
    pub fn push_child(&mut self) -> ScopeId {
        let id = ScopeId(self.arena.len() as u32);
        self.arena.push(Scope {
            parent: Some(self.current),
            names: FxHashMap::default(),
        });
        self.current = id;
        id
    }

    /// Close the current scope, restoring its parent.
    pub fn pop(&mut self) {
        let parent = self.arena[self.current.0 as usize]
            .parent
            .expect("pop() on root scope");
        self.current = parent;
    }

    /// Restore to a specific scope (for multi-clause function lowering).
    pub fn restore(&mut self, id: ScopeId) {
        self.current = id;
    }

    /// Register `name → sym_id` in the current scope.
    ///
    /// Returns `Err(prior_id)` if the name is already defined in this scope.
    /// The caller decides whether to emit a diagnostic.
    pub fn define(&mut self, name: String, sym_id: SymbolId) -> Result<(), SymbolId> {
        let scope = &mut self.arena[self.current.0 as usize];
        if let Some(&prior) = scope.names.get(&name) {
            return Err(prior);
        }
        scope.names.insert(name, sym_id);
        Ok(())
    }

    /// Look up `name` starting from the current scope, walking parent links.
    pub fn resolve(&self, name: &str) -> Option<SymbolId> {
        let mut id = self.current;
        loop {
            let scope = &self.arena[id.0 as usize];
            if let Some(&sym_id) = scope.names.get(name) {
                return Some(sym_id);
            }
            id = scope.parent?;
        }
    }

    /// Define in a specific (already-pushed) scope (used for top-level pre-population).
    pub fn define_in(
        &mut self,
        scope_id: ScopeId,
        name: String,
        sym_id: SymbolId,
    ) -> Result<(), SymbolId> {
        let scope = &mut self.arena[scope_id.0 as usize];
        if let Some(&prior) = scope.names.get(&name) {
            return Err(prior);
        }
        scope.names.insert(name, sym_id);
        Ok(())
    }
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self::new()
    }
}
