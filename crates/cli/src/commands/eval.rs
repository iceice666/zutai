use super::*;

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
///
/// When `enforce_abi` is set, a result that is not first-order serializable
/// data (a function, runtime `Type`, witness, or opaque handle — nested
/// included) is rejected with the same reason native compilation reports, so
/// `run`/`json` refuse exactly the entries `compile` refuses. The REPL passes
/// `false` so it can still display such values for interactive inspection.
pub(crate) fn eval_isolated(contents: &str, base: Option<&Path>, enforce_abi: bool) -> EvalOutcome {
    let contents = contents.to_owned();
    let base = base.map(Path::to_path_buf);
    run_isolated(move || {
        reject_unsupported_render_entry(&contents, base.as_deref())?;
        let value = zutai_eval::eval_with_base(&contents, base.as_deref())?;
        if enforce_abi && let Some(reason) = value.runtime_abi_reason() {
            return Err(zutai_eval::EvalError::NotRunnable(vec![reason.to_string()]));
        }
        Ok(value.to_string())
    })
}

fn reject_unsupported_render_entry(
    contents: &str,
    base: Option<&Path>,
) -> Result<(), zutai_eval::EvalError> {
    let analysis = zutai_semantic::analyze_with_base(
        contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    let Some(thir) = analysis.thir.as_ref().and_then(|thir| thir.file.as_ref()) else {
        return Ok(());
    };
    if let Some(reason) = super::compile::unsupported_thir_entry_type_reason(thir) {
        Err(zutai_eval::EvalError::NotRunnable(vec![reason.to_string()]))
    } else {
        Ok(())
    }
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
pub(crate) fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    // Resolve imports relative to the file's directory.
    let base = Path::new(path).parent();
    match eval_isolated(&contents, base, true) {
        EvalOutcome::Ok(rendered) => println!("{rendered}"),
        outcome => exit_for_eval_failure(path, &contents, base, outcome),
    }
    Ok(())
}

/// Render a non-`Ok` `.zt` evaluation outcome to stderr and exit nonzero.
///
/// Shared by `run_file` and `run_json` so `.zt` parse/import/type/runtime/panic
/// diagnostics and exit codes stay identical across both commands.
fn exit_for_eval_failure(
    path: &str,
    contents: &str,
    base: Option<&Path>,
    outcome: EvalOutcome,
) -> ! {
    match outcome {
        EvalOutcome::Ok(_) => unreachable!("exit_for_eval_failure called with EvalOutcome::Ok"),
        EvalOutcome::Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
            let analysis = super::compile::analyze_with_cli_diagnostics(
                path,
                contents,
                base,
                &zutai_semantic::AnalysisCache::default(),
            );
            if analysis.diagnostics.is_empty() {
                for msg in msgs {
                    eprintln!("error: {msg}");
                }
            }
            std::process::exit(1);
        }
        EvalOutcome::Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
            let analysis = super::compile::analyze_with_cli_diagnostics(
                path,
                contents,
                base,
                &zutai_semantic::AnalysisCache::default(),
            );
            if analysis.diagnostics.is_empty() {
                for msg in msgs {
                    eprintln!("type error: {msg}");
                }
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

pub(crate) fn run_format(path: &str, check: bool) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let formatted = match extension_or_error(path)?.as_str() {
        "zt" => match zutai_syntax::format_source(&contents) {
            Ok(formatted) => formatted,
            Err(diagnostics) => {
                print_zt_errors(path, &contents, &diagnostics);
                std::process::exit(1);
            }
        },
        "zti" => zutai_im::format_source(&contents)
            .map_err(|error| format!("Failed to format .zti: {error}"))?,
        other => return Err(format!("Unsupported extension: {other}").into()),
    };

    if formatted == contents {
        return Ok(());
    }
    if check {
        return Err(format!("formatting required: {path}").into());
    }
    fs::write(path, formatted)?;
    Ok(())
}

/// Parse a `.zti` or evaluate a `.zt` file and print the final result as
/// natural JSON.
///
/// `.zti` documents are parsed and their inert data serialized directly; `.zt`
/// programs are fully evaluated (under the same isolated large-stack worker as
/// `run`) and the forced final value serialized. Encoding is natural JSON:
/// atoms become `#`-prefixed strings and tagged values become
/// `{ "tag", "payload" }` objects.
pub(crate) fn run_json(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zti" => {
            let contents = fs::read_to_string(path)?;
            let block =
                zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
            let json = zti_block_to_json(&block);
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        "zt" => {
            let contents = fs::read_to_string(path)?;
            let base = Path::new(path).parent();
            let owned_contents = contents.clone();
            let owned_base = base.map(Path::to_path_buf);
            let outcome = run_isolated(move || {
                reject_unsupported_render_entry(&owned_contents, owned_base.as_deref())?;
                let value = zutai_eval::eval_with_base(&owned_contents, owned_base.as_deref())?;
                if let Some(reason) = value.runtime_abi_reason() {
                    return Err(zutai_eval::EvalError::NotRunnable(vec![reason.to_string()]));
                }
                let json = value.to_json()?;
                Ok(serde_json::to_string_pretty(&json)
                    .expect("serializing serde_json::Value cannot fail"))
            });
            match outcome {
                EvalOutcome::Ok(rendered) => println!("{rendered}"),
                outcome => exit_for_eval_failure(path, &contents, base, outcome),
            }
        }
        other => return Err(format!("Unsupported extension: {other}").into()),
    }
    Ok(())
}

/// Convert a parsed `.zti` block into a natural JSON object.
fn zti_block_to_json(block: &zutai_im::Block) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(block.len());
    for pair in block.iter() {
        map.insert(pair.field_name.clone(), zti_value_to_json(&pair.value));
    }
    serde_json::Value::Object(map)
}

