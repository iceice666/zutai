use std::error::Error;
use std::fs;
use std::path::Path;

mod reflect;
#[cfg(test)]
mod tests;
mod toolchain;

use self::reflect::*;
use self::toolchain::*;
use std::process::Command;

use crate::diagnostics::{
    ZtParseDiagnostic, extension_or_error, format_import_diagnostic, print_ast,
    print_semantic_errors, print_zt_errors,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmitMode {
    Llvm,
    Obj,
    Bin,
}

const UNSUPPORTED_TYPE_ENTRY_REASON: &str =
    "compiled entry point returns Type, which cannot be shown by the v0 runtime ABI";

fn unsupported_thir_entry_type_reason(thir: &zutai_thir::ThirFile) -> Option<&'static str> {
    let final_ty = thir.expr_arena[thir.final_expr].ty;
    let kind = &thir.type_arena.get(final_ty.0 as usize)?.kind;
    matches!(kind, zutai_thir::TypeKind::Type).then_some(UNSUPPORTED_TYPE_ENTRY_REASON)
}

pub(crate) fn run_bare_path(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zt" => run_file(path),
        "zti" => run_parse_zti(path),
        other => Err(format!("Unsupported file extension: .{other}").into()),
    }
}

// ─── eval isolation ───────────────────────────────────────────────────────────

/// Outcome of an isolated evaluation: successful rendered value, a structured
/// error, or a panic absorbed from the evaluator worker.
#[derive(Debug, PartialEq)]
pub(crate) enum EvalOutcome {
    Ok(String),
    Err(zutai_eval::EvalError),
    Panicked(String),
}

/// Extract a human-readable message from a `catch_unwind` panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "evaluator panicked (run with RUST_BACKTRACE=1 for details)".to_string()
    }
}

/// Run `f` on a large-stack (256 MiB) worker thread with full panic isolation.
///
/// Returns [`EvalOutcome::Panicked`] when the worker panics instead of
/// re-panicking the caller; the main thread is never unwound.
pub(crate) fn run_isolated<F>(f: F) -> EvalOutcome
where
    F: FnOnce() -> Result<String, zutai_eval::EvalError> + Send,
{
    const EVAL_STACK_SIZE: usize = 256 * 1024 * 1024;
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(scope, move || {
                // Absorb panics inside the worker so the thread itself never
                // unwinds past this closure; join() always returns Ok(_).
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            })
            .expect("failed to spawn evaluation thread");
        match handle.join() {
            Ok(catch_result) => match catch_result {
                Ok(Ok(s)) => EvalOutcome::Ok(s),
                Ok(Err(e)) => EvalOutcome::Err(e),
                Err(payload) => EvalOutcome::Panicked(panic_message(&*payload)),
            },
            Err(payload) => EvalOutcome::Panicked(panic_message(&*payload)),
        }
    })
}

