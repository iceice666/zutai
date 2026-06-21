use std::collections::HashMap;

use la_arena::{Arena, Idx};
use zutai_hir::{BindingId, HirImportSource};
use zutai_syntax::Span;

// ── Arena IDs ────────────────────────────────────────────────────────────────

pub type TlcDeclId = Idx<TlcDecl>;
pub type TlcExprId = Idx<TlcExpr>;
pub type TlcTypeId = Idx<TlcType>;

// ── Type variables ────────────────────────────────────────────────────────────

/// Two namespaces kept separate so their u32s never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlcTypeVar {
    Named(u32),    // BindingId.0 of an explicit HIR type parameter
    Inferred(u32), // InferVar id from poly_schemes
}

// ── Kind language (spec §3) ───────────────────────────────────────────────────

/// Kind language (spec §3). Phase 1 only ever constructs `Type(0)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `Type ℓ` — universe at level ℓ. Ground types are `Type(0)`.
    Type(u32),
    /// `Row κ` — a row whose entries have kind κ (records / unions / effects). Dormant until Phase 3.
    Row(Box<Kind>),
    /// `κ₁ -> κ₂` — type-constructor kind (HKT / F-ω layer). Dormant until Phase 3/5.
    Arrow(Box<Kind>, Box<Kind>),
}

impl Kind {
    /// The ground kind `Type 0` — the Phase 1 default for every annotation.
    pub fn ground() -> Kind {
        Kind::Type(0)
    }
}

// ── Row type (Phase 3: RVar added for open-row polymorphism) ─────────────────

/// A structural row — the spine of `RecordT` and `VariantT`.
/// Phase 3 adds `RVar` for open-row polymorphism and `optional` on `RExtend` for optional fields.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// Empty / closed tail.
    REmpty,
    /// Extend the row with one labelled field.
    RExtend {
        label: String,
        ty: TlcTypeId,
        /// True for optional record fields (`field? : T`). Always false for variant arms.
        optional: bool,
        tail: Box<Row>,
    },
    /// Row variable — the open tail of a polymorphic row (`...Rest`).
    RVar(TlcTypeVar),
}

impl Row {
    /// Build a closed row from `(label, ty)` pairs with `optional = false` (used for variant arms).
    pub fn from_fields(fields: impl IntoIterator<Item = (String, TlcTypeId)>) -> Self {
        Row::from_fields_with_tail(fields, Row::REmpty)
    }

    /// Like `from_fields` but folds the fields over an explicit `tail` row
    /// (`REmpty` for closed rows, `RVar` for open/row-polymorphic ones).
    pub fn from_fields_with_tail(
        fields: impl IntoIterator<Item = (String, TlcTypeId)>,
        tail: Row,
    ) -> Self {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.reverse();
        fields
            .into_iter()
            .fold(tail, |tail, (label, ty)| Row::RExtend {
                label,
                ty,
                optional: false,
                tail: Box::new(tail),
            })
    }

    /// Build a closed row from `(label, ty, optional)` triples (used for record types).
    pub fn from_record_fields(fields: impl IntoIterator<Item = (String, TlcTypeId, bool)>) -> Self {
        Row::from_record_fields_with_tail(fields, Row::REmpty)
    }

    /// Like `from_record_fields` but folds the fields over an explicit `tail` row.
    pub fn from_record_fields_with_tail(
        fields: impl IntoIterator<Item = (String, TlcTypeId, bool)>,
        tail: Row,
    ) -> Self {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.reverse();
        fields
            .into_iter()
            .fold(tail, |tail, (label, ty, optional)| Row::RExtend {
                label,
                ty,
                optional,
                tail: Box::new(tail),
            })
    }

    /// Iterate over `(label, ty)` pairs in declaration order, stopping at `RVar` or `REmpty`.
    pub fn fields(&self) -> impl Iterator<Item = (&str, TlcTypeId)> {
        RowIter(self)
    }

    /// Splice `replacement` into the position of `row_var`'s tail. Flattens an open row
    /// whose tail is `RVar(row_var)` into `replacement` (open→closed when `replacement` is
    /// closed). Inert if this row does not contain `RVar(row_var)`.
    ///
    /// This is a row-variable substitution — distinct from type-variable `subst` in
    /// `normalize.rs`, which leaves `RVar` inert (different kind).
    pub fn subst_row_var(self, row_var: TlcTypeVar, replacement: Row) -> Row {
        match self {
            Row::REmpty => Row::REmpty,
            Row::RVar(v) if v == row_var => replacement,
            Row::RVar(v) => Row::RVar(v),
            Row::RExtend {
                label,
                ty,
                optional,
                tail,
            } => Row::RExtend {
                label,
                ty,
                optional,
                tail: Box::new(tail.subst_row_var(row_var, replacement)),
            },
        }
    }
}

struct RowIter<'a>(&'a Row);

