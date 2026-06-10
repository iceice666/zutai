//! Resolved-import descriptors threaded into THIR lowering.
//!
//! THIR lowering is pure (no filesystem).  The semantic layer resolves each
//! `import` expression to a structural type descriptor and passes a map keyed
//! by the import source; THIR interns each descriptor into its own type arena
//! when it lowers the corresponding `Import` node.  Keeping the descriptor
//! independent of THIR's private `type_arena` is what lets the resolver live
//! outside this crate.

use zutai_hir::HirImportSource;

/// Key identifying a resolved import within a single file's analysis.
///
/// Equal import sources resolve to the same module, so the source itself is the
/// natural key.  It is already the payload of [`crate::ThirExprKind::Import`].
pub type ImportKey = HirImportSource;

/// Structural type of an imported module's value, independent of THIR's
/// internal type arena.
///
/// For `.zti` imports it mirrors the shape of the immediate data; for `.zt`
/// module imports it is the structurally-exported type of the module's final
/// expression (see [`crate::export_type`]).  Only self-contained data shapes
/// are representable — functions and type values cannot cross a module boundary
/// in the current evaluator and are refused before they reach here.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportedType {
    Bool,
    Int,
    Float,
    Text,
    Atom(String),
    List(Box<ImportedType>),
    Optional(Box<ImportedType>),
    Record(Vec<ImportedField>),
    Tuple(Vec<ImportedTupleItem>),
    Union(Vec<ImportedType>),
    /// Element type of an empty imported list, or an unconstrained position —
    /// interned as a fresh inference variable so it unifies with whatever the
    /// consumer needs.
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedField {
    pub name: String,
    pub optional: bool,
    pub ty: ImportedType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportedTupleItem {
    Named { name: String, ty: ImportedType },
    Positional(ImportedType),
}
