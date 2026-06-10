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
}
