//! Pure lazy evaluator scaffold for Zutai general mode (`.zt`).
//!
//! This crate is intended to become the shared execution substrate for:
//!
//! - `zutai run file.zt`
//! - a pure general-mode REPL
//! - the future shell-mode surface/effect runner
//!
//! The evaluator is not implemented yet. The public API is deliberately shaped
//! around the eventual interpreter boundary: parse and analyze first, reject
//! diagnostics, then evaluate HIR in a persistent session.

mod error;
mod import;
mod session;
mod value;

pub use error::{EvalError, EvalErrorKind, EvalResult};
pub use import::{ImportCache, ImportEntry};
pub use session::{EvalConfig, EvalSession};
pub use value::{ClosureValue, EvalValue, ThunkState, ThunkValue};
