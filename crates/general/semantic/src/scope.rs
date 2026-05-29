use rustc_hash::FxHashMap;
use text_size::TextRange;

use crate::ty::TyId;

// ── Identifiers ───────────────────────────────────────────────────────────────

/// Index into the `ScopeStack` arena. Stable for the lifetime of an analysis run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

// ── Symbol ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// A runtime value binding (`x := expr` or `x : T = expr`).
    Value,
    /// A function definition (`f :: ...`).
    Function,
    /// A type definition (`T :: type ...`).
    TypeDef,
    /// A type parameter inside `[A, B]` on a function/type declaration.
    TypeParam,
    /// A block-local binding (`x := expr` inside a block body).
    Local,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Source location of the definition token (for error labels).
    pub def: TextRange,
    /// Resolved type, if known. `None` until the type-checking pass runs.
    pub ty: Option<TyId>,
}

// ── Scope ─────────────────────────────────────────────────────────────────────

pub struct Scope {
    pub parent: Option<ScopeId>,
    pub symbols: FxHashMap<String, Symbol>,
}

// ── ScopeStack ────────────────────────────────────────────────────────────────

/// Arena-allocated scope stack. All scopes live in a flat `Vec`; parent links
/// are `ScopeId` indices, so there are no lifetime entanglements.
pub struct ScopeStack {
    arena: Vec<Scope>,
    current: ScopeId,
}

impl ScopeStack {
    /// Create the stack with an empty root scope.
    pub fn new() -> Self {
        Self {
            arena: vec![Scope {
                parent: None,
                symbols: FxHashMap::default(),
            }],
            current: ScopeId(0),
        }
    }

    pub fn current_id(&self) -> ScopeId {
        self.current
    }

    /// Open a new child scope, making it the current scope. Returns its id.
    pub fn push_child(&mut self) -> ScopeId {
        let id = ScopeId(self.arena.len() as u32);
        self.arena.push(Scope {
            parent: Some(self.current),
            symbols: FxHashMap::default(),
        });
        self.current = id;
        id
    }

    /// Close the current scope, restoring the parent. Panics if already at root.
    pub fn pop(&mut self) {
        let parent = self.arena[self.current.0 as usize]
            .parent
            .expect("pop() called on root scope");
        self.current = parent;
    }

    /// Define `sym` in the current scope.
    ///
    /// Returns `Err(prior_def)` if the name is already defined in the *same*
    /// scope (the caller should emit a duplicate-binding diagnostic). Does NOT
    /// shadow up the parent chain — that is intentional for the one-namespace rule.
    pub fn define(&mut self, sym: Symbol) -> Result<(), TextRange> {
        let scope = &mut self.arena[self.current.0 as usize];
        if let Some(prior) = scope.symbols.get(&sym.name) {
            return Err(prior.def);
        }
        scope.symbols.insert(sym.name.clone(), sym);
        Ok(())
    }

    /// Look up `name` starting from the current scope and walking up parent links.
    pub fn resolve(&self, name: &str) -> Option<&Symbol> {
        let mut id = self.current;
        loop {
            let scope = &self.arena[id.0 as usize];
            if let Some(sym) = scope.symbols.get(name) {
                return Some(sym);
            }
            id = scope.parent?;
        }
    }

    /// Look up `name` in a specific scope (used for top-level pre-population).
    pub fn resolve_in(&self, scope_id: ScopeId, name: &str) -> Option<&Symbol> {
        self.arena[scope_id.0 as usize].symbols.get(name)
    }
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self::new()
    }
}
