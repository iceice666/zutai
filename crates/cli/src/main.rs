use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use miette::{Diagnostic, LabeledSpan, NamedSource, SourceCode};
use thiserror::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    // Route to the appropriate subcommand.
    match args.as_slice() {
        // `zutai-cli repl` — interactive REPL
        [cmd] if cmd == "repl" => {
            run_repl()?;
        }
        // `zutai-cli parse <path>` — print AST (backwards-compat)
        [cmd, path] if cmd == "parse" => {
            run_parse(&path.clone())?;
        }
        // `zutai-cli run <path>` — evaluate and print result
        [cmd, path] if cmd == "run" => {
            run_file(&path.clone())?;
        }
        // `zutai-cli <path>` — bare path: `.zt` → run, `.zti` → parse/print AST
        [path] if !path.starts_with('-') => {
            let ext = extension_or_error(path)?.to_ascii_lowercase();
            match ext.as_str() {
                "zt" => run_file(&path.clone())?,
                "zti" => run_parse_zti(&path.clone())?,
                other => return Err(format!("Unsupported extension: {other}").into()),
            }
        }
        _ => {
            return Err("usage: zutai-cli [run <path>|parse <path>|repl|<path>]".into());
        }
    }

    let _ = args; // suppress unused binding warning
    Ok(())
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    // Resolve imports relative to the file's directory.
    let base = Path::new(path).parent();
    match zutai_eval::eval_with_base(&contents, base) {
        Ok(value) => println!("{value}"),
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

/// Parse a `.zt` file and print the AST (the old default behavior).
fn run_parse(path: &str) -> Result<(), Box<dyn Error>> {
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
                print_semantic_errors(&semantic_errors);
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
fn run_parse_zti(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let ast = zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
    print_ast("zti", &ast);
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
/// - An **expression** otherwise: we try `eval_file(decls_buf + input)` and
///   print the result (or the diagnostic).
///
/// `BindingId`s are NOT stable across separate `analyze()` calls, so we NEVER
/// cache thunks across turns — the env is rebuilt from scratch for each
/// evaluation.
fn run_repl() -> Result<(), Box<dyn Error>> {
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
fn count_decls_in(src: &str) -> usize {
    let analysis = zutai_semantic::analyze(src);
    match analysis.hir.as_ref() {
        Some(h) => h.file.decls.len(),
        None => 0,
    }
}

// ─── shared helpers ───────────────────────────────────────────────────────────

fn print_semantic_errors(errs: &[&zutai_semantic::SemanticDiagnostic]) {
    for err in errs {
        match &err.kind {
            zutai_semantic::SemanticDiagnosticKind::Import(import) => {
                eprintln!("import error: {}", format_import_diagnostic(import));
            }
            _ => eprintln!("semantic error: {err:?}"),
        }
    }
}

fn format_import_diagnostic(diag: &zutai_semantic::ImportDiagnostic) -> String {
    use zutai_semantic::ImportDiagnosticKind::*;
    match &diag.kind {
        NoBaseDirectory => "cannot resolve an import without a base directory".to_string(),
        UnsupportedImportForm { path } => format!("unsupported import path: {path}"),
        FileNotFound { path } => format!("file not found: {path}"),
        ReadError { path, msg } => format!("cannot read {path}: {msg}"),
        ParseError { path, msg } => format!("failed to parse {path}: {msg}"),
        ImportCycle { path } => format!("import cycle through {path}"),
        ModuleHasErrors { path } => format!("imported module {path} has errors"),
        UnsupportedExport { path, reason } => format!("cannot import {path}: {reason}"),
    }
}

fn print_zt_errors(path: &str, contents: &str, errs: &[zutai_syntax::Diagnostic]) {
    for err in errs {
        eprintln!(
            "{:?}",
            miette::Report::new(ZtParseDiagnostic::new(path, contents, err.clone()))
        );
    }
}

fn extension_or_error(path: &str) -> Result<String, Box<dyn Error>> {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| format!("File has no extension: {path}"))?
        .to_ascii_lowercase();
    Ok(extension)
}

fn print_ast(label: &str, ast: &impl std::fmt::Display) {
    println!("Parsed .{label} AST:");
    println!("{ast}");
}

// ─── miette parse-diagnostic renderer (unchanged) ────────────────────────────

#[derive(Debug, Error)]
#[error("{message}")]
struct ZtParseDiagnostic {
    source_code: NamedSource<String>,
    message: String,
    code: &'static str,
    help: Option<String>,
    label: String,
    span: (usize, usize),
}

impl ZtParseDiagnostic {
    fn new(path: &str, contents: &str, err: zutai_syntax::Diagnostic) -> Self {
        let span = err.primary_span();
        let start = span.start as usize;
        let end = span.end as usize;
        let clamped_start = start.min(contents.len());
        let max_len = contents.len().saturating_sub(clamped_start);
        let len = end.saturating_sub(start).max(1).min(max_len.max(1));
        let label = err
            .labels
            .iter()
            .find(|label| label.style == zutai_syntax::LabelStyle::Primary)
            .map(|label| label.message.clone())
            .unwrap_or_else(|| err.kind.label().to_string());
        Self {
            source_code: NamedSource::new(path, contents.to_string()),
            message: err.message,
            code: err.code,
            help: err.help,
            label,
            span: (clamped_start, len),
        }
    }
}

impl Diagnostic for ZtParseDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(self.code))
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.help
            .as_ref()
            .map(|help| Box::new(help) as Box<dyn fmt::Display>)
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(&self.source_code)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        Some(Box::new(std::iter::once(LabeledSpan::at(
            self.span,
            self.label.clone(),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_zt_parse_error_with_source_context() {
        let path = "bad.zt";
        let contents = "[1; 2]";
        let parsed = zutai_syntax::parse(contents);
        let err = parsed
            .diagnostics()
            .first()
            .expect("fixture should fail")
            .clone();

        let rendered = format!(
            "{:?}",
            miette::Report::new(ZtParseDiagnostic::new(path, contents, err))
        );

        assert!(rendered.contains(path), "{rendered}");
        assert!(rendered.contains(contents), "{rendered}");
        assert!(
            rendered.contains("list items must end with `;`"),
            "{rendered}"
        );
        assert!(
            rendered.contains("missing `;` before this delimiter"),
            "{rendered}"
        );
    }
}
