use zutai_syntax::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirDiagnostic {
    pub kind: ThirDiagnosticKind,
    pub span: Span,
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
    ExpectedOptional {
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
}
