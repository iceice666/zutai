use la_arena::{Arena, Idx};
use zutai_syntax::Span;
use zutai_syntax::ast;
use zutai_syntax::numlit::NumberType;
use zutai_syntax::posit::PositLiteral;

pub type HirDeclId = Idx<HirDecl>;
pub type HirExprId = Idx<HirExpr>;
pub type HirPatId = Idx<HirPat>;
pub type HirTypeId = Idx<HirTypeExpr>;

/// Names of the compiler-provided value bindings seeded into every module's
/// root scope ("the prelude"). `print` is a compatibility binding over the
/// `io.print` effect. Single source of truth shared by HIR seeding, THIR type
/// assignment, and the reference interpreter.
pub const BUILTIN_VALUE_NAMES: &[&str] = &["print", "fields", "schema", "overlay", "overlayDeep"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct HirFile {
    pub decls: Vec<HirDeclId>,
    pub final_expr: HirExprId,
    pub span: Span,
    pub bindings: Vec<Binding>,
    pub decl_arena: Arena<HirDecl>,
    pub expr_arena: Arena<HirExpr>,
    pub pat_arena: Arena<HirPat>,
    pub type_arena: Arena<HirTypeExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub kind: BindingKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    BuiltinType,
    /// A compiler-provided value binding seeded into the root scope (e.g. the
    /// `print` prelude builtin). Reserved like builtin types: a top-level user
    /// redefinition raises `DuplicateBinding`.
    BuiltinValue,
    TopValue,
    TopFunction,
    TopType,
    TopConstraint,
    TopWitness,
    /// A method declared inside a constraint definition (named or operator).
    /// Named methods are registered in scope; operator methods use unscoped bindings.
    ConstraintMethod,
    TypeParam,
    Local,
    Param,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirDecl {
    pub binding: BindingId,
    pub kind: HirDeclKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirTypeParam {
    pub binding: BindingId,
    pub bounds: Vec<BindingId>,
    pub kind: Option<HirTypeId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirConstraintMethod {
    pub name: String,
    pub is_operator: bool,
    pub optional: bool,
    pub params: Vec<HirTypeParam>,
    pub sig: HirTypeId,
    pub default: Vec<HirClause>,
    pub span: Span,
    /// `BindingId` allocated for this method in Pass 1.
    /// Both named and operator methods get a `ConstraintMethod` binding (D6/4b).
    /// Named methods are also registered in scope so expressions can reference them;
    /// operator methods use an unscoped binding since operators are not bare idents.
    pub binding: Option<BindingId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirWitnessField {
    pub name: String,
    pub is_operator: bool,
    pub value: HirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirDeclKind {
    Value {
        annotation: Option<HirTypeId>,
        value: HirExprId,
    },
    TypeAlias {
        params: Vec<BindingId>,
        ty: HirTypeId,
    },
    Function {
        params: Vec<HirTypeParam>,
        sig: Option<HirTypeId>,
        clauses: Vec<HirClause>,
    },
    Constraint {
        params: Vec<HirTypeParam>,
        target: HirTypeId,
        methods: Vec<HirConstraintMethod>,
        derivable: bool,
    },
    Witness {
        constraint: Option<BindingId>,
        target: HirTypeId,
        params: Vec<HirTypeParam>,
        fields: Vec<HirWitnessField>,
        derive: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirClause {
    pub patterns: Vec<HirPatId>,
    pub guard: Option<HirExprId>,
    pub body: HirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    True,
    False,
    Integer(i64, Option<NumberType>),
    Float(f64, Option<NumberType>),
    Posit(PositLiteral),
    String(String),
    Atom(String),
    TaggedValue {
        tag: String,
        payload: HirExprId,
    },
    BindingRef(BindingId),
    UnresolvedIdent(String),
    Record(Vec<HirRecordField>),
    RecordUpdate {
        receiver: HirExprId,
        fields: Vec<HirRecordField>,
    },
    Tuple(Vec<HirTupleItem>),
    List(Vec<HirExprId>),
    Block {
        bindings: Vec<HirLocalBinding>,
        result: HirExprId,
    },
    Lambda {
        params: Vec<HirPatId>,
        body: HirExprId,
    },
    If {
        cond: HirExprId,
        then_branch: HirExprId,
        else_branch: HirExprId,
    },
    Match {
        scrutinee: HirExprId,
        arms: Vec<HirClause>,
    },
    Import(HirImportSource),
    TypeForm(HirTypeId),
    Select {
        receiver: HirExprId,
        fields: Vec<HirSelectField>,
    },
    Perform {
        op: Vec<String>,
        arg: HirExprId,
    },
    Handle {
        expr: HirExprId,
        clauses: Vec<HirHandleClause>,
    },
    Resume {
        value: HirExprId,
    },
    Sequence(Vec<HirExprId>),
    Apply {
        func: HirExprId,
        arg: HirExprId,
    },
    Access {
        receiver: HirExprId,
        field: String,
    },
    OptAccess {
        receiver: HirExprId,
        field: String,
    },
    Binary {
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirRecordField {
    pub name: String,
    pub value: HirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirTupleItem {
    Named {
        name: String,
        value: HirExprId,
        span: Span,
    },
    Positional(HirExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirLocalBinding {
    pub binding: BindingId,
    pub value: HirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HirImportSource {
    String(String),
    Path(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirPat {
    pub kind: HirPatKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirPatKind {
    Wildcard,
    Bind(BindingId),
    True,
    False,
    Integer(i64, Option<NumberType>),
    Float(f64, Option<NumberType>),
    Posit(PositLiteral),
    String(String),
    Atom(String),
    TaggedValue {
        tag: String,
        payload: Vec<HirRecordPatField>,
    },
    Tuple(Vec<HirTuplePatItem>),
    Record(Vec<HirRecordPatField>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirTuplePatItem {
    Named {
        name: String,
        pattern: HirPatId,
        span: Span,
    },
    Positional(HirPatId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirRecordPatField {
    pub name: String,
    pub pattern: HirPatId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirTypeExpr {
    pub kind: HirTypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirTypeKind {
    BindingRef(BindingId),
    UnresolvedIdent(String),
    Record {
        fields: Vec<HirTypeRecordField>,
        tail: Option<HirRowTail>,
    },
    Union {
        variants: Vec<HirUnionVariant>,
        tail: Option<HirRowTail>,
    },
    Tuple(Vec<HirTypeTupleItem>),
    Optional(HirTypeId),
    Arrow {
        from: HirTypeId,
        to: HirTypeId,
    },
    Apply {
        func: HirTypeId,
        arg: HirTypeId,
    },
    Access {
        receiver: HirTypeId,
        field: String,
    },
    Effect {
        base: HirTypeId,
        row: HirEffectRow,
    },
    Select {
        receiver: HirTypeId,
        fields: Vec<HirSelectField>,
    },
    Atom(String),
    True,
    False,
    ExprEscape(HirExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirUnionVariant {
    pub name: String,
    pub payload: Option<HirTypeId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirTypeRecordField {
    pub name: String,
    pub optional: bool,
    pub ty: HirTypeId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirTypeTupleItem {
    Named {
        name: String,
        ty: HirTypeId,
        span: Span,
    },
    Positional(HirTypeId),
}

/// A field name selected by a value- or type-level `select` projection.
#[derive(Debug, Clone, PartialEq)]
pub struct HirSelectField {
    pub name: String,
    pub span: Span,
}

/// The row tail of an open record or union type.
#[derive(Debug, Clone, PartialEq)]
pub struct HirRowTail {
    pub kind: HirRowTailKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirRowTailKind {
    /// Anonymous open row tail `...`: accepts any extra fields/members but does
    /// not name them.
    Anonymous,
    /// Named row variable `...Rest` resolved to an in-scope type parameter.
    Var(BindingId),
    /// Spread `...Shape` of a named record/union type into this row.
    Spread(BindingId),
    /// `...Name` whose `Name` did not resolve to a type parameter or a type.
    Unresolved(String),
}

/// An effect row `! { op ...; }` annotating an effectful function type.
#[derive(Debug, Clone, PartialEq)]
pub struct HirEffectRow {
    pub ops: Vec<HirEffectOp>,
    pub span: Span,
}

/// A single operation in an effect row.
#[derive(Debug, Clone, PartialEq)]
pub struct HirEffectOp {
    /// Operation name path: a plain operation (`fail`) or a dotted capability
    /// operation (`fs.read`). Effect operations are not bindings in v0/v1 HIR.
    pub path: Vec<String>,
    pub payload: Option<HirTypeId>,
    pub signature: Option<HirTypeId>,
    pub span: Span,
}

/// A clause of a `handle ... with { ... }` expression.
#[derive(Debug, Clone, PartialEq)]
pub struct HirHandleClause {
    pub op: HirHandleOp,
    pub body: HirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirHandleOp {
    /// The special `value = ...` clause handling the final value.
    Value,
    /// An operation clause handling a performed operation by name path.
    Operation(Vec<String>),
}
