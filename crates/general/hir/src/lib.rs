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
    HirHandleClause, HirHandleOp, HirImportSource, HirLevel, HirLocalBinding, HirPat, HirPatId,
    HirPatKind, HirRecordField, HirRecordPatField, HirRowTail, HirRowTailKind, HirSelectField,
    HirTupleItem, HirTuplePatItem, HirTypeExpr, HirTypeId, HirTypeKind, HirTypeRecordField,
    HirTypeTupleItem, HirUnionVariant,
};
pub use lower::{HirLowerOptions, LoweredHir, lower_file, lower_file_with_options};
pub use pass::{
    HirPass, HirPassReport, StructuralKeyValidationPass, run_default_passes, run_passes,
};
pub use zutai_stdlib::{
    CMP_MODULE_SRC, CONFIG_MODULE_SRC, DATA_MODULE_SRC, FS_MODULE_SRC, LIST_MODULE_SRC,
    NUM_MODULE_SRC, OPTIONAL_MODULE_SRC, PRELUDE_MODULE_SRC, REFLECT_MODULE_SRC, RESULT_MODULE_SRC,
    STREAM_MODULE_SRC, TEXT_MODULE_SRC, VALIDATE_MODULE_SRC,
};
pub use zutai_syntax::numlit::NumberType;
