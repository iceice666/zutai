use zutai_syntax::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirDiagnostic {
    pub kind: HirDiagnosticKind,
    pub span: Span,
}

impl HirDiagnostic {
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    pub fn related_location(&self) -> Option<(Span, &'static str)> {
        self.kind.related_location()
    }
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
    /// A named row tail `...Name` must resolve to an in-scope type parameter.
    /// Type aliases now use explicit row-spread syntax, `* Name`.
    InvalidRowTailTarget {
        name: String,
    },
    /// `resume` appears outside an operation handler clause body.
    ResumeOutsideHandler,
    /// `yield from` appears outside tail position in a `stream { … }` generator.
    /// The codata `Stream` cell has no shared append, so a delegating yield is
    /// only sound when it is the block's final statement and nothing follows.
    NonTailYieldFrom,
    /// A level variable (declared `<$l>`) is used in type position, e.g. a bare
    /// `l` where a type is expected.
    LevelVarAsType {
        name: String,
    },
    /// A `$`-prefixed name in level position resolves to a binding that is not a
    /// level variable (e.g. `$A` where `A` is a type parameter).
    NonLevelAsLevel {
        name: String,
    },
    /// A `$`-prefixed level reference does not resolve to any declared level
    /// variable.
    UnknownLevelVar {
        name: String,
    },
    /// A declared level variable `<$l>` is never used in the signature.
    UnusedLevelParam {
        name: String,
    },
}

impl HirDiagnosticKind {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DuplicateBinding { .. } => "zutai::hir::duplicate_binding",
            Self::DuplicateRecordField { .. } => "zutai::hir::duplicate_record_field",
            Self::DuplicateTypeRecordField { .. } => "zutai::hir::duplicate_type_record_field",
            Self::DuplicateRecordPatternField { .. } => {
                "zutai::hir::duplicate_record_pattern_field"
            }
            Self::DuplicateTupleField { .. } => "zutai::hir::duplicate_tuple_field",
            Self::DuplicateTypeTupleField { .. } => "zutai::hir::duplicate_type_tuple_field",
            Self::DuplicateTuplePatternField { .. } => "zutai::hir::duplicate_tuple_pattern_field",
            Self::UnknownIdentifier { .. } => "zutai::hir::unknown_identifier",
            Self::DuplicateConstraintMethod { .. } => "zutai::hir::duplicate_constraint_method",
            Self::DuplicateWitnessField { .. } => "zutai::hir::duplicate_witness_field",
            Self::UnknownConstraint { .. } => "zutai::hir::unknown_constraint",
            Self::DuplicateSelectField { .. } => "zutai::hir::duplicate_select_field",
            Self::InvalidRowTailTarget { .. } => "zutai::hir::invalid_row_tail_target",
            Self::ResumeOutsideHandler => "zutai::hir::resume_outside_handler",
            Self::NonTailYieldFrom => "zutai::hir::non_tail_yield_from",
            Self::LevelVarAsType { .. } => "zutai::hir::level_var_as_type",
            Self::NonLevelAsLevel { .. } => "zutai::hir::non_level_as_level",
            Self::UnknownLevelVar { .. } => "zutai::hir::unknown_level_var",
            Self::UnusedLevelParam { .. } => "zutai::hir::unused_level_param",
        }
    }

    fn related_location(&self) -> Option<(Span, &'static str)> {
        match self {
            Self::DuplicateBinding { first_span, .. }
            | Self::DuplicateRecordField { first_span, .. }
            | Self::DuplicateTypeRecordField { first_span, .. }
            | Self::DuplicateRecordPatternField { first_span, .. }
            | Self::DuplicateTupleField { first_span, .. }
            | Self::DuplicateTypeTupleField { first_span, .. }
            | Self::DuplicateTuplePatternField { first_span, .. }
            | Self::DuplicateConstraintMethod { first_span, .. }
            | Self::DuplicateWitnessField { first_span, .. }
            | Self::DuplicateSelectField { first_span, .. } => {
                Some((*first_span, "first occurrence"))
            }
            _ => None,
        }
    }
}
