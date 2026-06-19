use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use clap::Parser;
use miette::{Diagnostic, LabeledSpan, NamedSource, SourceCode};
use thiserror::Error;
fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Run { path }) => run_file(&path)?,
        Some(Commands::Parse { path }) => run_parse(&path)?,
        Some(Commands::Check { path }) => run_check(&path)?,
        Some(Commands::Compile { path, output }) => {
            run_compile(&path, output.as_deref())?;
        }
        Some(Commands::Dataflow { path }) => run_dataflow(&path)?,
        Some(Commands::Repl) => run_repl()?,
        None => {
            let path = cli.path.expect("clap requires a subcommand or path");
            run_bare_path(&path)?;
        }
    }
    Ok(())
}

// ─── CLI definition ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "zutai-cli",
    about = "Zutai language compiler and interpreter",
    arg_required_else_help = true
)]
struct Cli {
    /// Legacy shorthand: run .zt files or parse .zti files without a subcommand.
    path: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Evaluate a .zt file and print the result
    Run {
        /// Path to the .zt file
        path: String,
    },
    /// Parse a file and print the AST
    Parse {
        /// Path to the .zt or .zti file
        path: String,
    },
    /// Type-check a .zt file and print diagnostics
    Check {
        /// Path to the .zt file
        path: String,
    },
    /// Compile a .zt file to LLVM IR
    Compile {
        /// Path to the .zt file
        path: String,
        /// Output file path (default: stdout)
        #[arg(short)]
        output: Option<String>,
    },
    /// Print the Dataflow Core graph for a .zt file
    Dataflow {
        /// Path to the .zt file
        path: String,
    },
    /// Run an interactive REPL
    Repl,
}

fn run_bare_path(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zt" => run_file(path),
        "zti" => run_parse_zti(path),
        other => Err(format!("Unsupported file extension: .{other}").into()),
    }
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
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
/// The interim interpreter is a tree-walker that uses native recursion, so deep
/// (but finite) recursion — including per-element recursion over a long list —
/// can overflow the default ~8 MiB main-thread stack and abort the process. A
/// 256 MiB worker stack lets realistic recursion complete. The forced `Value`
/// holds `Rc`s and is not `Send`, so it is rendered to its `Display` string
/// inside the worker; only the `String` (or the `Send` `EvalError`) crosses the
/// join boundary.
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

