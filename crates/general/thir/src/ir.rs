use la_arena::{Arena, Idx};
use zutai_hir::{BindingId, HirDeclId, HirExprId, HirImportSource, HirPatId};
use zutai_syntax::Span;
use zutai_syntax::ast;

pub type ThirDeclId = Idx<ThirDecl>;
pub type ThirExprId = Idx<ThirExpr>;
pub type ThirPatId = Idx<ThirPat>;

/// Type arena index. Kept as a plain `u32` newtype (not `la_arena::Idx`) because
/// the same ID space serves double duty as type-inference variable addresses
/// (`next_infer_var` / `infer_subst` in the lowerer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct ThirFile {
    pub decls: Vec<ThirDeclId>,
    pub final_expr: ThirExprId,
    pub decl_arena: Arena<ThirDecl>,
    pub expr_arena: Arena<ThirExpr>,
    pub pat_arena: Arena<ThirPat>,
    pub type_arena: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirDecl {
    pub source: HirDeclId,
    pub binding: BindingId,
    pub kind: ThirDeclKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirDeclKind {
    Value {
        ty: TypeId,
        value: ThirExprId,
    },
    TypeAlias {
        params: Vec<BindingId>,
        ty: TypeId,
    },
    Function {
        params: Vec<BindingId>,
        sig: TypeId,
        clauses: Vec<ThirClause>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirClause {
    pub patterns: Vec<ThirPatId>,
    pub guard: Option<ThirExprId>,
    pub body: ThirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirExpr {
    pub source: HirExprId,
    pub ty: TypeId,
    pub kind: ThirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirExprKind {
    Error,
    True,
    False,
    Integer(i64),
    Float(f64),
    String(String),
    Atom(String),
    BindingRef(BindingId),
    Record(Vec<ThirRecordField>),
    Tuple(Vec<ThirTupleItem>),
    List(Vec<ThirExprId>),
    Block {
        bindings: Vec<ThirLocalBinding>,
        result: ThirExprId,
    },
    Lambda {
        params: Vec<ThirPatId>,
        body: ThirExprId,
    },
    If {
        cond: ThirExprId,
        then_branch: ThirExprId,
        else_branch: ThirExprId,
    },
    Match {
        scrutinee: ThirExprId,
        arms: Vec<ThirClause>,
    },
    Import(HirImportSource),
    TypeValue(TypeId),
    Apply {
        func: ThirExprId,
        arg: ThirExprId,
        instantiation: Vec<TypeId>,
    },
    Access {
        receiver: ThirExprId,
        field: String,
    },
    OptionalAccess {
        receiver: ThirExprId,
        field: String,
    },
    Binary {
        op: ast::BinOp,
        lhs: ThirExprId,
        rhs: ThirExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirRecordField {
    pub name: String,
    pub value: ThirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirTupleItem {
    Named {
        name: String,
        value: ThirExprId,
        span: Span,
    },
    Positional(ThirExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirLocalBinding {
    pub binding: BindingId,
    pub ty: TypeId,
    pub value: ThirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirPat {
    pub source: HirPatId,
    pub ty: TypeId,
    pub kind: ThirPatKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirPatKind {
    Error,
    Wildcard,
    Bind(BindingId),
    True,
    False,
    Integer(i64),
    Float(f64),
    String(String),
    Atom(String),
    Tuple(Vec<ThirTuplePatItem>),
    Record(Vec<ThirRecordPatField>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirTuplePatItem {
    Named {
        name: String,
        pattern: ThirPatId,
        span: Span,
    },
    Positional(ThirPatId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirRecordPatField {
    pub name: String,
    pub pattern: ThirPatId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Type,
    Bool,
    Text,
    Int,
    Float,
    Atom(String),
    True,
    False,
    List(TypeId),
    Optional(TypeId),
    Record(Vec<TypeRecordField>),
    Union(Vec<TypeId>),
    Tuple(Vec<TypeTupleItem>),
    Function {
        from: TypeId,
        to: TypeId,
    },
    TypeVar(BindingId),
    /// Inference metavariable generated during type inference.  Solved by the
    /// unification engine and replaced (zonked) with the concrete type before
    /// the `ThirFile` is returned.  Free (unsolved) InferVars represent
    /// polymorphic positions; TLC (Phase 2) will generalize them explicitly.
    InferVar(u32),
    Alias(BindingId),
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeRecordField {
    pub name: String,
    pub optional: bool,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeTupleItem {
    Named {
        name: String,
        ty: TypeId,
        span: Span,
    },
    Positional(TypeId),
}
