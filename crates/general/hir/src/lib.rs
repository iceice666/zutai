//! High-level intermediate representation (HIR) for Zutai general mode (`.zt`).
//!
//! The HIR is a rich, near-surface, arena-allocated representation that sits
//! between the lossless CST (`zutai-syntax`) and the semantic passes in
//! `zutai-semantic`. It:
//!
//! - Resolves names to `SymbolId`s (M1).
//! - Desugars pure sugar: pipelines `|>/<|`, `??`, `?.`, multi-clause functions.
//! - Keeps everything else intact for readable diagnostics.
//!
//! # Entry point
//!
//! ```no_run
//! use zutai_hir::lower::lower_file;
//! use zutai_syntax::parse;
//!
//! let parsed = parse("x := 42\nx");
//! let (hir, diags) = lower_file(&parsed.syntax());
//! ```

pub mod arena;
pub mod decl;
pub mod expr;
pub mod file;
pub mod lower;
pub mod pat;
pub mod scope;
pub mod symbol;
pub mod ty;

pub use file::HirFile;
pub use lower::lower_file;
pub use symbol::{Symbol, SymbolId, SymbolKind, SymbolTable};
