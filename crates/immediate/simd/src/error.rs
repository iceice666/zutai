use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub kind: ParseErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    DuplicateField(String),
    InvalidAtom,
    InvalidEscape,
    InvalidNumber,
    InvalidString,
    Expected { expected: &'static str },
    TrailingData,
    UnclosedString,
}

pub(crate) fn error(offset: usize, kind: ParseErrorKind) -> ParseError {
    ParseError { offset, kind }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseErrorKind::DuplicateField(name) => {
                write!(f, "duplicate field `{name}` at byte {}", self.offset)
            }
            ParseErrorKind::InvalidAtom => write!(f, "invalid atom at byte {}", self.offset),
            ParseErrorKind::InvalidEscape => write!(f, "invalid escape at byte {}", self.offset),
            ParseErrorKind::InvalidNumber => write!(f, "invalid number at byte {}", self.offset),
            ParseErrorKind::InvalidString => write!(f, "invalid string at byte {}", self.offset),
            ParseErrorKind::Expected { expected } => {
                write!(f, "expected {expected} at byte {}", self.offset)
            }
            ParseErrorKind::TrailingData => write!(f, "trailing data at byte {}", self.offset),
            ParseErrorKind::UnclosedString => write!(f, "unclosed string at byte {}", self.offset),
        }
    }
}

impl Error for ParseError {}
