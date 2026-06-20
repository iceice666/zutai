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
/// The kind of a type: `Star` is the kind of ordinary types (`Int`, `List Int`);
/// `Arrow` is the kind of a type constructor (`List : Type -> Type`). Used to
/// kind-check higher-kinded constraints/witnesses. Mirrors TLC's richer `Kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Star,
    Arrow(Box<Kind>, Box<Kind>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirFile {
    pub decls: Vec<ThirDeclId>,
    pub final_expr: ThirExprId,
    pub decl_arena: Arena<ThirDecl>,
    pub expr_arena: Arena<ThirExpr>,
    pub pat_arena: Arena<ThirPat>,
    pub type_arena: Vec<Type>,
    pub poly_schemes: std::collections::HashMap<zutai_hir::BindingId, Vec<u32>>,
    /// Declared kind of each type parameter (`<F :: Type -> Type>` → `Arrow`),
    /// keyed by the param's `BindingId`. Absent params have kind `Star`. Carried
    /// for TLC so higher-kinded quantifiers/vars get the right kind.
    pub type_param_kinds: std::collections::HashMap<zutai_hir::BindingId, Kind>,
    pub binding_names: Vec<String>,
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
        /// Per-param constraint bounds, parallel to `params`. `param_bounds[i]` is the
        /// list of constraint `BindingId`s required by `params[i]` (a conditional
        /// witness predicate such as `<A: Eq>`). Empty inner vecs mean an unconstrained
        /// witness parameter. Mirrors `Function::param_bounds`.
        param_bounds: Vec<Vec<BindingId>>,
        fields: Vec<ThirWitnessField>,
        derive: bool,
    },
}

/// A single method in a constraint definition.
/// D6: operator bindings and default bodies are now carried through.
/// Phase 14: method-level type params (`<A,B>`) are preserved in `params`.
#[derive(Debug, Clone, PartialEq)]
pub struct ThirConstraintMethod {
    pub name: String,
    pub is_operator: bool,
    pub optional: bool,
    pub sig: TypeId,
    /// Method-level type parameters (`<A, B>` on the method itself), distinct from
    /// the constraint's own params. Source of truth for how many `TyLam` wrap the
    /// witness field and how many `TyApp` apply at a call site.
    pub params: Vec<BindingId>,
    /// Per-method-param constraint bounds, parallel to `params`.
    pub param_bounds: Vec<Vec<BindingId>>,
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
    RecordUpdate {
        receiver: ThirExprId,
        fields: Vec<ThirRecordField>,
    },
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
    Perform {
        op: String,
        arg: ThirExprId,
    },
    Handle {
        expr: ThirExprId,
        value: Option<ThirExprId>,
        ops: Vec<ThirHandleClause>,
    },
    Resume {
        value: ThirExprId,
    },
    Sequence(Vec<ThirExprId>),
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
pub struct ThirHandleClause {
    pub op: String,
    pub body: ThirExprId,
    pub span: Span,
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
pub struct EffectOp {
    pub name: String,
    pub param: TypeId,
    pub result: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectRow {
    pub ops: Vec<EffectOp>,
    pub tail: RowTail,
}

impl EffectRow {
    pub fn closed_empty() -> Self {
        Self {
            ops: Vec::new(),
            tail: RowTail::Closed,
        }
    }

    pub fn is_pure(&self) -> bool {
        self.ops.is_empty() && self.tail == RowTail::Closed
    }

    pub fn find(&self, name: &str) -> Option<&EffectOp> {
        self.ops.iter().find(|op| op.name == name)
    }
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
    Maybe(TypeId),
    Patch {
        target: TypeId,
        deep: bool,
    },
    Record(Vec<TypeRecordField>, RowTail),
    Union(Vec<UnionVariant>, RowTail),
    Tuple(Vec<TypeTupleItem>),
    Function {
        from: TypeId,
        to: TypeId,
    },
    Effect {
        base: TypeId,
        row: EffectRow,
    },
    Never,
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
    /// Curried application of a type constructor to a single argument — the
    /// representation for higher-kinded application (`F A`) and partial
    /// application (`Result E`). The `func` head is a `TypeId` so it composes
    /// under substitution: a `TypeVar(F)` head becomes an `InferVar` at a call
    /// site, then a concrete constructor once solved. Reduced by `resolve_alias`,
    /// which spine-walks, folds builtin `Con` applications (`Apply{Con(List),X}`
    /// → `List(X)`), expands saturated named aliases, and leaves abstract or
    /// under-saturated heads inert. Compared via `app_view` at every boundary so
    /// it is canonicalization-equivalent to the saturated `AliasApply`.
    Apply {
        func: TypeId,
        arg: TypeId,
    },
    /// A bare, unapplied builtin type constructor (`List`, `Optional`, `Maybe`) used as a
    /// higher-kinded witness/constraint target (`Functor @List`). Named aliases
    /// use `Alias(binding)` for their bare head; `Con` exists only for builtins
    /// that have no alias body.
    Con(BindingId),
    Error,
}

/// The tail of a record or union row: whether the listed fields/members are the
/// exact contents (`Closed`) or the row is open to additional ones. Open rows
/// carry either no name (`Open`, an anonymous `...`), a rigid row variable from
/// a `<Rest>` type parameter (`Param`), or a flexible metavariable solved during
/// unification (`Infer`). Mirrors the rigid `TypeVar` / flexible `InferVar` split
/// for ordinary types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTail {
    Closed,
    Open,
    Param(BindingId),
    Infer(u32),
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
