mod ir;
mod lower;
mod normalize;

#[cfg(test)]
mod tests;

pub use ir::{
    BuiltinOp, Kind, Literal, PrimTy, Row, TlcAlt, TlcDecl, TlcDeclId, TlcExpr, TlcExprId,
    TlcModule, TlcPat, TlcPatItem, TlcTupleField, TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};
pub use lower::lower_thir;
pub use normalize::{DEFAULT_FUEL, NormalizeError};
