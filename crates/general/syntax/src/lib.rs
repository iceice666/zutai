//! Syntax support for Zutai general mode (`.zt`).
//!
//! This crate is intended to contain the parser and AST definitions for
//! general-mode files. General mode parses zero or more top-level declarations
//! followed by a final expression, including records, tuples, lists, imports,
//! functions, types, conditionals, pattern matching, field access, and operators.
