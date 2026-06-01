use crate::arena::Arena;
use crate::decl::{HirDecl, HirDeclId};
use crate::expr::{HirExpr, HirExprId};
use crate::pat::HirPat;
use crate::symbol::SymbolTable;
use crate::ty::HirType;

/// The lowered representation of a single `.zt` source file.
///
/// All top-level declarations are in a single mutual-recursion group.
/// The `final_expr` is the file's output value.
pub struct HirFile {
    /// Top-level declarations (implicitly mutually recursive).
    pub decls: Vec<HirDeclId>,
    /// The file's output expression (the trailing expression, not a declaration).
    pub final_expr: HirExprId,

    // ── Arenas ────────────────────────────────────────────────────────────────
    pub exprs: Arena<HirExpr>,
    pub pats: Arena<HirPat>,
    pub types: Arena<HirType>,
    pub decls_arena: Arena<HirDecl>,

    /// All symbols defined in this file, indexed by `SymbolId`.
    pub symbols: SymbolTable,
}
