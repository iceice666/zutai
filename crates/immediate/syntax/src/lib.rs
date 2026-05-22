//! Syntax support for Zutai immediate mode (`.zti`).
//!
//! This crate is intended to contain the parser and AST definitions for
//! immediate-mode documents. Immediate mode is Zutai's inert data literal
//! format: it parses records, lists, atoms, strings, numbers, booleans, and
//! `none` without imports, name resolution, functions, or evaluation.