/// Convert a parsed `.zti` value into natural JSON.
fn zti_value_to_json(value: &zutai_im::Value) -> serde_json::Value {
    use serde_json::Value as J;
    use zutai_im::Value as Im;
    match value {
        Im::True => J::Bool(true),
        Im::False => J::Bool(false),
        Im::Integer(n) => serde_json::json!(n),
        Im::Float(f) => serde_json::json!(f),
        Im::String(s) => J::String(s.clone()),
        Im::Atom(s) => J::String(format!("#{s}")),
        Im::Array(items) => J::Array(items.iter().map(zti_value_to_json).collect()),
        Im::Block(block) => zti_block_to_json(block),
    }
}

/// Run the type-checker on a `.zt` file and print diagnostics.
pub(crate) fn run_check(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let cache = zutai_semantic::AnalysisCache::default();
    let analysis = super::compile::analyze_with_cli_diagnostics(path, &contents, base, &cache);

    if !analysis.is_thir_complete() {
        eprintln!("check incomplete: THIR has errors");
        std::process::exit(1);
    }
    if let Some(reason) = super::compile::unsupported_cli_entry_type_reason(&analysis) {
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_ENTRY_TYPE_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: super::compile::backend_entry_span(&analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    // Reflection-fold refusals are backend-only support boundaries, not type
    // errors: the program is well-typed, so `check` surfaces them as warnings
    // (matching `backend_diagnostics()` and LSP severity) while `compile` and
    // `dataflow` keep rejecting.
    if analysis.aot_reflection_program().is_some()
        && let Err(err) = super::compile::fold_aot_reflection_for_cli(&contents, base)
    {
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_REFLECTION_FOLD_CODE,
                severity: zutai_syntax::Severity::Warning,
                message: err.to_string(),
                span: super::compile::backend_entry_span(&analysis),
                related: Vec::new(),
            },
        );
    }
    for diagnostic in analysis.native_import_diagnostics() {
        print_backend_error(path, &contents, &diagnostic);
    }
    println!("check passed: {path}");
    Ok(())
}

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
                let outcome = eval_isolated(&eval_src, None, false);
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
    // Count user declarations from the parsed AST — the HIR additionally carries
    // the builtin prelude (e.g. the `Stream` codata type), which is not user code.
    let analysis = zutai_semantic::analyze(src);
    match analysis.ast.as_ref() {
        Some(ast) => ast.decls.len(),
        None => 0,
    }
}
