mod erase;
mod ir;
mod lower;
mod normalize;

#[cfg(test)]
mod tests;

pub use ir::{
    BuiltinOp, Kind, Literal, PrimTy, Row, TlcAlt, TlcDecl, TlcDeclId, TlcExpr, TlcExprId,
    TlcHandleClause, TlcModule, TlcPat, TlcPatItem, TlcTupleField, TlcTupleItem, TlcType,
    TlcTypeId, TlcTypeVar,
};
pub use lower::lower_thir;
pub use normalize::{DEFAULT_FUEL, NormalizeError};

/// Return why a TLC module still cannot enter Dataflow Core.
///
/// Phase 16 keeps DC/ANF/SSA pure. Handled effects are executable in the TLC
/// reference evaluator, but residual effect nodes or non-empty function effect
/// rows must not be silently erased before compilation.
pub fn residual_effect_reason(module: &TlcModule) -> Option<&'static str> {
    if module.expr_arena.iter().any(|(_, expr)| {
        matches!(
            expr,
            TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. }
        )
    }) {
        return Some(
            "algebraic effects remain after TLC lowering; compile/dataflow effect lowering is not implemented yet",
        );
    }

    if module.type_arena.iter().any(|(_, ty)| {
        matches!(
            ty,
            TlcType::Fun(_, _, row) if !matches!(row, Row::REmpty)
        )
    }) {
        return Some(
            "effectful function types remain after TLC lowering; compile/dataflow effect lowering is not implemented yet",
        );
    }

    None
}