impl<'a> Iterator for RowIter<'a> {
    type Item = (&'a str, TlcTypeId);
    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            Row::REmpty | Row::RVar(_) => None,
            Row::RExtend {
                label, ty, tail, ..
            } => {
                self.0 = tail;
                Some((label.as_str(), *ty))
            }
        }
    }
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TlcType {
    Prim(PrimTy),
    /// Singleton type: `true`, `false`, `#atom`, integer literal, …
    /// Fixes the silent `True`/`False` and `Atom` data-loss bugs.
    Singleton(Literal),
    /// A function type carrying an effect row (`EffRow ::= Row`, spec §4 line 133).
    /// In v0 every function is pure: `eff = REmpty` always.
    Fun(TlcTypeId, TlcTypeId, Row),
    ForAll(TlcTypeVar, Kind, TlcTypeId),
    TyVar(TlcTypeVar, Kind),
    TyApp(TlcTypeId, TlcTypeId),
    /// Type-level lambda (F-ω). Binds `a : k` in `body`. Introduced in Phase 2 for
    /// generic type aliases; reduced by the NbE normalizer (`normalize.rs`).
    TyLamK(TlcTypeVar, Kind, TlcTypeId),
    /// Record type — the spine is a `Row` (closed in front-end lowering; open via `RVar` in hand-built IR).
    Record(Row),
    /// Union / sum type former — replaces the silent `Union → Record([])` collapse.
    VariantT(Row),
    Tuple(Vec<TlcTupleField>),
    List(TlcTypeId),
    Optional(TlcTypeId),
    Maybe(TlcTypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimTy {
    Int,
    Float,
    FixedNum(zutai_thir::FixedWidth),
    Bool,
    Str,
    /// Kept for the unqualified `Atom` primitive type (not a singleton).
    Atom,
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TlcTupleField {
    Named { name: String, ty: TlcTypeId },
    Positional(TlcTypeId),
}

// ── Expressions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TlcExpr {
    Var(BindingId),
    Lit(Literal),
    Lam(BindingId, TlcTypeId, TlcExprId),
    App(TlcExprId, TlcExprId),
    TyLam(TlcTypeVar, Kind, TlcExprId),
    TyApp(TlcExprId, TlcTypeId),
    Let {
        binding: BindingId,
        ty: TlcTypeId,
        value: TlcExprId,
        body: TlcExprId,
    },
    Letrec {
        bindings: Vec<(BindingId, TlcTypeId, TlcExprId)>,
        body: TlcExprId,
    },
    Case(TlcExprId, Vec<TlcAlt>),
    Record(Vec<(String, TlcExprId)>),
    RecordUpdate {
        receiver: TlcExprId,
        fields: Vec<(String, TlcExprId)>,
    },
    GetField(TlcExprId, String),
    Tuple(Vec<TlcTupleItem>),
    List(Vec<TlcExprId>),
    Builtin(BuiltinOp, TlcExprId, TlcExprId),
    Import(HirImportSource),
    /// Inject a value into a sum / union arm: `#dev` or `(#circle, …)`.
    Variant(String, TlcExprId),
    /// Invoke an algebraic effect operation. The result is delivered by a
    /// source handler or by the host boundary for supported host effects.
    Perform {
        op: String,
        arg: TlcExprId,
    },
    /// Delimit effect handling for `expr`.
    Handle {
        expr: TlcExprId,
        value: Option<TlcExprId>,
        ops: Vec<TlcHandleClause>,
    },
    /// Resume the current one-shot operation continuation.
    Resume {
        value: TlcExprId,
    },
    /// Explicit left-to-right sequencing boundary.
    Sequence(Vec<TlcExprId>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TlcHandleClause {
    pub op: String,
    pub body: TlcExprId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TlcAlt {
    pub pat: TlcPat,
    pub guard: Option<TlcExprId>,
    pub body: TlcExprId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TlcPat {
    Wildcard,
    Bind(BindingId),
    Lit(Literal),
    Atom(String),
    Tuple(Vec<TlcPatItem>),
    Record(Vec<(String, TlcPat)>),
    /// Match a sum / union arm: `(#circle, inner_pat)`.
    Variant(String, Box<TlcPat>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TlcPatItem {
    Named { name: String, pat: TlcPat },
    Positional(TlcPat),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TlcTupleItem {
    Named { name: String, value: TlcExprId },
    Positional(TlcExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Atom(String),
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
}

// ── Declarations ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TlcDecl {
    Value {
        binding: BindingId,
        ty: TlcTypeId,
        body: TlcExprId,
    },
    TypeAlias {
        binding: BindingId,
        params: Vec<BindingId>,
        body: TlcTypeId,
    },
}

// ── Module ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct TlcModule {
    pub decls: Vec<TlcDeclId>,
    pub decl_arena: Arena<TlcDecl>,
    pub expr_arena: Arena<TlcExpr>,
    pub type_arena: Arena<TlcType>,
    pub expr_types: HashMap<TlcExprId, TlcTypeId>,
    pub spans: HashMap<TlcExprId, Span>,
    /// The module's final expression — evaluated to produce the module's value.
    pub final_expr: Option<TlcExprId>,
}
