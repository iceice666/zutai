use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
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

pub(crate) fn run_bare_path(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zt" => run_file(path),
        "zti" => run_parse_zti(path),
        other => Err(format!("Unsupported file extension: .{other}").into()),
    }
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
pub(crate) fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    // Resolve imports relative to the file's directory.
    let base = Path::new(path).parent();
    match eval_to_string(&contents, base) {
        Ok(rendered) => println!("{rendered}"),
        Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
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
        Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
            for m in msgs {
                eprintln!("type error: {m}");
            }
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("runtime error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Evaluate `contents` on a worker thread with a large stack.
///
/// The reference evaluator can still use deep native recursion on both the TLC
/// path and the THIR reflection boundary. A 256 MiB worker stack lets realistic
/// recursion complete. The forced `Value` holds `Rc`s and is not `Send`, so it is
/// rendered to its `Display` string inside the worker; only the `String` (or the
/// `Send` `EvalError`) crosses the join boundary.
fn eval_to_string(contents: &str, base: Option<&Path>) -> Result<String, zutai_eval::EvalError> {
    const EVAL_STACK_SIZE: usize = 256 * 1024 * 1024;
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(scope, || {
                zutai_eval::eval_with_base(contents, base).map(|value| value.to_string())
            })
            .expect("failed to spawn evaluation thread")
            .join()
            .expect("evaluation thread panicked")
    })
}

/// Parse a `.zt` file and print the AST (the old default behavior).
pub(crate) fn run_parse(path: &str) -> Result<(), Box<dyn Error>> {
    let ext = extension_or_error(path)?;
    let contents = fs::read_to_string(path)?;
    match ext.as_str() {
        "zt" => {
            let analysis = zutai_semantic::analyze(&contents);
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
                        zutai_semantic::SemanticStage::Hir | zutai_semantic::SemanticStage::Thir
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
    let hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    if let Some(reason) = analysis.reflection_builtin_program() {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }

    // TLC lowering.
    let module = zutai_tlc::lower_thir(thir);
    if let Some(reason) = zutai_tlc::residual_effect_reason(&module) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }

    // DC → ANF → SSA → LLVM IR pipeline.
    let graph = zutai_dataflow::lower_tlc(&module, hir_bindings);
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

fn output_path_for(input: &str, output_path: Option<&str>, emit: EmitMode) -> PathBuf {
    if let Some(out) = output_path {
        return PathBuf::from(out);
    }
    let mut out = PathBuf::from(input);
    match emit {
        EmitMode::Llvm => out.set_extension("ll"),
        EmitMode::Obj => out.set_extension("o"),
        EmitMode::Bin => out.set_extension(""),
    };
    out
}

fn tool_name(env_name: &str, fallback_env: &str, default: &'static str) -> String {
    std::env::var(env_name)
        .or_else(|_| std::env::var(fallback_env))
        .unwrap_or_else(|_| default.to_string())
}

fn run_tool(command: &mut Command, tool: &str, purpose: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status().map_err(|err| {
        format!(
            "compile error: required tool `{tool}` failed to start for {purpose}: {err}; install it or set ZUTAI_{}",
            tool.to_ascii_uppercase()
        )
    })?;
    if !status.success() {
        return Err(format!("compile error: `{tool}` failed while {purpose}").into());
    }
    Ok(())
}

fn assemble_object(ll: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let llc = tool_name("ZUTAI_LLC", "LLC", "llc");
    let mut command = Command::new(&llc);
    command.arg("-filetype=obj").arg("-o").arg(out).arg(ll);
    run_tool(&mut command, &llc, "assembling LLVM IR")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli crate lives under crates/cli")
        .to_path_buf()
}

fn build_runtime_archive() -> Result<PathBuf, Box<dyn Error>> {
    let root = workspace_root();
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("-p")
        .arg("zutai-rt")
        .current_dir(&root);
    run_tool(&mut command, "cargo", "building zutai-rt")?;
    Ok(root.join("target").join("debug").join("libzutai_rt.a"))
}

fn runtime_link_flags() -> &'static [&'static str] {
    match std::env::consts::OS {
        "linux" => &["-lpthread", "-ldl", "-lm"],
        "macos" => &[],
        _ => &[],
    }
}

fn link_binary(obj: &Path, runtime: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let clang = tool_name("ZUTAI_CLANG", "CLANG", "clang");
    let mut command = Command::new(&clang);
    command.arg(obj).arg(runtime);
    for flag in runtime_link_flags() {
        command.arg(flag);
    }
    command.arg("-o").arg(out);
    run_tool(&mut command, &clang, "linking native binary")
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
    let hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    if let Some(reason) = analysis.reflection_builtin_program() {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }

    let module = zutai_tlc::lower_thir(thir);
    if let Some(reason) = zutai_tlc::residual_effect_reason(&module) {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    let graph = zutai_dataflow::lower_tlc(&module, hir_bindings);
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
                match zutai_eval::eval_file(&eval_src) {
                    Ok(value) => println!("{value}"),
                    Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
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
                    Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
                        for m in msgs {
                            eprintln!("type error: {m}");
                        }
                    }
                    Err(e) => eprintln!("error: {e}"),
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

/// Estimate the number of top-level declarations in `src` by counting
/// top-level `::` or `:=` tokens (cheap heuristic; used only to classify
/// REPL input).
pub(crate) fn count_decls_in(src: &str) -> usize {
    let analysis = zutai_semantic::analyze(src);
    match analysis.hir.as_ref() {
        Some(h) => h.file.decls.len(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_decls_in_returns_zero_for_unparseable() {
        assert_eq!(count_decls_in(""), 0);
    }

    #[test]
    fn count_decls_in_returns_one_for_single_decl() {
        assert_eq!(count_decls_in("x := 1\nx\n"), 1);
    }

    #[test]
    fn count_decls_in_returns_two_for_two_decls() {
        assert_eq!(count_decls_in("x := 1\ny := 2\nx\n"), 2);
    }
}
