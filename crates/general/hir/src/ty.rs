use text_size::TextRange;

use crate::arena::Idx;
use crate::symbol::SymbolId;

// ── HirType identifiers ───────────────────────────────────────────────────────

pub type HirTypeId = Idx<HirType>;

// ── HirTyRef ──────────────────────────────────────────────────────────────────

/// Reference to a semantic `Ty` from the semantic crate.
/// Stored as `u32` to avoid a dependency on the semantic crate from hir.
/// `u32::MAX` = Unknown (not yet inferred).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirTyRef(pub u32);

impl HirTyRef {
    pub const UNKNOWN: Self = Self(u32::MAX);
}

// ── Field kind ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Required,
    /// Optional field (the field may be absent; `field? : T` in source).
    Optional,
}

// ── HirType ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HirType {
    pub kind: HirTypeKind,
    pub range: TextRange,
}

#[derive(Debug, Clone)]
pub enum HirTypeKind {
    /// Reference to a named type (built-in `Int`, `Bool`, type param `A`, type alias `Foo`).
    Var(SymbolId),
    /// Type constructor application: `List T`, `Pair A B`.
    Apply { ctor: HirTypeId, arg: HirTypeId },
    /// Function type: `A -> B`.
    Function { param: HirTypeId, ret: HirTypeId },
    /// Closed record type: `{ field : T; field? : U; }`.
    Record {
        fields: Vec<(String, HirTypeId, FieldKind)>,
    },
    /// Union type: `[ T; U; ]`.
    Union { variants: Vec<HirTypeId> },
    /// Tagged variant type: `(#tag, field : T)`.
    /// Kept distinct from Record intentionally (see plan §"Variant vs Record").
    Variant {
        tag: String,
        fields: Vec<(String, HirTypeId)>,
    },
    /// Optional type sugar: `T?`. Normalized to `Union([T, none])` by M2.
    Optional(HirTypeId),
    /// Singleton atom type: `#ok`.
    SingletonAtom(String),
    /// Singleton literal type: `true`, `false`, or `none`.
    SingletonLit(LitVal),
    /// Placeholder when type elaboration fails.
    Error,
}

// ── LitVal ────────────────────────────────────────────────────────────────────

/// A literal value — used in both expressions and patterns.
#[derive(Debug, Clone, PartialEq)]
pub enum LitVal {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Atom(String),
}
