use text_size::TextRange;
use zutai_syntax::diag::{Diagnostic, ErrorCode};

use crate::arena::Arena;
use crate::decl::{HirDecl, HirDeclId};
use crate::expr::{HirExpr, HirExprId, HirExprKind};
use crate::file::HirFile;
use crate::pat::{HirPat, HirPatId, HirPatKind};
use crate::scope::{ScopeId, ScopeStack};
use crate::symbol::{ERROR_SYM, Symbol, SymbolId, SymbolKind, SymbolTable};
use crate::ty::{HirType, HirTypeId, HirTypeKind};

// ── LowerCtx ──────────────────────────────────────────────────────────────────

/// Mutable state accumulated during lowering of a single file.
pub(crate) struct LowerCtx {
    pub exprs: Arena<HirExpr>,
    pub pats: Arena<HirPat>,
    pub types: Arena<HirType>,
    pub decls: Arena<HirDecl>,
    pub symbols: SymbolTable,
    pub scopes: ScopeStack,
    pub diagnostics: Vec<Diagnostic>,
    pub top_decls: Vec<HirDeclId>,
    pub final_expr: Option<HirExprId>,
}

impl LowerCtx {
    pub fn new() -> Self {
        Self {
            exprs: Arena::new(),
            pats: Arena::new(),
            types: Arena::new(),
            decls: Arena::new(),
            symbols: SymbolTable::new(),
            scopes: ScopeStack::new(),
            diagnostics: Vec::new(),
            top_decls: Vec::new(),
            final_expr: None,
        }
    }

    /// Allocate a new symbol and register its name in the current scope.
    ///
    /// Does NOT re-emit E0010 — `validation.rs` already catches top-level
    /// duplicate names. For duplicate locals, callers may emit their own diagnostic.
    /// The new SymbolId is always allocated regardless of collision.
    pub fn define_sym(&mut self, name: String, kind: SymbolKind, def_range: TextRange) -> SymbolId {
        let sym_id = self.symbols.alloc(Symbol {
            name: name.clone(),
            kind,
            def_range,
            ty: None,
        });
        // Silently ignore scope collisions: the first definition wins for lookups.
        let _ = self.scopes.define(name, sym_id);
        sym_id
    }

    /// Like `define_sym` but registers into a specific scope (used for the top-level
    /// mutual-recursion pre-population phase).
    pub fn define_sym_in(
        &mut self,
        scope_id: ScopeId,
        name: String,
        kind: SymbolKind,
        def_range: TextRange,
    ) -> SymbolId {
        let sym_id = self.symbols.alloc(Symbol {
            name: name.clone(),
            kind,
            def_range,
            ty: None,
        });
        let _ = self.scopes.define_in(scope_id, name, sym_id);
        sym_id
    }

    /// Resolve a name, emitting E0020 on failure and returning `ERROR_SYM`.
    pub fn resolve_name(&mut self, name: &str, use_range: TextRange) -> SymbolId {
        if let Some(id) = self.scopes.resolve(name) {
            id
        } else {
            self.diagnostics.push(Diagnostic::error(
                use_range,
                ErrorCode::UnknownIdentifier,
                format!("unknown identifier `{name}`"),
            ));
            ERROR_SYM
        }
    }

    // ── Alloc helpers ─────────────────────────────────────────────────────────

    pub fn alloc_expr(&mut self, kind: HirExprKind, range: TextRange) -> HirExprId {
        self.exprs.alloc(HirExpr { kind, range })
    }

    pub fn error_expr(&mut self, range: TextRange) -> HirExprId {
        self.alloc_expr(HirExprKind::Error, range)
    }

    pub fn alloc_pat(&mut self, kind: crate::pat::HirPatKind, range: TextRange) -> HirPatId {
        self.pats.alloc(HirPat { kind, range })
    }

    pub fn error_pat(&mut self, range: TextRange) -> HirPatId {
        self.alloc_pat(HirPatKind::Error, range)
    }

    pub fn alloc_type(&mut self, kind: HirTypeKind, range: TextRange) -> HirTypeId {
        self.types.alloc(HirType { kind, range })
    }

    pub fn error_type(&mut self, range: TextRange) -> HirTypeId {
        self.alloc_type(HirTypeKind::Error, range)
    }

    pub fn alloc_decl(&mut self, decl: HirDecl) -> HirDeclId {
        self.decls.alloc(decl)
    }

    // ── Finalise ──────────────────────────────────────────────────────────────

    pub fn into_file(self, final_expr: HirExprId) -> (HirFile, Vec<Diagnostic>) {
        let file = HirFile {
            decls: self.top_decls,
            final_expr,
            exprs: self.exprs,
            pats: self.pats,
            types: self.types,
            decls_arena: self.decls,
            symbols: self.symbols,
        };
        (file, self.diagnostics)
    }
}