/// Run the type-checker on a `.zt` file and print diagnostics.
fn run_check(path: &str) -> Result<(), Box<dyn Error>> {
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
        print_semantic_errors(&semantic_errors);
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

/// Compile a `.zt` file to LLVM IR. Writes to stdout or `-o <output>`.
fn run_compile(path: &str, output_path: Option<&str>) -> Result<(), Box<dyn Error>> {
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
        print_semantic_errors(&semantic_errors);
        std::process::exit(1);
    }
    if !analysis.is_thir_complete() {
        eprintln!("compile error: THIR incomplete");
        std::process::exit(1);
    }
    if let Some(name) = analysis.compiler_unsupported_builtin() {
        eprintln!(
            "compile error: `{name}` is an interpreter-only builtin and cannot be compiled; \
             the v0 compiled core has no ambient effects (use `run` instead)"
        );
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    let hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    // TLC lowering.
    let module = zutai_tlc::lower_thir(thir);

    // DC → ANF → SSA → LLVM IR pipeline.
    let graph = zutai_dataflow::lower_tlc(&module, hir_bindings);
    let anf = zutai_anf::lower_dc(&graph);
    let ssa = zutai_ssa::lower_anf(&anf);
    let llvm_ir = zutai_codegen::emit_llvm(&ssa);

    match output_path {
        Some(out) => fs::write(out, &llvm_ir)?,
        None => println!("{llvm_ir}"),
    }
    Ok(())
}

/// Print the Dataflow Core graph for a `.zt` file.
fn run_dataflow(path: &str) -> Result<(), Box<dyn Error>> {
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
        print_semantic_errors(&semantic_errors);
        std::process::exit(1);
    }
    if !analysis.is_thir_complete() {
        eprintln!("error: cannot lower incomplete THIR");
        std::process::exit(1);
    }
    if let Some(name) = analysis.compiler_unsupported_builtin() {
        eprintln!(
            "error: `{name}` is an interpreter-only builtin and cannot be lowered to Dataflow Core"
        );
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    let hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    let module = zutai_tlc::lower_thir(thir);
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
        ConflictingWitness { constraint, target } => {
            format!("conflicting imported witnesses for {constraint} {target}")
        }
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

    // ── extension_or_error ────────────────────────────────────────────────────

    #[test]
    fn extension_or_error_returns_lowercase_ext() {
        assert_eq!(extension_or_error("hello.ZT").unwrap(), "zt");
        assert_eq!(extension_or_error("data.zti").unwrap(), "zti");
    }

    #[test]
    fn extension_or_error_no_extension_returns_err() {
        assert!(extension_or_error("noext").is_err());
        assert!(extension_or_error("no/ext").is_err());
    }

    // ── count_decls_in ────────────────────────────────────────────────────────

    #[test]
    fn count_decls_in_returns_zero_for_unparseable() {
        // Empty or invalid → HIR not produced → 0.
        assert_eq!(count_decls_in(""), 0);
    }

    #[test]
    fn count_decls_in_returns_one_for_single_decl() {
        // One declaration plus a final expression.
        assert_eq!(count_decls_in("x := 1\nx\n"), 1);
    }

    #[test]
    fn count_decls_in_returns_two_for_two_decls() {
        assert_eq!(count_decls_in("x := 1\ny := 2\nx\n"), 2);
    }

    // ── format_import_diagnostic — all arms ─────────────────────────────────

    fn make_diag(kind: zutai_semantic::ImportDiagnosticKind) -> zutai_semantic::ImportDiagnostic {
        zutai_semantic::ImportDiagnostic {
            kind,
            span: zutai_syntax::Span { start: 0, end: 1 },
        }
    }

    #[test]
    fn format_import_diag_no_base() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::NoBaseDirectory);
        assert!(format_import_diagnostic(&d).contains("base directory"));
    }

    #[test]
    fn format_import_diag_unsupported_form() {
        let d = make_diag(
            zutai_semantic::ImportDiagnosticKind::UnsupportedImportForm {
                path: "a/b.zt".to_string(),
            },
        );
        let s = format_import_diagnostic(&d);
        assert!(
            s.contains("unsupported import path") && s.contains("a/b.zt"),
            "{s}"
        );
    }

    #[test]
    fn format_import_diag_file_not_found() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::FileNotFound {
            path: "missing.zti".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(
            s.contains("file not found") && s.contains("missing.zti"),
            "{s}"
        );
    }

    #[test]
    fn format_import_diag_read_error() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::ReadError {
            path: "file.zti".to_string(),
            msg: "permission denied".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(
            s.contains("file.zti") && s.contains("permission denied"),
            "{s}"
        );
    }

    #[test]
    fn format_import_diag_parse_error() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::ParseError {
            path: "data.zti".to_string(),
            msg: "unexpected EOF".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(
            s.contains("data.zti") && s.contains("unexpected EOF"),
            "{s}"
        );
    }

    #[test]
    fn format_import_diag_import_cycle() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::ImportCycle {
            path: "a.zti".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(s.contains("import cycle") && s.contains("a.zti"), "{s}");
    }

    #[test]
    fn format_import_diag_module_has_errors() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::ModuleHasErrors {
            path: "lib.zti".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(s.contains("lib.zti") && s.contains("has errors"), "{s}");
    }

    #[test]
    fn format_import_diag_unsupported_export() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::UnsupportedExport {
            path: "mod.zti".to_string(),
            reason: "not a type",
        });
        let s = format_import_diagnostic(&d);
        assert!(s.contains("mod.zti") && s.contains("not a type"), "{s}");
    }

    #[test]
    fn format_import_diag_conflicting_witness() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::ConflictingWitness {
            constraint: "Eq".to_string(),
            target: "Int".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(s.contains("conflicting imported witnesses"), "{s}");
        assert!(s.contains("Eq") && s.contains("Int"), "{s}");
    }

    // ── ZtParseDiagnostic span clamping ──────────────────────────────────────

    #[test]
    fn zt_parse_diagnostic_clamps_span_end_to_content_length() {
        // Produce a real parse diagnostic with a span that might exceed the
        // content length when rendered.
        let contents = "[1; 2]";
        let parsed = zutai_syntax::parse(contents);
        let err = parsed
            .diagnostics()
            .first()
            .expect("fixture should fail")
            .clone();
        // This should not panic even if span.end > contents.len().
        let d = ZtParseDiagnostic::new("f.zt", contents, err);
        // We just need to ensure the span was clamped (no panic, valid len ≥ 1).
        assert!(d.span.1 >= 1);
    }
}
