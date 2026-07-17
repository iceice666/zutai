use zutai_syntax::Span;

use crate::import::ImportedTypeOrigin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirDiagnostic {
    pub kind: ThirDiagnosticKind,
    pub span: Span,
}

impl ThirDiagnostic {
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOverlapItem {
    RecordField,
    UnionMember,
    EffectOperation,
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
    ImportedDataTypeMismatch {
        expected: String,
        found: String,
        origin: ImportedTypeOrigin,
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
    SpreadOnlyLiteralNeedsType,
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
        /// Source location of the constraint's declaration (the "expansion
        /// definition"), distinct from the primary `span` at the derive request.
        definition: Span,
    },
    DeriveComponentMissingWitness {
        constraint: String,
        component: String,
        definition: Span,
    },
    DeriveUnsupportedMethod {
        constraint: String,
        method: String,
        definition: Span,
    },
    WitnessReflectNotInScope {
        constraint: String,
        target: String,
    },
    DeriveRecipeFuelExhausted {
        constraint: String,
        definition: Span,
    },
    /// A `Code`-typed derive recipe promised a witness record but its pure
    /// reducer could not reduce it to one (e.g. it stalls on arithmetic or a
    /// comparison the compile-time reducer does not evaluate). Refused rather
    /// than falling through to a structural witness the recipe never described.
    DeriveRecipeIrreducible {
        constraint: String,
        definition: Span,
    },
    /// A structural derive (`eq`/`show`/`compare`, or a structural-code recipe)
    /// targets an *open* record or union row. The visible members do not
    /// determine the full value, so a derived witness would be unsound. Refused
    /// to match the reflection and `FromData` open-row boundaries.
    DeriveOpenRowTarget {
        constraint: String,
        target: String,
        definition: Span,
    },
    DeriveRecipeTypeMismatch {
        constraint: String,
        method: String,
        expected: String,
        found: String,
        definition: Span,
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

impl ThirDiagnosticKind {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TypeCheckerNotImplemented => "zutai::thir::type_checker_not_implemented",
            Self::UnsupportedFeature { .. } => "zutai::thir::unsupported_feature",
            Self::TypeMismatch { .. } => "zutai::thir::type_mismatch",
            Self::ImportedDataTypeMismatch { .. } => "zutai::thir::imported_data_type_mismatch",
            Self::ExpectedFunction { .. } => "zutai::thir::expected_function",
            Self::FunctionClauseArityMismatch { .. } => {
                "zutai::thir::function_clause_arity_mismatch"
            }
            Self::ExpectedRecord { .. } => "zutai::thir::expected_record",
            Self::ExpectedList { .. } => "zutai::thir::expected_list",
            Self::ExpectedTuple { .. } => "zutai::thir::expected_tuple",
            Self::ExpectedOptionalOrMaybe { .. } => "zutai::thir::expected_optional_or_maybe",
            Self::EmptyListNeedsType => "zutai::thir::empty_list_needs_type",
            Self::SpreadOnlyLiteralNeedsType => "zutai::thir::spread_only_literal_needs_type",
            Self::TupleArityMismatch { .. } => "zutai::thir::tuple_arity_mismatch",
            Self::TupleFieldNameMismatch { .. } => "zutai::thir::tuple_field_name_mismatch",
            Self::InvalidBinaryOperands { .. } => "zutai::thir::invalid_binary_operands",
            Self::NumericLiteralOutOfRange { .. } => "zutai::thir::numeric_literal_out_of_range",
            Self::MissingRecordField { .. } => "zutai::thir::missing_record_field",
            Self::UnexpectedRecordField { .. } => "zutai::thir::unexpected_record_field",
            Self::UnknownField { .. } => "zutai::thir::unknown_field",
            Self::AliasCycle { .. } => "zutai::thir::alias_cycle",
            Self::TypeConstructorArityMismatch { .. } => {
                "zutai::thir::type_constructor_arity_mismatch"
            }
            Self::TypeLevelEvalLimitExceeded => "zutai::thir::type_level_eval_limit_exceeded",
            Self::UniverseLevelCycle { .. } => "zutai::thir::universe_level_cycle",
            Self::ExplicitLevelTooLow { .. } => "zutai::thir::explicit_level_too_low",
            Self::ValueTypeUnavailable { .. } => "zutai::thir::value_type_unavailable",
            Self::InvalidTypeExpression { .. } => "zutai::thir::invalid_type_expression",
            Self::LambdaNeedsTypeContext => "zutai::thir::lambda_needs_type_context",
            Self::MatchArmPatternCountMismatch { .. } => {
                "zutai::thir::match_arm_pattern_count_mismatch"
            }
            Self::NonExhaustiveMatch { .. } => "zutai::thir::non_exhaustive_match",
            Self::UnreachableMatchArm => "zutai::thir::unreachable_match_arm",
            Self::WitnessFieldTypeMismatch { .. } => "zutai::thir::witness_field_type_mismatch",
            Self::MissingWitnessField { .. } => "zutai::thir::missing_witness_field",
            Self::UnknownWitnessField { .. } => "zutai::thir::unknown_witness_field",
            Self::DeriveConstraintNotDerivable { .. } => {
                "zutai::thir::derive_constraint_not_derivable"
            }
            Self::DeriveComponentMissingWitness { .. } => {
                "zutai::thir::derive_component_missing_witness"
            }
            Self::DeriveUnsupportedMethod { .. } => "zutai::thir::derive_unsupported_method",
            Self::WitnessReflectNotInScope { .. } => "zutai::thir::witness_reflect_not_in_scope",
            Self::DeriveRecipeFuelExhausted { .. } => "zutai::thir::derive_recipe_fuel_exhausted",
            Self::DeriveRecipeIrreducible { .. } => "zutai::thir::derive_recipe_irreducible",
            Self::DeriveOpenRowTarget { .. } => "zutai::thir::derive_open_row_target",
            Self::DeriveRecipeTypeMismatch { .. } => "zutai::thir::derive_recipe_type_mismatch",
            Self::ConflictingWitness { .. } => "zutai::thir::conflicting_witness",
            Self::RecursiveWitness { .. } => "zutai::thir::recursive_witness",
            Self::WitnessTargetKindMismatch { .. } => "zutai::thir::witness_target_kind_mismatch",
            Self::UnsupportedMultiParamConstraint { .. } => {
                "zutai::thir::unsupported_multi_param_constraint"
            }
            Self::OverlappingRowField { .. } => "zutai::thir::overlapping_row_field",
            Self::RowAnnotationRequired { .. } => "zutai::thir::row_annotation_required",
            Self::EffectNotInRow { .. } => "zutai::thir::effect_not_in_row",
            Self::MalformedEffectOp { .. } => "zutai::thir::malformed_effect_op",
            Self::ResumeTypeMismatch { .. } => "zutai::thir::resume_type_mismatch",
            Self::HandlerClauseArityMismatch { .. } => "zutai::thir::handler_clause_arity_mismatch",
            Self::MultipleResume { .. } => "zutai::thir::multiple_resume",
            Self::InfiniteType => "zutai::thir::infinite_type",
        }
    }
}

impl ThirDiagnostic {
    /// Secondary "constraint defined here" location for a derive/recipe
    /// diagnostic, resolved against `source` — the entry buffer the primary span
    /// indexes — with a label describing its role. Derive/recipe failures point
    /// their primary `span` at the derivation *request* and carry the constraint
    /// declaration's span as the secondary *definition*.
    ///
    /// The `definition` span starts at the constraint's name token, so the label
    /// is emitted only when `source` at that offset spells the constraint name.
    /// A constraint declared in a prelude or imported module shares the THIR decl
    /// arena but spans a *different* buffer; that content check (mirroring
    /// `binding_range` in the LSP) keeps such a definition from mislocating a
    /// label into unrelated entry-file bytes. The returned span covers just the
    /// name token.
    pub fn related_location_in(&self, source: &str) -> Option<(Span, &'static str)> {
        use ThirDiagnosticKind::*;
        let (constraint, definition) = match &self.kind {
            DeriveConstraintNotDerivable {
                constraint,
                definition,
            }
            | DeriveComponentMissingWitness {
                constraint,
                definition,
                ..
            }
            | DeriveUnsupportedMethod {
                constraint,
                definition,
                ..
            }
            | DeriveRecipeFuelExhausted {
                constraint,
                definition,
            }
            | DeriveRecipeIrreducible {
                constraint,
                definition,
            }
            | DeriveOpenRowTarget {
                constraint,
                definition,
                ..
            }
            | DeriveRecipeTypeMismatch {
                constraint,
                definition,
                ..
            } => (constraint.as_str(), *definition),
            _ => return None,
        };
        let start = definition.start as usize;
        let end = start.checked_add(constraint.len())?;
        (source.get(start..end) == Some(constraint)).then_some((
            Span {
                start: definition.start,
                end: end as u32,
            },
            "constraint defined here",
        ))
    }
}
