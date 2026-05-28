use text_size::{TextRange, TextSize};

// ── Severity ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

// ── ErrorCode ─────────────────────────────────────────────────────────────────

/// Maps to the error classes in `docs/v0_spec/08-reference/error-model.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Lexical,
    Parse,
    DuplicateBinding,
    DuplicateKey,
    UnknownIdentifier,
    UnknownField,
    TypeMismatch,
    NonExhaustiveMatch,
    InvalidImportPath,
    ReservedName,
    CapitalizationConvention,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "E0001",
            Self::Parse => "E0002",
            Self::DuplicateBinding => "E0010",
            Self::DuplicateKey => "E0011",
            Self::UnknownIdentifier => "E0020",
            Self::UnknownField => "E0021",
            Self::TypeMismatch => "E0030",
            Self::NonExhaustiveMatch => "E0031",
            Self::InvalidImportPath => "E0040",
            Self::ReservedName => "E0050",
            Self::CapitalizationConvention => "W0001",
        }
    }
}

// ── Label ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Label {
    pub range: TextRange,
    pub message: String,
}

// ── Diagnostic ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub range: TextRange,
    pub severity: Severity,
    pub code: ErrorCode,
    pub message: String,
    pub labels: Vec<Label>,
}

impl Diagnostic {
    pub fn error(range: TextRange, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Error,
            code,
            message: message.into(),
            labels: vec![],
        }
    }

    pub fn warning(range: TextRange, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Warning,
            code,
            message: message.into(),
            labels: vec![],
        }
    }

    pub fn parse_error(offset: TextSize, message: impl Into<String>) -> Self {
        Self::error(TextRange::empty(offset), ErrorCode::Parse, message)
    }

    pub fn lexical_error(range: TextRange, message: impl Into<String>) -> Self {
        Self::error(range, ErrorCode::Lexical, message)
    }

    pub fn with_label(mut self, range: TextRange, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            range,
            message: message.into(),
        });
        self
    }
}

// ── Renderer (behind the `render` feature) ────────────────────────────────────

#[cfg(feature = "render")]
pub mod render;
