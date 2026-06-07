//! Syntax support for Zutai general mode (`.zt`).
//!
//! This crate contains the parser and AST definitions for general-mode files.
//! See [`parse`] for the entry point.

pub mod ast;
pub mod error;
pub mod parser;
pub mod span;

mod display;

#[cfg(test)]
mod tests;

pub use ast::File;
pub use error::{ParseError, ParseErrorKind};
pub use span::Span;

/// Parse a `.zt` source file.
///
/// Returns the syntax tree on success or a list of errors on failure.
pub fn parse(input: &str) -> Result<File, Vec<ParseError>> {
    parser::parse(input)
}
