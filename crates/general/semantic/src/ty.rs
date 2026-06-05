/// A handle into the [`TyInterner`] arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyId(pub u32);

// Pre-interned primitive TyIds — positions MUST match `TyInterner::new()`.
pub const UNKNOWN_TY: TyId = TyId(0);
pub const INT_TY: TyId = TyId(1);
pub const FLOAT_TY: TyId = TyId(2);
pub const TEXT_TY: TyId = TyId(3);
pub const BOOL_TY: TyId = TyId(4);
pub const NONE_TY: TyId = TyId(5);

// ── FieldKind ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Required,
    Optional,
}

// ── RecordField ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordField {
    pub name: String,
    pub ty: TyId,
    pub kind: FieldKind,
}

// ── TupleElem ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TupleElem {
    Positional(TyId),
    Named(String, TyId),
}

// ── Ty ────────────────────────────────────────────────────────────────────────

/// A type in the Zutai v0 type system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// Placeholder — not yet inferred or elaboration failed. `TyId(0)` is always `Unknown`.
    Unknown,

    // ── Primitives ────────────────────────────────────────────────────────────
    Int,
    Float,
    Text,
    Bool,
    /// The `none` value's type.
    None,

    // ── Singleton types ───────────────────────────────────────────────────────
    /// Singleton atom type: `#ok`, `#err`, etc.
    Atom(String),

    // ── Composite types ───────────────────────────────────────────────────────
    /// `T?` — optional type.
    Optional(TyId),
    /// `List T` — homogeneous list.
    List(TyId),
    /// Closed record type: `{ field : T; field? : U }`.
    Record(Vec<RecordField>),
    /// Union type: `[ T; U ]`.
    Union(Vec<TyId>),
    /// Tuple type: `(A, B)` or `(#tag, field : T)`.
    Tuple(Vec<TupleElem>),
    /// Function type: `A -> B`.
    Function {
        param: TyId,
        ret: TyId,
    },
    /// Unresolved type constructor application (e.g. user-defined `Pair A B`).
    Apply {
        ctor: TyId,
        arg: TyId,
    },

    // ── Polymorphism ──────────────────────────────────────────────────────────
    /// Rigid type parameter (from `[A, B]` on a declaration).
    Param(u32),
}

impl Ty {
    /// If this tuple starts with a singleton atom, return that tag and the
    /// remaining payload elements. Union and pattern analysis use this as the
    /// semantic tagged-tuple recognizer.
    pub fn as_tagged_tuple<'a>(
        &'a self,
        interner: &'a TyInterner,
    ) -> Option<(&'a str, &'a [TupleElem])> {
        let Ty::Tuple(items) = self else {
            return None;
        };
        let Some(TupleElem::Positional(first)) = items.first() else {
            return None;
        };
        let Ty::Atom(tag) = interner.get(*first) else {
            return None;
        };
        Some((tag.as_str(), &items[1..]))
    }
}

// ── TyInterner ────────────────────────────────────────────────────────────────

/// Structural-equality deduplicating arena for [`Ty`] values.
///
/// The first six slots are always the pre-interned primitives (see constants
/// above). `TyId(0)` is always `Unknown`. The ordering of `new()`'s initial
/// `vec!` MUST match the constant definitions — a unit test guards this.
pub struct TyInterner {
    types: Vec<Ty>,
}

impl TyInterner {
    pub fn new() -> Self {
        Self {
            // Order MUST match UNKNOWN_TY / INT_TY / FLOAT_TY / TEXT_TY / BOOL_TY / NONE_TY.
            types: vec![
                Ty::Unknown, // 0
                Ty::Int,     // 1
                Ty::Float,   // 2
                Ty::Text,    // 3
                Ty::Bool,    // 4
                Ty::None,    // 5
            ],
        }
    }

    /// Intern a type, returning a stable `TyId`. Deduplicates by structural equality.
    pub fn intern(&mut self, ty: Ty) -> TyId {
        if let Some(pos) = self.types.iter().position(|t| t == &ty) {
            return TyId(pos as u32);
        }
        let id = TyId(self.types.len() as u32);
        self.types.push(ty);
        id
    }

    pub fn get(&self, id: TyId) -> &Ty {
        &self.types[id.0 as usize]
    }

    pub fn is_unknown(&self, id: TyId) -> bool {
        id == UNKNOWN_TY
    }
}

impl Default for TyInterner {
    fn default() -> Self {
        Self::new()
    }
}
