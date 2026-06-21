//! Resolved-import descriptors threaded into THIR lowering.
//!
//! THIR lowering is pure (no filesystem).  The semantic layer resolves each
//! `import` expression to a structural type descriptor and passes a map keyed
//! by the import source; THIR interns each descriptor into its own type arena
//! when it lowers the corresponding `Import` node.  Keeping the descriptor
//! independent of THIR's private `type_arena` is what lets the resolver live
//! outside this crate.

use crate::ir::FixedWidth;
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
/// expression (see [`crate::export_type`]).
#[derive(Debug, Clone, PartialEq)]
pub enum ImportedType {
    Bool,
    Int,
    Float,
    FixedNum(FixedWidth),
    Text,
    Atom(String),
    List(Box<ImportedType>),
    Optional(Box<ImportedType>),
    Maybe(Box<ImportedType>),
    Record(Vec<ImportedField>),
    Tuple(Vec<ImportedTupleItem>),
    Union(Vec<ImportedUnionVariant>),
    /// A function value crossing a module boundary.  The evaluator stamps a
    /// home-module handle on every closure so the body is evaluated against
    /// the *defining* module's arenas.
    Function {
        from: Box<ImportedType>,
        to: Box<ImportedType>,
    },
    /// A type-value field carrying its denotation.  Enables annotation-position
    /// use (`x : serverLib.Server`) by threading the concrete type through the
    /// import boundary.  The inner descriptor is the denoted structural type
    /// (e.g. the record `{ host: Text; port: Int }` behind `Server`).
    Type(Box<ImportedType>),
    /// Element type of an empty imported list, or an unconstrained position —
    /// interned as a fresh inference variable so it unifies with whatever the
    /// consumer needs.
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedUnionVariant {
    pub name: String,
    pub payload: Option<Box<ImportedType>>,
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
