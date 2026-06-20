//! Golden-semantics test suite for the Zutai default reference evaluator.
//!
//! The default path is TLC-first for executable value programs. THIR-specific
//! behavior is asserted through explicit `eval_thir_*` oracle APIs.

use std::path::Path;

use crate::{
    EvalError, Value, eval_file, eval_thir_file, eval_tlc_file, eval_tlc_with_base, eval_with_base,
    thunk, value,
};

// ─── helpers ──────────────────────────────────────────────────────────────────

fn run(src: &str) -> Value {
    eval_file(src).unwrap_or_else(|e| panic!("eval_file failed for:\n{src}\nerror: {e}"))
}

fn run_err(src: &str) -> EvalError {
    eval_file(src).expect_err(&format!("expected error for:\n{src}"))
}

fn run_thir_err(src: &str) -> EvalError {
    eval_thir_file(src).expect_err(&format!("expected THIR oracle error for:\n{src}"))
}

fn list_item(value: &Value, index: usize) -> Value {
    let Value::List(items) = value else {
        panic!("expected list, got {value:?}");
    };
    items
        .get(index)
        .and_then(thunk::Thunk::peek)
        .unwrap_or_else(|| panic!("missing forced list item {index}"))
}

fn record_field_value(value: &Value, field_name: &str) -> Value {
    let Value::Record(fields) = value else {
        panic!("expected record, got {value:?}");
    };
    fields
        .iter()
        .find(|(name, _)| name.as_ref() == field_name)
        .and_then(|(_, value)| value.peek())
        .unwrap_or_else(|| panic!("missing forced record field {field_name}"))
}

fn run_in_imports(src: &str) -> Value {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports");
    eval_with_base(src, Some(&base))
        .unwrap_or_else(|e| panic!("eval_with_base failed for:\n{src}\nerror: {e}"))
}
fn imports_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports")
}

/// Evaluate `src` with the shared fixtures directory as the import base.
fn run_import(src: &str) -> Value {
    crate::eval_with_base(src, Some(&imports_dir()))
        .unwrap_or_else(|e| panic!("eval failed for:\n{src}\nerror: {e}"))
}

fn run_import_err(src: &str) -> EvalError {
    crate::eval_with_base(src, Some(&imports_dir()))
        .expect_err(&format!("expected error for:\n{src}"))
}
fn imports_path(name: &str) -> std::path::PathBuf {
    imports_dir().join(name)
}

mod basic;
mod diagnostics_effects;
mod display_runtime;
mod functions_patterns;
mod imports;
mod witnesses;
