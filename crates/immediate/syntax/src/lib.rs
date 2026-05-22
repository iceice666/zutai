//! Syntax support for Zutai immediate mode (`.zti`).
//!
//! This crate contains the `.zti` parser and re-exports the shared AST types
//! used by immediate mode.

pub mod parser;

#[cfg(test)]
mod tests;
