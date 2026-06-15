mod ir;
mod lower;

#[cfg(test)]
mod tests;

pub use ir::{
    BuiltinOp, Literal, PrimTy, TlcAlt, TlcDecl, TlcDeclId, TlcExpr, TlcExprId, TlcModule, TlcPat,
    TlcPatItem, TlcRecordField, TlcTupleField, TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};
pub use lower::lower_thir;
