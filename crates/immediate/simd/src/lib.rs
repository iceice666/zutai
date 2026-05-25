//! SIMD-accelerated parsing support for Zutai immediate mode (`.zti`).
//!
//! This crate is intended to contain the high-throughput parser for
//! immediate-mode documents. It focuses on fast structural scanning and parsing
//! of Zutai's inert data literal format: records, lists, atoms, strings,
//! numbers, booleans, and `none`.

mod charclass;
mod error;
mod parser;
mod scanner;

pub use error::{ParseError, ParseErrorKind};
pub use scanner::{ChunkScan, StructuralIndex};

use zutai_types::Block;

pub fn scan(input: &str) -> Result<StructuralIndex, ParseError> {
    scanner::Scanner::new(input).scan()
}

pub fn parse(input: &str) -> Result<Block, ParseError> {
    let index = scan(input)?;
    parser::Parser::new(input, &index).parse_document()
}

#[cfg(test)]
mod tests;
