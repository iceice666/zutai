use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    ChainedComparison,
    MixedPipeline,
    LambdaArrow,
    LambdaDotNeedsWhitespace,
    MissingListItemSemicolon,
    MissingBlockResult,
    ValueRecordFieldUsesColon,
    TopLevelSingleColon,
    TypeRecordFieldUsesEquals,
    TrailingOperator,
    MissingFieldAfterAccess,
    LocalBindingDoubleColon,
    TaggedValuePayloadUsesColon,
    TypeUnionPayloadUsesEquals,
    StaleTypeDeclaration,
    UnknownNumberPostfix,
    IntegerPostfixOnFloatLiteral,
    UnsignedPostfixOnNegative,
    ExpectedToken(&'static str),
    ExpectedExpression,
    ExpectedPattern,
    ExpectedType,
    UnclosedDelimiter(char),
    TrailingInput,
    Generic,
}

impl ParseErrorKind {
    pub fn message(&self) -> &'static str {
        match self {
            ParseErrorKind::ChainedComparison => "comparison operators are non-associative in v0",
            ParseErrorKind::MixedPipeline => {
                "a single pipeline chain cannot mix forward and backward directions"
            }
            ParseErrorKind::LambdaArrow => "general-mode lambdas use `.` rather than `=>`",
            ParseErrorKind::LambdaDotNeedsWhitespace => {
                "lambda `.` must be followed by whitespace before the body"
            }
            ParseErrorKind::MissingListItemSemicolon => "list items must end with `;`",
            ParseErrorKind::MissingBlockResult => {
                "block expressions require a final result expression"
            }
            ParseErrorKind::ValueRecordFieldUsesColon => "value record fields use `=`, not `:`",
            ParseErrorKind::TopLevelSingleColon => "top-level typed bindings use `::`, not `:`",
            ParseErrorKind::TypeRecordFieldUsesEquals => "type record fields use `:`, not `=`",
            ParseErrorKind::TrailingOperator => "binary operator requires a right-hand operand",
            ParseErrorKind::MissingFieldAfterAccess => "field access requires a field name",
            ParseErrorKind::LocalBindingDoubleColon => "local typed bindings use `:`, not `::`",
            ParseErrorKind::TaggedValuePayloadUsesColon => {
                "tagged values attach payloads without `:`"
            }
            ParseErrorKind::TypeUnionPayloadUsesEquals => "union variant payloads use `:`, not `=`",
            ParseErrorKind::StaleTypeDeclaration => {
                "type declarations use `Name :: type`, not `type Name =`"
            }
            ParseErrorKind::UnknownNumberPostfix => "unknown numeric type postfix",
            ParseErrorKind::IntegerPostfixOnFloatLiteral => {
                "integer type postfix on a non-integer literal"
            }
            ParseErrorKind::UnsignedPostfixOnNegative => {
                "unsigned type postfix on a negative literal"
            }
            ParseErrorKind::ExpectedToken(_) => "expected token",
            ParseErrorKind::ExpectedExpression => "expected expression",
            ParseErrorKind::ExpectedPattern => "expected pattern",
            ParseErrorKind::ExpectedType => "expected type",
            ParseErrorKind::UnclosedDelimiter(_) => "unclosed delimiter",
            ParseErrorKind::TrailingInput => "trailing input",
            ParseErrorKind::Generic => "parse error",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ParseErrorKind::ChainedComparison => "second comparison in the same expression",
            ParseErrorKind::MixedPipeline => "pipeline direction changes here",
            ParseErrorKind::LambdaArrow => "use `.` here",
            ParseErrorKind::LambdaDotNeedsWhitespace => "add whitespace after this `.`",
            ParseErrorKind::MissingListItemSemicolon => "missing `;` before this delimiter",
            ParseErrorKind::MissingBlockResult => {
                "block ends after bindings with no result expression"
            }
            ParseErrorKind::ValueRecordFieldUsesColon => "use `=` for value fields",
            ParseErrorKind::TopLevelSingleColon => "use `::` for a typed top-level binding",
            ParseErrorKind::TypeRecordFieldUsesEquals => "use `:` for type fields",
            ParseErrorKind::TrailingOperator => "operator has no right-hand operand",
            ParseErrorKind::MissingFieldAfterAccess => {
                "expected a field name after this access operator"
            }
            ParseErrorKind::LocalBindingDoubleColon => "use `:` for a typed local binding",
            ParseErrorKind::TaggedValuePayloadUsesColon => "remove `:` after this tag",
            ParseErrorKind::TypeUnionPayloadUsesEquals => "use `:` for a variant payload type",
            ParseErrorKind::StaleTypeDeclaration => "stale type declaration syntax starts here",
            ParseErrorKind::UnknownNumberPostfix => "not one of i8…f64",
            ParseErrorKind::IntegerPostfixOnFloatLiteral => "this body has a fraction or exponent",
            ParseErrorKind::UnsignedPostfixOnNegative => {
                "remove the leading `-` or use a signed type"
            }
            ParseErrorKind::ExpectedToken(token) => token,
            ParseErrorKind::ExpectedExpression => "expected an expression here",
            ParseErrorKind::ExpectedPattern => "expected a pattern here",
            ParseErrorKind::ExpectedType => "expected a type here",
            ParseErrorKind::UnclosedDelimiter(_) => "delimiter opened here",
            ParseErrorKind::TrailingInput => "unexpected input starts here",
            ParseErrorKind::Generic => "parser stopped here",
        }
    }

    pub fn help(&self) -> Option<&'static str> {
        match self {
            ParseErrorKind::ChainedComparison => Some("parenthesize comparisons explicitly"),
            ParseErrorKind::MixedPipeline => {
                Some("split the expression or use one pipeline direction")
            }
            ParseErrorKind::LambdaArrow => Some("write lambdas as `\\x. body`"),
            ParseErrorKind::LambdaDotNeedsWhitespace => Some("write `\\x. body`, not `\\x.body`"),
            ParseErrorKind::MissingListItemSemicolon => Some("write list values like `[1; 2; 3;]`"),
            ParseErrorKind::MissingBlockResult => {
                Some("add a final expression after the local bindings")
            }
            ParseErrorKind::ValueRecordFieldUsesColon => {
                Some("write value records as `{ field = value; }`")
            }
            ParseErrorKind::TopLevelSingleColon => Some("write `name :: Type = value`"),
            ParseErrorKind::TypeRecordFieldUsesEquals => {
                Some("write type records as `type { field : Type; }`")
            }
            ParseErrorKind::TrailingOperator => {
                Some("add the missing expression after the operator or remove the operator")
            }
            ParseErrorKind::MissingFieldAfterAccess => {
                Some("write `value.field` or remove the trailing access operator")
            }
            ParseErrorKind::LocalBindingDoubleColon => {
                Some("write `[name : Type = value; result]`")
            }
            ParseErrorKind::TaggedValuePayloadUsesColon => {
                Some("write `#tag { field = value; }` or `#tag (value)`, not `#tag : ...`")
            }
            ParseErrorKind::TypeUnionPayloadUsesEquals => {
                Some("write `#tag : Payload;` in union types")
            }
            ParseErrorKind::StaleTypeDeclaration => Some("write `Name :: type TypeExpr;`"),
            _ => None,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ParseErrorKind::ChainedComparison => "zutai::parse::chained_comparison",
            ParseErrorKind::MixedPipeline => "zutai::parse::mixed_pipeline",
            ParseErrorKind::LambdaArrow => "zutai::parse::lambda_arrow",
            ParseErrorKind::LambdaDotNeedsWhitespace => "zutai::parse::lambda_dot_whitespace",
            ParseErrorKind::MissingListItemSemicolon => "zutai::parse::missing_list_item_semicolon",
            ParseErrorKind::MissingBlockResult => "zutai::parse::missing_block_result",
            ParseErrorKind::ValueRecordFieldUsesColon => "zutai::parse::value_record_field_colon",
            ParseErrorKind::TopLevelSingleColon => "zutai::parse::top_level_single_colon",
            ParseErrorKind::TypeRecordFieldUsesEquals => "zutai::parse::type_record_field_equals",
            ParseErrorKind::TrailingOperator => "zutai::parse::trailing_operator",
            ParseErrorKind::MissingFieldAfterAccess => "zutai::parse::missing_field_after_access",
            ParseErrorKind::LocalBindingDoubleColon => "zutai::parse::local_binding_double_colon",
            ParseErrorKind::TaggedValuePayloadUsesColon => {
                "zutai::parse::tagged_value_payload_colon"
            }
            ParseErrorKind::TypeUnionPayloadUsesEquals => "zutai::parse::type_union_payload_equals",
            ParseErrorKind::StaleTypeDeclaration => "zutai::parse::stale_type_declaration",
            ParseErrorKind::UnknownNumberPostfix => "zutai::parse::unknown_number_postfix",
            ParseErrorKind::IntegerPostfixOnFloatLiteral => {
                "zutai::parse::integer_postfix_on_float_literal"
            }
            ParseErrorKind::UnsignedPostfixOnNegative => {
                "zutai::parse::unsigned_postfix_on_negative"
            }
            ParseErrorKind::ExpectedToken(_) => "zutai::parse::expected_token",
            ParseErrorKind::ExpectedExpression => "zutai::parse::expected_expression",
            ParseErrorKind::ExpectedPattern => "zutai::parse::expected_pattern",
            ParseErrorKind::ExpectedType => "zutai::parse::expected_type",
            ParseErrorKind::UnclosedDelimiter(_) => "zutai::parse::unclosed_delimiter",
            ParseErrorKind::TrailingInput => "zutai::parse::trailing_input",
            ParseErrorKind::Generic => "zutai::parse::generic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
    pub expected: Vec<&'static str>,
    pub kind: ParseErrorKind,
}

impl ParseError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            expected: vec![],
            kind: ParseErrorKind::Generic,
        }
    }

    pub fn from_kind(span: Span, kind: ParseErrorKind) -> Self {
        let message = match &kind {
            ParseErrorKind::ExpectedToken(token) => format!("expected `{token}`"),
            ParseErrorKind::UnclosedDelimiter(delim) => format!("unclosed `{delim}` delimiter"),
            _ => kind.message().to_string(),
        };
        Self {
            span,
            message,
            expected: vec![],
            kind,
        }
    }

    pub fn with_expected(mut self, expected: Vec<&'static str>) -> Self {
        self.expected = expected;
        self
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}..{}] {}",
            self.span.start, self.span.end, self.message
        )?;
        if !self.expected.is_empty() {
            write!(f, " (expected: {})", self.expected.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}
