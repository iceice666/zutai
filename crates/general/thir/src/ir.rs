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
    pub poly_schemes: std::collections::HashMap<zutai_hir::BindingId, Vec<u32>>,
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
        /// Per-param constraint bounds. `param_bounds[i]` is the list of constraint
        /// `BindingId`s required by `params[i]`. Populated from `HirTypeParam.bounds`
        /// during HIR→THIR lowering. Empty inner vecs mean an unconstrained type param.
        param_bounds: Vec<Vec<BindingId>>,
        sig: TypeId,
        clauses: Vec<ThirClause>,
    },
    /// A constraint definition: `Eq :: <A> @A { eq :: A -> A -> Bool; }`.
    /// `params` are the constraint's own type-param bindings (e.g. `A`).
    /// Increments 3/4 (witness checking, coherence) are done.
    /// D6: operator bindings and default bodies are now present in each method.
    /// Method-level params are still deferred.
    Constraint {
        params: Vec<BindingId>,
        target: TypeId,
        methods: Vec<ThirConstraintMethod>,
        derivable: bool,
    },
    /// A constraint witness: `Eq @Int :: { eq = intEq; }`.
    /// `constraint` is the resolved binding of the named constraint (None if unresolved).
    /// Witness fields are lowered via `infer_expr` but not checked against method sigs.
    Witness {
        constraint: Option<BindingId>,
        target: TypeId,
        params: Vec<BindingId>,
        fields: Vec<ThirWitnessField>,
        derive: bool,
    },
}

/// A single method in a constraint definition.
/// D6: operator bindings and default bodies are now carried through.
/// Method-level type params (`<A,B>`) are still deferred.
#[derive(Debug, Clone, PartialEq)]
pub struct ThirConstraintMethod {
    pub name: String,
    pub is_operator: bool,
    pub optional: bool,
    pub sig: TypeId,
    pub span: Span,
    /// `BindingId` for this method. Both named and operator methods now get `Some(_)` (D6/4b).
    pub binding: Option<BindingId>,
    /// Lowered default clause body, if one was provided in the constraint definition (D6/4a).
    /// `None` means no default; `Some(clauses)` carries the type-checked clauses.
    pub default: Option<Vec<ThirClause>>,
}

/// A single field in a constraint witness.
#[derive(Debug, Clone, PartialEq)]
pub struct ThirWitnessField {
    pub name: String,
    pub is_operator: bool,
    pub value: ThirExprId,
    pub span: Span,
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
    TaggedValue {
        tag: String,
        payload: ThirExprId,
    },
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
    TaggedValue {
        tag: String,
        payload: Vec<ThirRecordPatField>,
    },
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
    Union(Vec<UnionVariant>),
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
    /// Application of a parametric type constructor (generic alias or type-level
    /// function) to type arguments. Lazy: expanded on demand in `resolve_alias`
    /// by substituting `args` for the constructor's params. Never reaches `unify`
    /// directly (callers resolve via `resolve_alias`/`type_matches` first),
    /// mirroring `Alias`.
    AliasApply {
        binding: BindingId,
        args: Vec<TypeId>,
    },
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnionVariant {
    pub name: String,
    pub payload: Option<TypeId>,
    pub span: Span,
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