/// Evaluate `contents` on an isolated large-stack worker thread.
///
/// The forced `Value` holds `Rc`s and is not `Send`, so it is rendered to its
/// `Display` string inside the worker; only the `String` (or the `Send`
/// `EvalError`, or the caught panic message) crosses the join boundary.
pub(crate) fn eval_isolated(contents: &str, base: Option<&Path>) -> EvalOutcome {
    let contents = contents.to_owned();
    let base = base.map(Path::to_path_buf);
    run_isolated(move || {
        zutai_eval::eval_with_base(&contents, base.as_deref()).map(|v| v.to_string())
    })
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
pub(crate) fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    // Resolve imports relative to the file's directory.
    let base = Path::new(path).parent();
    match eval_isolated(&contents, base) {
        EvalOutcome::Ok(rendered) => println!("{rendered}"),
        EvalOutcome::Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
            // These are parse/HIR/import errors — render with miette if possible,
            // otherwise fall back to the semantic analyzer for pretty output.
            let analysis = zutai_semantic::analyze_with_base(
                &contents,
                base,
                zutai_semantic::AnalysisOptions::default(),
            );
            let parse_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|d| match &d.kind {
                    zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            if !parse_errors.is_empty() {
                print_zt_errors(path, &contents, &parse_errors);
                std::process::exit(1);
            }
            let import_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|d| match &d.kind {
                    zutai_semantic::SemanticDiagnosticKind::Import(i) => Some(i.clone()),
                    _ => None,
                })
                .collect();
            if !import_errors.is_empty() {
                for err in &import_errors {
                    eprintln!("import error: {}", format_import_diagnostic(err));
                }
                std::process::exit(1);
            }
            for m in msgs {
                eprintln!("error: {m}");
            }
            std::process::exit(1);
        }
        EvalOutcome::Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
            for m in msgs {
                eprintln!("type error: {m}");
            }
            std::process::exit(1);
        }
        EvalOutcome::Err(e) => {
            eprintln!("runtime error: {e}");
            std::process::exit(1);
        }
        EvalOutcome::Panicked(msg) => {
            eprintln!("internal evaluator error: {msg}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Parse a `.zt` file and print the AST (the old default behavior).
pub(crate) fn run_parse(path: &str) -> Result<(), Box<dyn Error>> {
    let ext = extension_or_error(path)?;
    let contents = fs::read_to_string(path)?;
    match ext.as_str() {
        "zt" => {
            let analysis = zutai_semantic::analyze_with_base(
                &contents,
                Path::new(path).parent(),
                zutai_semantic::AnalysisOptions::default(),
            );
            let parse_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|d| match &d.kind {
                    zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            if !parse_errors.is_empty() {
                print_zt_errors(path, &contents, &parse_errors);
                std::process::exit(1);
            }
            let semantic_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter(|d| {
                    matches!(
                        d.stage,
                        zutai_semantic::SemanticStage::Import
                            | zutai_semantic::SemanticStage::Hir
                            | zutai_semantic::SemanticStage::Thir
                    )
                })
                .collect();
            if !semantic_errors.is_empty() {
                print_semantic_errors(path, &contents, &semantic_errors);
                std::process::exit(1);
            }
            if let Some(ast) = analysis.ast.as_ref() {
                print_ast("zt", ast);
            } else {
                eprintln!("parse produced no AST");
                std::process::exit(1);
            }
        }
        "zti" => run_parse_zti(path)?,
        other => return Err(format!("Unsupported extension: {other}").into()),
    }
    Ok(())
}

/// Parse and print a `.zti` immediate-format file.
pub(crate) fn run_parse_zti(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let ast = zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
    print_ast("zti", &ast);
    Ok(())
}

/// Run the type-checker on a `.zt` file and print diagnostics.
pub(crate) fn run_check(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let analysis = zutai_semantic::analyze_with_base(
        &contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if !parse_errors.is_empty() {
        print_zt_errors(path, &contents, &parse_errors);
        std::process::exit(1);
    }
    let semantic_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.stage,
                zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Thir
                    | zutai_semantic::SemanticStage::Import
            )
        })
        .collect();
    if !semantic_errors.is_empty() {
        print_semantic_errors(path, &contents, &semantic_errors);
        std::process::exit(1);
    }
    if analysis.is_thir_complete() {
        println!("check passed: {path}");
    } else {
        eprintln!("check incomplete: THIR has errors");
        std::process::exit(1);
    }
    Ok(())
}

/// Compile a `.zt` file. LLVM emits text; object/binary modes invoke the host LLVM toolchain.
pub(crate) fn run_compile(
    path: &str,
    output_path: Option<&str>,
    emit: EmitMode,
) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let analysis = zutai_semantic::analyze_with_base(
        &contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    // Gate on parse and semantic errors.
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if !parse_errors.is_empty() {
        print_zt_errors(path, &contents, &parse_errors);
        std::process::exit(1);
    }
    let semantic_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.stage,
                zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Thir
                    | zutai_semantic::SemanticStage::Import
            )
        })
        .collect();
    if !semantic_errors.is_empty() {
        print_semantic_errors(path, &contents, &semantic_errors);
        std::process::exit(1);
    }
    if !analysis.is_thir_complete() {
        eprintln!("compile error: THIR incomplete");
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    if let Some(reason) = unsupported_thir_entry_type_reason(thir) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let original_hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;
    let uses_reflection = analysis.reflection_builtin_program().is_some();

    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }

    // TLC lowering. Effectful programs enter DC only when TLC lowering has
    // eliminated effect markers or mapped ambient `io.print` to the runtime
    // HostPrint path.
    let mut module = zutai_tlc::lower_thir(thir);
    let mut folded_bindings = None;
    let boundary_host_grants = zutai_tlc::HostEffectSet::ALL;
    let has_host_io_print = zutai_tlc::contains_host_io_print(&module);
    let has_unfolded_effects = zutai_tlc::residual_effect_reason(&module).is_some();
    let residual_effect_reason =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants);
    if uses_reflection && (has_host_io_print || has_unfolded_effects) {
        eprintln!(
            "compile error: reflection builtins cannot be AOT-folded with effectful code yet"
        );
        std::process::exit(1);
    }
    if uses_reflection {
        match fold_aot_reflection(&contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
            }
            Err(err) => {
                eprintln!("compile error: {err}");
                std::process::exit(1);
            }
        }
    } else if let Some(reason) = residual_effect_reason {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    if let Some(reason) =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants)
    {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let hir_bindings = folded_bindings
        .as_deref()
        .unwrap_or(original_hir_bindings.as_slice());

    // DC → ANF → SSA → LLVM IR pipeline.
    let graph =
        zutai_dataflow::lower_tlc_with_host_grants(&module, hir_bindings, boundary_host_grants);
    let anf = zutai_anf::lower_dc(&graph);
    let ssa = zutai_ssa::lower_anf(&anf);
    if let Some(reason) = zutai_codegen::unsupported_entry_type_reason(&ssa) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let llvm_ir = zutai_codegen::emit_llvm(&ssa);

    match emit {
        EmitMode::Llvm => match output_path {
            Some(out) => fs::write(out, &llvm_ir)?,
            None => println!("{llvm_ir}"),
        },
        EmitMode::Obj => {
            let out = output_path_for(path, output_path, EmitMode::Obj);
            let ll = out.with_extension("ll");
            fs::write(&ll, &llvm_ir)?;
            assemble_object(&ll, &out)?;
        }
        EmitMode::Bin => {
            let out = output_path_for(path, output_path, EmitMode::Bin);
            let ll = out.with_extension("ll");
            let obj = out.with_extension("o");
            fs::write(&ll, &llvm_ir)?;
            assemble_object(&ll, &obj)?;
            let rt = build_runtime_archive()?;
            link_binary(&obj, &rt, &out)?;
        }
    }
    Ok(())
}

