//! SIMD-accelerated parsing support for Zutai immediate mode (`.zti`).
//!
//! This crate is intended to contain the high-throughput parser for
//! immediate-mode documents. It focuses on fast structural scanning and parsing
//! of Zutai's inert data literal format: records, lists, atoms, strings,
//! numbers, booleans, and atoms.

mod charclass;
mod error;
mod parser;
mod scanner;
mod string_scan;

pub use error::{ParseError, ParseErrorKind};
pub use scanner::{ChunkScan, StructuralIndex};

use zutai_types::Block;

pub fn scan(input: &str) -> Result<StructuralIndex, ParseError> {
    scanner::Scanner::new(input).scan()
}

pub fn parse(input: &str) -> Result<Block, ParseError> {
    let significant = scanner::Scanner::new(input).scan_significant()?;
    parser::Parser::new(input, &significant).parse_document()
}

/// Parses using the SSE2 structural-scanner backend, bypassing runtime
/// backend detection. SSE2 is baseline on x86_64, so this is always safe.
#[cfg(target_arch = "x86_64")]
pub fn parse_sse2(input: &str) -> Result<Block, ParseError> {
    let significant = scanner::Scanner::new(input).scan_significant_sse2()?;
    parser::Parser::new(input, &significant).parse_document()
}

/// Parses using the AVX2 structural-scanner backend, bypassing runtime
/// backend detection.
///
/// # Safety
/// The current process must support AVX2
/// (`std::is_x86_feature_detected!("avx2")`).
#[cfg(target_arch = "x86_64")]
pub unsafe fn parse_avx2(input: &str) -> Result<Block, ParseError> {
    // SAFETY: the caller guarantees AVX2 support for this process.
    let significant = unsafe { scanner::Scanner::new(input).scan_significant_avx2() }?;
    parser::Parser::new(input, &significant).parse_document()
}

#[cfg(test)]
mod tests;
