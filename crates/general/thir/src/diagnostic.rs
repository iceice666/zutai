use zutai_syntax::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirDiagnostic {
    pub kind: ThirDiagnosticKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOverlapItem {
    RecordField,
    UnionMember,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThirDiagnosticKind {
    TypeCheckerNotImplemented,
    UnsupportedFeature {
        feature: &'static str,
    },
    TypeMismatch {
        expected: String,
        found: String,
    },
    ExpectedFunction {
        found: String,
    },
    FunctionClauseArityMismatch {
        expected: usize,
        found: usize,
    },
    ExpectedRecord {
        found: String,
    },
    ExpectedList {
        found: String,
    },
    ExpectedTuple {
        found: String,
    },
    ExpectedOptionalOrMaybe {
        found: String,
    },
    EmptyListNeedsType,
    TupleArityMismatch {
        expected: usize,
        found: usize,
    },
    TupleFieldNameMismatch {
        expected: String,
        found: String,
    },
    InvalidBinaryOperands {
        op: &'static str,
        lhs: String,
        rhs: String,
    },
    NumericLiteralOutOfRange {
        value: i64,
        ty: String,
    },
    MissingRecordField {
        name: String,
    },
    UnexpectedRecordField {
        name: String,
    },
    UnknownField {
        name: String,
    },
    AliasCycle {
        name: String,
    },
    /// A parametric type constructor was applied to the wrong number of arguments.
    TypeConstructorArityMismatch {
        name: String,
        expected: usize,
        found: usize,
    },
    /// Type-level alias expansion exceeded the deterministic evaluation budget.
    TypeLevelEvalLimitExceeded,
    UniverseLevelCycle {
        name: String,
    },
    /// An explicit universe level (`$ℓ`) is lower than the universe the annotated
    /// definition inhabits (e.g. `Bad :: $0 = $0`, where `$0 : $1`).
    ExplicitLevelTooLow {
        required: u32,
        found: u32,
    },
    ValueTypeUnavailable {
        name: String,
    },
    InvalidTypeExpression {
        reason: &'static str,
    },
    LambdaNeedsTypeContext,
    MatchArmPatternCountMismatch {
        found: usize,
    },
    /// A `match` or multi-clause function does not cover every possible value of
    /// the scrutinee type. `witness` is a rendered example of an unmatched
    /// pattern (e.g. `#dev`, `(#square, ...)`, or `_`).
    NonExhaustiveMatch {
        witness: String,
    },
    /// A `match` arm or function clause can never be reached because earlier
    /// unguarded arms already cover every value its pattern would match.
    UnreachableMatchArm,
    WitnessFieldTypeMismatch {
        name: String,
        expected: String,
        found: String,
    },
    MissingWitnessField {
        name: String,
    },
    UnknownWitnessField {
        name: String,
    },
    DeriveConstraintNotDerivable {
        constraint: String,
    },
    DeriveComponentMissingWitness {
        constraint: String,
        component: String,
    },
    DeriveUnsupportedMethod {
        constraint: String,
        method: String,
    },
    WitnessReflectNotInScope {
        constraint: String,
        target: String,
    },
    DeriveRecipeFuelExhausted {
        constraint: String,
    },
    DeriveRecipeTypeMismatch {
        constraint: String,
        method: String,
        expected: String,
        found: String,
    },
    /// Two witnesses claim the same `(Constraint, Type)` pair.
    ConflictingWitness {
        constraint: String,
        target: String,
    },
    /// A conditional witness whose target is one of its own type parameters
    /// (e.g. `Eq @A :: <A: Eq>`). Resolving such a witness for a type requires a
    /// witness for the same type, so the search never terminates.
    RecursiveWitness {
        constraint: String,
    },
    /// A witness target's kind does not match the constraint's expected kind
    /// (e.g. `Functor @Int` where `Functor` constrains a `Type -> Type`).
    WitnessTargetKindMismatch {
        constraint: String,
        target: String,
    },
    /// A constraint definition uses more than one type parameter.  Multi-param
    /// constraints are not yet supported; witness checking is skipped for them.
    UnsupportedMultiParamConstraint {
        name: String,
    },
    /// A row tail (spread or row variable) would introduce a field or member
    /// that the row already declares explicitly.
    OverlappingRowField {
        item: RowOverlapItem,
        source: String,
        name: String,
        existing: String,
        incoming: String,
    },
    /// Row-polymorphic inference is not principal here; an explicit type
    /// annotation is required. `field` is present for field-access cases such as
    /// `x.host`, where the unknown receiver type blocks principal row inference.
    RowAnnotationRequired {
        field: Option<String>,
    },
    EffectNotInRow {
        op: String,
    },
    MalformedEffectOp {
        op: String,
        reason: &'static str,
    },
    ResumeTypeMismatch {
        expected: String,
        found: String,
    },
    HandlerClauseArityMismatch {
        op: String,
        expected: usize,
        found: usize,
    },
    MultipleResume {
        op: String,
    },
    /// Unifying an inference variable with a type that contains it would build an
    /// infinite type (e.g. self-application `\x. x x`). Rejected by the occurs check.
    InfiniteType,
}