/// Print the Dataflow Core graph for a `.zt` file.
pub(crate) fn run_dataflow(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let analysis = zutai_semantic::analyze_with_base(
        &contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if !parse_errors.is_empty() {
        print_zt_errors(path, &contents, &parse_errors);
        std::process::exit(1);
    }
    let semantic_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.stage,
                zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Thir
                    | zutai_semantic::SemanticStage::Import
            )
        })
        .collect();
    if !semantic_errors.is_empty() {
        print_semantic_errors(path, &contents, &semantic_errors);
        std::process::exit(1);
    }
    if !analysis.is_thir_complete() {
        eprintln!("error: cannot lower incomplete THIR");
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    if let Some(reason) = unsupported_thir_entry_type_reason(thir) {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    let original_hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    let uses_reflection = analysis.reflection_builtin_program().is_some();
    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }

    let mut module = zutai_tlc::lower_thir(thir);
    let mut folded_bindings = None;
    let boundary_host_grants = zutai_tlc::HostEffectSet::ALL;
    let has_host_io_print = zutai_tlc::contains_host_io_print(&module);
    let has_unfolded_effects = zutai_tlc::residual_effect_reason(&module).is_some();
    let residual_effect_reason =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants);
    if uses_reflection && (has_host_io_print || has_unfolded_effects) {
        eprintln!("error: reflection builtins cannot be AOT-folded with effectful code yet");
        std::process::exit(1);
    }
    if uses_reflection {
        match fold_aot_reflection(&contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
            }
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
    } else if let Some(reason) = residual_effect_reason {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    if let Some(reason) =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants)
    {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    let hir_bindings = folded_bindings
        .as_deref()
        .unwrap_or(original_hir_bindings.as_slice());
    let graph =
        zutai_dataflow::lower_tlc_with_host_grants(&module, hir_bindings, boundary_host_grants);
    println!("{graph:#?}");
    Ok(())
}

