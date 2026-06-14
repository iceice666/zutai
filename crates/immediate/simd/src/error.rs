#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[error("{kind} at byte {offset}")]
pub struct ParseError {
    pub offset: usize,
    pub kind: ParseErrorKind,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    #[error("duplicate field `{0}`")]
    DuplicateField(String),
    #[error("invalid atom")]
    InvalidAtom,
    #[error("invalid escape")]
    InvalidEscape,
    #[error("invalid number")]
    InvalidNumber,
    #[error("invalid string")]
    InvalidString,
    #[error("expected {expected}")]
    Expected { expected: &'static str },
    #[error("trailing data")]
    TrailingData,
    #[error("unclosed string")]
    UnclosedString,
}

pub(crate) fn error(offset: usize, kind: ParseErrorKind) -> ParseError {
    ParseError { offset, kind }
}
