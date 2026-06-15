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

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TlcType {
    Prim(PrimTy),
    Fun(TlcTypeId, TlcTypeId),
    ForAll(TlcTypeVar, TlcTypeId),
    TyVar(TlcTypeVar),
    TyApp(TlcTypeId, TlcTypeId),
    Record(Vec<TlcRecordField>),
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
    TyLam(TlcTypeVar, TlcExprId),
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
