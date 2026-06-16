use std::collections::HashMap;

use la_arena::{Arena, Idx};
use zutai_hir::BindingId;
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

// ── Row type (Phase 0: closed rows only; RVar added in Phase 3) ───────────────

/// A structural row — the spine of `RecordT` and `VariantT`.
/// Closed in v0 (no `RVar`); open-row polymorphism is Phase 3.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// Empty / closed tail.
    REmpty,
    /// Extend the row with one labelled field.
    RExtend {
        label: String,
        ty: TlcTypeId,
        tail: Box<Row>,
    },
}

impl Row {
    /// Build a closed row from an iterator of `(label, ty)` pairs (first pair = outermost).
    pub fn from_fields(fields: impl IntoIterator<Item = (String, TlcTypeId)>) -> Self {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.reverse();
        fields
            .into_iter()
            .fold(Row::REmpty, |tail, (label, ty)| Row::RExtend {
                label,
                ty,
                tail: Box::new(tail),
            })
    }

    /// Iterate over `(label, ty)` pairs in declaration order.
    pub fn fields(&self) -> impl Iterator<Item = (&str, TlcTypeId)> {
        RowIter(self)
    }
}

struct RowIter<'a>(&'a Row);

impl<'a> Iterator for RowIter<'a> {
    type Item = (&'a str, TlcTypeId);
    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            Row::REmpty => None,
            Row::RExtend { label, ty, tail } => {
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
    Fun(TlcTypeId, TlcTypeId),
    ForAll(TlcTypeVar, Kind, TlcTypeId),
    TyVar(TlcTypeVar, Kind),
    TyApp(TlcTypeId, TlcTypeId),
    /// Type-level lambda (F-ω). Binds `a : k` in `body`. Introduced in Phase 2 for
    /// generic type aliases; reduced by the NbE normalizer (`normalize.rs`).
    TyLamK(TlcTypeVar, Kind, TlcTypeId),
    Record(Vec<TlcRecordField>),
    /// Union / sum type former — replaces the silent `Union → Record([])` collapse.
    VariantT(Row),
    Tuple(Vec<TlcTupleField>),
    List(TlcTypeId),
    Optional(TlcTypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimTy {
    Int,
    Float,
    Bool,
    Str,
    /// Kept for the unqualified `Atom` primitive type (not a singleton).
    Atom,
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TlcRecordField {
    pub name: String,
    pub optional: bool,
    pub ty: TlcTypeId,
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
    GetField(TlcExprId, String),
    Tuple(Vec<TlcTupleItem>),
    List(Vec<TlcExprId>),
    Builtin(BuiltinOp, TlcExprId, TlcExprId),
    /// Inject a value into a sum / union arm: `#dev` or `(#circle, …)`.
    Variant(String, TlcExprId),
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
}
