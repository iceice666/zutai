//! Reference evaluators for Zutai general mode (`.zt`).
//!
//! ## Design
//! This crate is a *semantics oracle*: it REFUSES to evaluate any program that
//! is not fully type-checked by THIR. The pre-flight gates guarantee that no
//! `ThirExprKind::Error` node is reachable before evaluation begins, so a
//! returned `Value` is always a faithful representation of what the program's
//! final expression evaluates to.
//!
//! ## IR-agnostic core
//! The modules `value`, `thunk`, and `env` remain independent of any specific IR.
//! `eval_tlc` is the default evaluator for executable value programs. `eval` is
//! the THIR regression oracle and the runtime `Type`/reflection boundary.
//! `eval_file`/`eval_with_base`/`eval_path` are TLC-first defaults; `eval_thir_*`
//! are explicit oracle APIs; `eval_tlc_*` are strict TLC APIs.
//!
//! ## Note on resource management
//! Top-level evaluation builds a `letrec` environment where closures capture
//! the environment and the environment contains closures, creating `Rc` cycles.
//! This is an intentional per-run leak: the entire env graph is dropped at the
//! end of `eval_file`, which is acceptable for an interactive/batch tool.

pub mod env;
pub mod eval;
pub mod eval_tlc;
pub mod thunk;
pub mod value;

mod analysis_eval;
mod errors;
mod force;
mod gate;
mod posit;
mod tlc_entry;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zutai_hir::BindingId;
use zutai_thir::{ImportKey, ThirDeclKind, ThirExprKind, ThirFile, TypeKind};

use eval::{Evaluator, ModuleRegistry, RuntimeWitness};
use value::ModuleId;

pub use analysis_eval::{
    eval_file, eval_path, eval_thir, eval_thir_file, eval_thir_path, eval_thir_with_base,
    eval_thir_with_imports, eval_with_base,
};
pub use errors::EvalError;
pub use force::force_deep;
pub use gate::{
    check_runnable, check_well_typed, describe_hir_diagnostic, describe_semantic_diagnostic,
    describe_thir_diagnostic,
};
pub use tlc_entry::{eval_tlc_file, eval_tlc_path, eval_tlc_with_base};
pub use value::Value;
