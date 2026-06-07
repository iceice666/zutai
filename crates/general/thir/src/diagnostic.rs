use zutai_syntax::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirDiagnostic {
    pub kind: ThirDiagnosticKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThirDiagnosticKind {
    TypeCheckerNotImplemented,
    UnsupportedFeature { feature: &'static str },
    TypeMismatch { expected: String, found: String },
    ExpectedRecord { found: String },
    MissingRecordField { name: String },
    UnexpectedRecordField { name: String },
    UnknownField { name: String },
    AliasCycle { name: String },
    ValueTypeUnavailable { name: String },
    InvalidTypeExpression { reason: &'static str },
}
