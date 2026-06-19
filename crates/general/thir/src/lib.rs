//! Typed high-level IR for Zutai general mode (`.zt`).
//!
//! THIR is the planned output of type checking and elaboration. It is distinct
//! from HIR so that HIR remains useful when type checking fails and THIR can
//! lower type-dependent sugar such as optional access and defaulting.

pub mod diagnostic;
pub mod export;
pub mod import;
pub mod ir;
pub mod lower;
pub mod pass;

#[cfg(test)]
mod tests;

pub use diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
pub use export::{ExportUnsupported, export_type};
pub use import::{ImportKey, ImportedField, ImportedTupleItem, ImportedType};
pub use ir::{
    RowTail, ThirClause, ThirConstraintMethod, ThirDecl, ThirDeclId, ThirDeclKind, ThirExpr,
    ThirExprId, ThirExprKind, ThirFile, ThirLocalBinding, ThirPat, ThirPatId, ThirPatKind,
    ThirRecordField, ThirRecordPatField, ThirTupleItem, ThirTuplePatItem, ThirWitnessField, Type,
    TypeId, TypeKind, TypeRecordField, TypeTupleItem,
};
pub use lower::{LoweredThir, ThirLowerOptions, lower_hir, lower_hir_with_options};
pub use pass::{ThirPass, ThirPassReport, run_default_passes, run_passes};
