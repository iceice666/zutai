/// A handle into the [`TyInterner`] arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyId(pub u32);

/// A type in the Zutai v0 type system.
///
/// **Currently only `Unknown` is used (M0 skeleton).**
///
/// Reserved variants to add as the type-checking pass (M2) lands:
/// ```text
/// Int
/// Float
/// Text
/// Bool
/// Atom(String)            // singleton atom type like #ok
/// Optional(TyId)          // T?
/// List(TyId)              // List T
/// Record(Box<RecordTy>)   // { field : T; ... }
/// Union(Vec<TyId>)        // [ T; U; ... ]
/// Function { param: TyId, ret: TyId }
/// Var(u32)                // unification variable (for HM inference)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// Placeholder used before type information is computed.
    Unknown,
}

// ── TyInterner ────────────────────────────────────────────────────────────────

/// Cheap structural-equality deduplicating arena for [`Ty`] values.
///
/// `TyId(0)` is always the `Unknown` sentinel.
pub struct TyInterner {
    types: Vec<Ty>,
}

impl TyInterner {
    pub fn new() -> Self {
        Self {
            types: vec![Ty::Unknown],
        }
    }

    /// The `Unknown` type sentinel (always `TyId(0)`).
    pub fn unknown() -> TyId {
        TyId(0)
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
}

impl Default for TyInterner {
    fn default() -> Self {
        Self::new()
    }
}