/// Run an interactive REPL session.
///
/// ## Session model
///
/// We keep a running `decls_buf: String` that accumulates all declarations the
/// user has entered.  Each input line is classified as:
///
/// - A **declaration** if `analyze(decls_buf + input + "\n0")` (with a throwaway
///   final expression appended so THIR has something to resolve) adds a new
///   declaration, parses cleanly, and passes the THIR gate.  If so, we append
///   the line to `decls_buf` and acknowledge.
///
/// - An **expression** otherwise: we try `eval_file(decls_buf + input)` through
///   the default TLC-first evaluator and print the result (or the diagnostic).
///
/// `BindingId`s are NOT stable across separate `analyze()` calls, so we NEVER
/// cache thunks across turns — the env is rebuilt from scratch for each
/// evaluation.
pub(crate) fn run_repl() -> Result<(), Box<dyn Error>> {
    use rustyline::DefaultEditor;
    use rustyline::error::ReadlineError;

    let mut rl = DefaultEditor::new()?;
    let mut decls_buf = String::new();

    println!("Zutai REPL (type `:quit` to exit, `:reset` to clear bindings)");

    loop {
        let prompt = if decls_buf.is_empty() { "zt> " } else { "... " };
        match rl.readline(prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(trimmed);

                match trimmed {
                    ":quit" | ":q" => break,
                    ":reset" => {
                        decls_buf.clear();
                        println!("(bindings cleared)");
                        continue;
                    }
                    _ => {}
                }

                // Try treating the input as a declaration by appending it and
                // a throwaway final expression `0`.
                let probe_src = format!("{decls_buf}{trimmed}\n0\n");
                let probe = zutai_semantic::analyze(&probe_src);
                let decl_count_before = count_decls_in(&decls_buf);
                let decl_count_after = count_decls_in(&probe_src);

                let is_new_decl = !probe.has_parse_errors()
                    && !probe.has_hir_errors()
                    && probe.is_thir_complete()
                    && decl_count_after > decl_count_before;

                if is_new_decl {
                    decls_buf.push_str(trimmed);
                    decls_buf.push('\n');
                    println!("ok");
                    continue;
                }

                // Otherwise treat as an expression to evaluate.
                let eval_src = format!("{decls_buf}{trimmed}\n");
                // Suppress the worker's default panic-hook line so only the
                // clean error message reaches stderr; the hook is always
                // restored before the next prompt so no global state leaks.
                let prev_hook = std::panic::take_hook();
                std::panic::set_hook(Box::new(|_| {}));
                let outcome = eval_isolated(&eval_src, None);
                std::panic::set_hook(prev_hook);
                match outcome {
                    EvalOutcome::Ok(rendered) => println!("{rendered}"),
                    EvalOutcome::Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
                        // Try pretty parse error output.
                        let analysis = zutai_semantic::analyze(&eval_src);
                        let parse_errors: Vec<_> = analysis
                            .diagnostics
                            .iter()
                            .filter_map(|d| match &d.kind {
                                zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
                                _ => None,
                            })
                            .collect();
                        if !parse_errors.is_empty() {
                            for err in &parse_errors {
                                eprintln!(
                                    "{:?}",
                                    miette::Report::new(ZtParseDiagnostic::new(
                                        "<repl>",
                                        &eval_src,
                                        err.clone()
                                    ))
                                );
                            }
                        } else {
                            for m in msgs {
                                eprintln!("error: {m}");
                            }
                        }
                    }
                    EvalOutcome::Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
                        for m in msgs {
                            eprintln!("type error: {m}");
                        }
                    }
                    EvalOutcome::Err(e) => eprintln!("error: {e}"),
                    EvalOutcome::Panicked(msg) => {
                        eprintln!(
                            "internal evaluator error: {msg}\n(input not applied; session preserved)"
                        );
                    }
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

/// Estimate the number of top-level declarations in `src` from parsed HIR
/// declarations (used only to classify REPL input).
pub(crate) fn count_decls_in(src: &str) -> usize {
    let analysis = zutai_semantic::analyze(src);
    match analysis.hir.as_ref() {
        Some(h) => h.file.decls.len(),
        None => 0,
    }
}
