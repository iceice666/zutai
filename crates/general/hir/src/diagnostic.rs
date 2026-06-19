use zutai_syntax::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirDiagnostic {
    pub kind: HirDiagnosticKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HirDiagnosticKind {
    DuplicateBinding {
        name: String,
        first_span: Span,
    },
    DuplicateRecordField {
        name: String,
        first_span: Span,
    },
    DuplicateTypeRecordField {
        name: String,
        first_span: Span,
    },
    DuplicateRecordPatternField {
        name: String,
        first_span: Span,
    },
    DuplicateTupleField {
        name: String,
        first_span: Span,
    },
    DuplicateTypeTupleField {
        name: String,
        first_span: Span,
    },
    DuplicateTuplePatternField {
        name: String,
        first_span: Span,
    },
    UnknownIdentifier {
        name: String,
    },
    DuplicateConstraintMethod {
        name: String,
        first_span: Span,
    },
    DuplicateWitnessField {
        name: String,
        first_span: Span,
    },
    UnknownConstraint {
        name: String,
    },
    /// A `select` projection names the same field more than once.
    DuplicateSelectField {
        name: String,
        first_span: Span,
    },
    /// A named row tail `...Name` resolves to a binding that is neither a type
    /// parameter (row variable) nor a type (spread).
    InvalidRowTailTarget {
        name: String,
    },
    /// `resume` appears outside an operation handler clause body.
    ResumeOutsideHandler,
}
