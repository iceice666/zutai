use crate::numlit::NumberType;
use crate::posit::PositLiteral;
use crate::span::Span;

#[derive(Debug, PartialEq)]
pub struct File {
    pub decls: Vec<Decl>,
    pub final_expr: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct TypeParam {
    pub name: String,
    /// Set when the binder was written `$l` — a universe-level variable rather
    /// than a type parameter. Level binders carry no bounds or kind annotation.
    pub is_level: bool,
    pub bounds: Vec<TypeParamBound>,
    pub kind: Option<Box<TypeExpr>>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct TypeParamBound {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub enum MethodName {
    Ident(String),
    Operator(String),
}

impl MethodName {
    pub fn as_str(&self) -> &str {
        match self {
            MethodName::Ident(s) | MethodName::Operator(s) => s,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ConstraintMethod {
    pub name: MethodName,
    pub optional: bool,
    pub params: Vec<TypeParam>,
    pub sig: TypeExpr,
    pub default: Vec<FuncClause>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct WitnessField {
    pub name: MethodName,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub enum WitnessBody {
    Fields(Vec<WitnessField>),
    Derive,
}

#[derive(Debug, PartialEq)]
pub enum Decl {
    Inferred {
        name: String,
        value: Expr,
        span: Span,
    },
    Typed {
        name: String,
        ty: TypeExpr,
        value: Expr,
        span: Span,
    },
    /// Selective destructuring binding: `{ a; b; c } ::= rec;` binds each named
    /// member of the record value `value` as a top-level name. Reuses the
    /// select-field list syntax; the canonical use is bringing imported module
    /// members into scope unqualified (`{ map; fold } ::= s;`).
    Destructure {
        fields: Vec<SelectField>,
        value: Expr,
        span: Span,
    },
    TypeAlias {
        name: String,
        params: Vec<TypeParam>,
        ty: TypeExpr,
        span: Span,
    },
    Function {
        name: String,
        params: Vec<TypeParam>,
        sig: TypeExpr,
        clauses: Vec<FuncClause>,
        span: Span,
    },
    NoSigFn {
        name: String,
        patterns: Vec<Pattern>,
        body: Expr,
        span: Span,
    },
    Constraint {
        name: String,
        params: Vec<TypeParam>,
        target: TypeExpr,
        methods: Vec<ConstraintMethod>,
        derivable: bool,
        recipe: Option<DeriveRecipe>,
        span: Span,
    },
    Witness {
        constraint: String,
        target: TypeExpr,
        params: Vec<TypeParam>,
        body: WitnessBody,
        span: Span,
    },
}

impl Decl {
    pub fn span(&self) -> Span {
        match self {
            Decl::Inferred { span, .. }
            | Decl::Typed { span, .. }
            | Decl::TypeAlias { span, .. }
            | Decl::Function { span, .. }
            | Decl::NoSigFn { span, .. }
            | Decl::Constraint { span, .. }
            | Decl::Destructure { span, .. }
            | Decl::Witness { span, .. } => *span,
        }
    }

    /// The single bound name, or `""` for a destructuring binding (which binds
    /// several names — callers that care handle `Decl::Destructure` explicitly).
    pub fn name(&self) -> &str {
        match self {
            Decl::Inferred { name, .. }
            | Decl::Typed { name, .. }
            | Decl::TypeAlias { name, .. }
            | Decl::Function { name, .. }
            | Decl::NoSigFn { name, .. }
            | Decl::Constraint { name, .. } => name,
            Decl::Witness { constraint, .. } => constraint,
            Decl::Destructure { .. } => "",
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct FuncClause {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct DeriveRecipe {
    pub params: Vec<TypeParam>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct RecordField {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// A statement inside a `stream { … }` generator block. Richer-`yield` (V3-G3)
/// lets `yield` appear under conditionals and recursion; each statement desugars
/// onto the codata `Stream` cell with no second iterator abstraction.
#[derive(Debug, PartialEq)]
pub enum GenStmt {
    /// `yield e;` — emit one element.
    Yield { value: Expr, span: Span },
    /// `yield from e;` — splice every element of the sub-`Stream` `e`. Supported
    /// in tail position (the canonical recursive/loop generator); a non-tail use
    /// is reported by HIR lowering.
    YieldFrom { stream: Expr, span: Span },
    /// `if cond then { … } [else { … }]` — conditional yield. Branches are
    /// themselves generator-statement blocks; a missing `else` yields nothing.
    If {
        cond: Expr,
        then_body: Vec<GenStmt>,
        else_body: Vec<GenStmt>,
        span: Span,
    },
}

#[derive(Debug, PartialEq)]
pub enum TupleItem {
    Named {
        name: String,
        value: Expr,
        span: Span,
    },
    Positional(Expr),
}

#[derive(Debug, PartialEq)]
pub struct LocalBinding {
    pub name: String,
    pub annotation: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Add,
    Sub,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineDir {
    Forward,
    Backward,
}

#[derive(Debug, PartialEq)]
pub enum ImportSource {
    String(String),
    Path(Vec<String>),
}

#[derive(Debug, PartialEq)]
pub struct SelectField {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct EffectOp {
    pub path: Vec<String>,
    pub payload: Option<Box<TypeExpr>>,
    pub signature: Option<Box<TypeExpr>>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct EffectRow {
    pub ops: Vec<EffectOp>,
    /// Named effect-row spreads written before the final tail, such as
    /// `{ ...FsRead; ...e; }`. Each spread is resolved in HIR; non-final
    /// anonymous or row-variable spreads are rejected later.
    pub spreads: Vec<RowTail>,
    /// An optional open row tail `...e` (a row variable) or `...` (anonymous
    /// open), or a final named spread `...FsRead`, mirroring record/union row
    /// tails. `None` is a closed row.
    pub tail: Option<RowTail>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct HandleClause {
    pub op: Vec<String>,
    pub body: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub enum Expr {
    True(Span),
    False(Span),
    Integer {
        value: i64,
        postfix: Option<NumberType>,
        span: Span,
    },
    Float {
        value: f64,
        postfix: Option<NumberType>,
        span: Span,
    },
    Posit {
        literal: PositLiteral,
        span: Span,
    },
    String {
        value: String,
        span: Span,
    },
    Atom {
        name: String,
        span: Span,
    },
    /// `import "path"` / `import a.b.c` — a static import. The source is always a
    /// literal (string or dotted path), never a runtime value, so resolution is
    /// pure and static regardless of where the expression appears. The canonical
    /// uses are `lib ::= import "lib.zt";` and `{ map; fold } ::= import stdlib.stream;`.
    Import {
        source: ImportSource,
        span: Span,
    },
    TaggedValue {
        tag: String,
        payload: Box<Expr>,
        span: Span,
    },
    Ident {
        name: String,
        span: Span,
    },
    Record {
        fields: Vec<RecordField>,
        span: Span,
    },
    RecordUpdate {
        receiver: Box<Expr>,
        fields: Vec<RecordField>,
        span: Span,
    },
    Tuple {
        items: Vec<TupleItem>,
        span: Span,
    },
    List {
        items: Vec<Expr>,
        span: Span,
    },
    Generator {
        body: Vec<GenStmt>,
        span: Span,
    },
    Block {
        bindings: Vec<LocalBinding>,
        result: Box<Expr>,
        span: Span,
    },
    Lambda {
        params: Vec<Pattern>,
        body: Box<Expr>,
        span: Span,
    },
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
        span: Span,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<FuncClause>,
        span: Span,
    },
    TypeForm {
        ty: Box<TypeExpr>,
        span: Span,
    },
    WitnessReflect {
        constraint: String,
        target: Box<TypeExpr>,
        span: Span,
    },
    Select {
        receiver: Box<Expr>,
        fields: Vec<SelectField>,
        span: Span,
    },
    Perform {
        op: Vec<String>,
        arg: Box<Expr>,
        span: Span,
    },
    Handle {
        expr: Box<Expr>,
        clauses: Vec<HandleClause>,
        span: Span,
    },
    Resume {
        value: Box<Expr>,
        span: Span,
    },
    Sequence {
        items: Vec<Expr>,
        span: Span,
    },
    Apply {
        func: Box<Expr>,
        arg: Box<Expr>,
        span: Span,
    },
    Access {
        receiver: Box<Expr>,
        field: String,
        span: Span,
    },
    OptAccess {
        receiver: Box<Expr>,
        field: String,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Pipeline {
        dir: PipelineDir,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::True(s) | Expr::False(s) => *s,
            Expr::Integer { span, .. }
            | Expr::Float { span, .. }
            | Expr::Posit { span, .. }
            | Expr::String { span, .. }
            | Expr::Atom { span, .. }
            | Expr::Import { span, .. }
            | Expr::TaggedValue { span, .. }
            | Expr::Ident { span, .. }
            | Expr::Record { span, .. }
            | Expr::RecordUpdate { span, .. }
            | Expr::Tuple { span, .. }
            | Expr::List { span, .. }
            | Expr::Generator { span, .. }
            | Expr::Block { span, .. }
            | Expr::Lambda { span, .. }
            | Expr::If { span, .. }
            | Expr::Match { span, .. }
            | Expr::TypeForm { span, .. }
            | Expr::WitnessReflect { span, .. }
            | Expr::Select { span, .. }
            | Expr::Perform { span, .. }
            | Expr::Handle { span, .. }
            | Expr::Resume { span, .. }
            | Expr::Sequence { span, .. }
            | Expr::Apply { span, .. }
            | Expr::Access { span, .. }
            | Expr::OptAccess { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Pipeline { span, .. } => *span,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Pattern {
    Wildcard(Span),
    Ident {
        name: String,
        span: Span,
    },
    True(Span),
    False(Span),
    Integer {
        value: i64,
        postfix: Option<NumberType>,
        span: Span,
    },
    Float {
        value: f64,
        postfix: Option<NumberType>,
        span: Span,
    },
    Posit {
        literal: PositLiteral,
        span: Span,
    },
    String {
        value: String,
        span: Span,
    },
    Atom {
        name: String,
        span: Span,
    },
    TaggedValue {
        tag: String,
        payload: Vec<RecordPatternField>,
        span: Span,
    },
    Tuple {
        items: Vec<TuplePatternItem>,
        span: Span,
    },
    ListNil {
        span: Span,
    },
    ListCons {
        head: Box<Pattern>,
        tail: Box<Pattern>,
        span: Span,
    },
    Record {
        fields: Vec<RecordPatternField>,
        span: Span,
    },
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(s) | Pattern::True(s) | Pattern::False(s) => *s,
            Pattern::Ident { span, .. }
            | Pattern::Integer { span, .. }
            | Pattern::Float { span, .. }
            | Pattern::Posit { span, .. }
            | Pattern::String { span, .. }
            | Pattern::Atom { span, .. }
            | Pattern::TaggedValue { span, .. }
            | Pattern::Tuple { span, .. }
            | Pattern::ListNil { span }
            | Pattern::ListCons { span, .. }
            | Pattern::Record { span, .. } => *span,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum TuplePatternItem {
    Named {
        name: String,
        pattern: Pattern,
        span: Span,
    },
    Positional(Pattern),
}

#[derive(Debug, PartialEq)]
pub struct RecordPatternField {
    pub name: String,
    pub pattern: Pattern,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct UnionVariant {
    pub name: String,
    pub payload: Option<Box<TypeExpr>>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct TypeRecordField {
    pub name: String,
    pub optional: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub enum RowTail {
    Anonymous { span: Span },
    Named { name: String, span: Span },
    Qualified { path: Vec<String>, span: Span },
}

impl RowTail {
    pub fn span(&self) -> Span {
        match self {
            RowTail::Anonymous { span }
            | RowTail::Named { span, .. }
            | RowTail::Qualified { span, .. } => *span,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum TypeTupleItem {
    Named {
        name: String,
        ty: TypeExpr,
        span: Span,
    },
    Positional(TypeExpr),
}

#[derive(Debug, PartialEq)]
pub enum TypeExpr {
    Ident {
        name: String,
        span: Span,
    },
    Record {
        fields: Vec<TypeRecordField>,
        tail: Option<RowTail>,
        span: Span,
    },
    Union {
        variants: Vec<UnionVariant>,
        tail: Option<RowTail>,
        span: Span,
    },
    Tuple {
        items: Vec<TypeTupleItem>,
        span: Span,
    },
    Optional {
        inner: Box<TypeExpr>,
        span: Span,
    },
    Arrow {
        from: Box<TypeExpr>,
        to: Box<TypeExpr>,
        span: Span,
    },
    Effect {
        base: Box<TypeExpr>,
        effects: EffectRow,
        span: Span,
    },
    Select {
        receiver: Box<TypeExpr>,
        fields: Vec<SelectField>,
        span: Span,
    },
    Apply {
        func: Box<TypeExpr>,
        arg: Box<TypeExpr>,
        span: Span,
    },
    Access {
        receiver: Box<TypeExpr>,
        field: String,
        span: Span,
    },
    Atom {
        name: String,
        span: Span,
    },
    True(Span),
    False(Span),
    ForAll {
        params: Vec<TypeParam>,
        body: Box<TypeExpr>,
        span: Span,
    },
    /// A universe at an explicit level: `$0`, `$l`, `$(l + 1)`, `$(max a b)`.
    /// Bare `Type` stays `TypeExpr::Ident` (inferred level); this node only
    /// appears for the `$`-sigil form.
    UniverseType {
        level: Level,
        span: Span,
    },
    ExprEscape(Box<Expr>),
}

/// A universe level expression — surface sugar over the internal `UniverseLevel`
/// algebra (`Known | Meta | Succ | Max`). Levels erase before Dataflow Core.
#[derive(Debug, PartialEq)]
pub enum Level {
    /// `$0` — a concrete universe level.
    Known { value: u32, span: Span },
    /// `$l` — a level variable, bound by a `<$l>` binder.
    Var { name: String, span: Span },
    /// `$(l + n)` — `n` nested successors over a level atom.
    Succ {
        base: Box<Level>,
        by: u32,
        span: Span,
    },
    /// `$(max a b)` — the least upper bound of two levels (binary).
    Max {
        left: Box<Level>,
        right: Box<Level>,
        span: Span,
    },
}

impl Level {
    pub fn span(&self) -> Span {
        match self {
            Level::Known { span, .. }
            | Level::Var { span, .. }
            | Level::Succ { span, .. }
            | Level::Max { span, .. } => *span,
        }
    }
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::True(s) | TypeExpr::False(s) => *s,
            TypeExpr::Ident { span, .. }
            | TypeExpr::Record { span, .. }
            | TypeExpr::Union { span, .. }
            | TypeExpr::Tuple { span, .. }
            | TypeExpr::Optional { span, .. }
            | TypeExpr::Arrow { span, .. }
            | TypeExpr::Effect { span, .. }
            | TypeExpr::Select { span, .. }
            | TypeExpr::Apply { span, .. }
            | TypeExpr::Access { span, .. }
            | TypeExpr::Atom { span, .. }
            | TypeExpr::ForAll { span, .. }
            | TypeExpr::UniverseType { span, .. } => *span,
            TypeExpr::ExprEscape(e) => e.span(),
        }
    }
}
