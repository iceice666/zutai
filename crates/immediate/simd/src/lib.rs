//! SIMD-accelerated parsing support for Zutai immediate mode (`.zti`).
//!
//! This crate is intended to contain the high-throughput parser for
//! immediate-mode documents. It focuses on fast structural scanning and parsing
//! of Zutai's inert data literal format: records, lists, atoms, strings,
//! numbers, booleans, and `none`.
