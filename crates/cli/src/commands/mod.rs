use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

mod compile;
mod eval;
mod model;
mod package;
mod reflect;
#[cfg(test)]
mod tests;
mod toolchain;

use self::reflect::*;
use self::toolchain::*;
pub(crate) use package::PackageCommand;

use crate::diagnostics::{
    ZtParseDiagnostic, extension_or_error, print_ast, print_backend_error, print_semantic_errors,
    print_zt_errors,
};

pub(crate) use compile::{EmitMode, run_compile, run_dataflow};
#[allow(unused_imports)]
pub(crate) use eval::{
    EvalOutcome, count_decls_in, eval_isolated, run_bare_path, run_check, run_file, run_format,
    run_isolated, run_json, run_parse, run_repl,
};
pub(crate) use model::run_model_check;
