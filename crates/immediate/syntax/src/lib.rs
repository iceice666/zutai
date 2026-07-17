//! Syntax support for Zutai immediate mode (`.zti`).
//!
//! This crate contains the `.zti` parser and re-exports the shared AST types
//! used by immediate mode.

pub mod format;
pub mod parser;

pub use format::format_source;

#[cfg(test)]
mod tests;
