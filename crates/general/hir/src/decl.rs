use crate::arena::Idx;
use crate::expr::HirExprId;
use crate::symbol::SymbolId;
use crate::ty::HirTypeId;

pub type HirDeclId = Idx<HirDecl>;

/// A top-level declaration in a `.zt` file.
///
/// All top-level declarations form a single implicit mutual-recursion group
/// (per spec §5.6): every name is in scope within every declaration body.
/// There is no explicit `LetRec` HIR node.
#[derive(Debug, Clone)]
pub enum HirDecl {
    /// `name := expr` or `name : T = expr` — inferred or annotated value binding.
    Value {
        name: SymbolId,
        ty: Option<HirTypeId>,
        body: HirExprId,
    },
    /// `name :: [A,B] (Sig ::)? clause+` — function definition.
    /// Multi-clause functions are lowered to a single `Lambda` with nested
    /// `Match` arms (see plan §"Multi-clause lowering").
    Function {
        name: SymbolId,
        type_params: Vec<SymbolId>,
        sig: Option<HirTypeId>,
        body: HirExprId,
    },
    /// `Name :: type { ... }` or `Name :: type [ ... ]` — type alias or definition.
    TypeDef {
        name: SymbolId,
        type_params: Vec<SymbolId>,
        body: HirTypeId,
    },
}
