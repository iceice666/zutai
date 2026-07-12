//! Resolved high-level IR for Zutai general mode (`.zt`).
//!
//! HIR is the first semantic representation after the parser AST. It resolves
//! lexical names to bindings, normalizes declaration shape, and removes purely
//! syntactic sugar such as pipelines. Type-dependent elaboration belongs in
//! THIR, not here.

pub mod diagnostic;
pub mod ir;
pub mod lower;
pub mod pass;

#[cfg(test)]
mod tests;

pub use diagnostic::{HirDiagnostic, HirDiagnosticKind};
pub use ir::{
    BUILTIN_VALUE_NAMES, Binding, BindingId, BindingKind, HirClause, HirDecl, HirDeclId,
    HirDeclKind, HirEffectOp, HirEffectRow, HirExpr, HirExprId, HirExprKind, HirFile,
    HirHandleClause, HirHandleOp, HirImportSource, HirLevel, HirListItem, HirLocalBinding, HirPat,
    HirPatId, HirPatKind, HirRecordField, HirRecordItem, HirRecordPatField, HirRowSpread,
    HirRowSpreadKind, HirRowTail, HirRowTailKind, HirSelectField, HirTupleItem, HirTuplePatItem,
    HirTypeExpr, HirTypeId, HirTypeKind, HirTypeRecordField, HirTypeTupleItem, HirUnionVariant,
    HirValueSpread,
};
pub use lower::{
    HirLowerOptions, LoweredHir, SourcePreludes, lower_file, lower_file_with_options,
    lower_file_with_preludes,
};
pub use pass::{
    HirPass, HirPassReport, StructuralKeyValidationPass, run_default_passes, run_passes,
};
pub use zutai_syntax::numlit::NumberType;
