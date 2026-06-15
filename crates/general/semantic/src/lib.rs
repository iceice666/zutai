//! Semantic analysis pipeline for Zutai general mode (`.zt`).
//!
//! This crate is the facade that wires parser AST lowering into HIR and then
//! THIR. It deliberately keeps stage results separate so callers can inspect
//! partial output when a later semantic phase fails or is not implemented yet.
//!
//! It also owns module loading: because THIR lowering is pure, the filesystem
//! work for `import` expressions lives here (see [`import`]).  Path-relative
//! imports require a base directory, so the path-aware entry points
//! ([`analyze_path`], [`analyze_with_base`]) carry one; the string-only entry
//! points resolve imports with no base, which surfaces a clean diagnostic.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

mod import;

pub use import::{ImportDiagnostic, ImportDiagnosticKind};

#[derive(Debug)]
pub struct Analysis {
    pub ast: Option<zutai_syntax::File>,
    pub hir: Option<zutai_hir::LoweredHir>,
    pub thir: Option<zutai_thir::LoweredThir>,
    pub diagnostics: Vec<SemanticDiagnostic>,
    pub pass_reports: Vec<SemanticPassReport>,
    /// Parsed `.zti` import values, keyed by import source, for the evaluator.
    pub import_values: HashMap<zutai_thir::ImportKey, zutai_im::Value>,
    /// Analyzed `.zt` sub-modules, keyed by import source, for the evaluator to
    /// evaluate recursively.
    pub import_modules: HashMap<zutai_thir::ImportKey, Rc<Analysis>>,
    /// TLC module produced by lowering THIR; `None` when THIR is incomplete.
    pub tlc: Option<zutai_tlc::TlcModule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticPassReport {
    pub stage: SemanticStage,
    pub name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiagnostic {
    pub stage: SemanticStage,
    pub kind: SemanticDiagnosticKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticStage {
    Parse,
    Hir,
    Import,
    Thir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticDiagnosticKind {
    Parse(zutai_syntax::Diagnostic),
    Hir(zutai_hir::HirDiagnostic),
    Import(ImportDiagnostic),
    Thir(zutai_thir::ThirDiagnostic),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisOptions {
    pub run_hir_passes: bool,
    pub run_thir_passes: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            run_hir_passes: true,
            run_thir_passes: true,
        }
    }
}

impl Analysis {
    pub fn has_parse_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.stage == SemanticStage::Parse)
    }

    pub fn has_hir_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.stage == SemanticStage::Hir)
    }

    pub fn is_thir_complete(&self) -> bool {
        self.thir
            .as_ref()
            .is_some_and(|lowered| lowered.file.is_some())
    }

    pub fn blocking_diagnostics(&self) -> impl Iterator<Item = &SemanticDiagnostic> {
        self.diagnostics.iter().filter(|diagnostic| {
            matches!(
                diagnostic.stage,
                SemanticStage::Parse | SemanticStage::Hir | SemanticStage::Import
            )
        })
    }
}

pub fn analyze(input: &str) -> Analysis {
    analyze_with_base(input, None, AnalysisOptions::default())
}

pub fn analyze_with_options(input: &str, options: AnalysisOptions) -> Analysis {
    analyze_with_base(input, None, options)
}

/// Analyze a `.zt` file on disk, resolving imports relative to its directory.
pub fn analyze_path(path: &Path) -> std::io::Result<Analysis> {
    let input = std::fs::read_to_string(path)?;
    let mut ctx = import::ImportContext::with_root(path);
    Ok(analyze_inner(
        &input,
        path.parent(),
        AnalysisOptions::default(),
        &mut ctx,
    ))
}

/// Analyze `input`, resolving `import` expressions relative to `base`.
///
/// `base` is the directory of the importing file; `None` (string-only entry
/// points, REPL) means imports cannot be resolved and yield a diagnostic.
pub fn analyze_with_base(input: &str, base: Option<&Path>, options: AnalysisOptions) -> Analysis {
    let mut ctx = import::ImportContext::default();
    analyze_inner(input, base, options, &mut ctx)
}

/// Analyze `input`, threading the recursive-import `ctx` (cycle stack + module
/// cache) so that `.zt` module imports can be resolved depth-first.
pub(crate) fn analyze_inner(
    input: &str,
    base: Option<&Path>,
    options: AnalysisOptions,
    ctx: &mut import::ImportContext,
) -> Analysis {
    let parsed = zutai_syntax::parse_ast_only(input);
    let parse_diagnostics: Vec<_> = parsed
        .diagnostics()
        .iter()
        .cloned()
        .map(|diagnostic| SemanticDiagnostic {
            stage: SemanticStage::Parse,
            kind: SemanticDiagnosticKind::Parse(diagnostic),
        })
        .collect();

    if parsed.has_errors() {
        return Analysis {
            ast: parsed.into_ast(),
            hir: None,
            thir: None,
            diagnostics: parse_diagnostics,
            pass_reports: Vec::new(),
            import_values: HashMap::new(),
            import_modules: HashMap::new(),
            tlc: None,
        };
    }

    let Some(ast) = parsed.into_ast() else {
        return Analysis {
            ast: None,
            hir: None,
            thir: None,
            diagnostics: parse_diagnostics,
            pass_reports: Vec::new(),
            import_values: HashMap::new(),
            import_modules: HashMap::new(),
            tlc: None,
        };
    };

    let hir = zutai_hir::lower_file_with_options(
        &ast,
        zutai_hir::HirLowerOptions {
            run_passes: options.run_hir_passes,
        },
    );
    let mut diagnostics = parse_diagnostics;
    diagnostics.extend(
        hir.diagnostics
            .iter()
            .cloned()
            .map(|diagnostic| SemanticDiagnostic {
                stage: SemanticStage::Hir,
                kind: SemanticDiagnosticKind::Hir(diagnostic),
            }),
    );

    let mut pass_reports: Vec<_> = hir
        .pass_reports
        .iter()
        .map(|report| SemanticPassReport {
            stage: SemanticStage::Hir,
            name: report.name,
        })
        .collect();

    let mut import_values = HashMap::new();
    let mut import_modules = HashMap::new();
    let thir =
        if hir.diagnostics.is_empty() {
            // Resolve imports before THIR lowering: the resolved types feed type
            // checking, the values/modules are kept for the evaluator, and any
            // failure is a diagnostic (the import then has no type, becoming a
            // THIR Error node).
            let resolved = import::resolve_imports(&hir.file, base, ctx);
            diagnostics.extend(resolved.diagnostics.into_iter().map(|diagnostic| {
                SemanticDiagnostic {
                    stage: SemanticStage::Import,
                    kind: SemanticDiagnosticKind::Import(diagnostic),
                }
            }));
            import_values = resolved.values;
            import_modules = resolved.modules;

            let lowered = zutai_thir::lower_hir_with_options(
                &hir.file,
                zutai_thir::ThirLowerOptions {
                    run_passes: options.run_thir_passes,
                    imports: resolved.types,
                    type_eval_fuel: None,
                },
            );
            pass_reports.extend(
                lowered
                    .pass_reports
                    .iter()
                    .map(|report| SemanticPassReport {
                        stage: SemanticStage::Thir,
                        name: report.name,
                    }),
            );
            diagnostics.extend(lowered.diagnostics.iter().cloned().map(|diagnostic| {
                SemanticDiagnostic {
                    stage: SemanticStage::Thir,
                    kind: SemanticDiagnosticKind::Thir(diagnostic),
                }
            }));
            Some(lowered)
        } else {
            None
        };

    let tlc = thir
        .as_ref()
        .and_then(|t| t.file.as_ref())
        .map(|file| zutai_tlc::lower_thir(file));

    Analysis {
        ast: Some(ast),
        hir: Some(hir),
        thir,
        diagnostics,
        pass_reports,
        import_values,
        import_modules,
        tlc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_stops_before_hir() {
        let analysis = analyze("[1; 2]");

        assert!(analysis.has_parse_errors());
        assert!(analysis.hir.is_none());
        assert!(analysis.thir.is_none());
    }

    #[test]
    fn hir_error_stops_before_thir() {
        let analysis = analyze("missing");

        assert!(!analysis.has_parse_errors());
        assert!(analysis.has_hir_errors());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_none());
    }

    #[test]
    fn structural_key_hir_error_stops_before_thir() {
        let analysis = analyze("{ a = 1; a = 2; }");

        assert!(!analysis.has_parse_errors());
        assert!(analysis.has_hir_errors());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_none());
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind,
                SemanticDiagnosticKind::Hir(zutai_hir::HirDiagnostic {
                    kind: zutai_hir::HirDiagnosticKind::DuplicateRecordField { .. },
                    ..
                })
            )
        }));
    }

    #[test]
    fn valid_parse_and_hir_reaches_thir_stage() {
        let analysis = analyze("x := 1\nx");

        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors());
        assert!(analysis.ast.is_some());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_some());
        assert!(analysis.is_thir_complete());
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn thir_type_error_is_reported_by_semantic_analysis() {
        let analysis = analyze("x :: Int = \"bad\"\nx");

        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_some());
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.kind,
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::TypeMismatch { expected, found },
                    ..
                }) if expected == "Int" && found == "Text"
            )
        }));
    }

    // ── `.zti` imports ────────────────────────────────────────────────────────

    fn imports_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports")
    }

    fn analyze_in_imports(input: &str) -> Analysis {
        analyze_with_base(input, Some(&imports_dir()), AnalysisOptions::default())
    }

    fn has_import_diagnostic(analysis: &Analysis, expected: &ImportDiagnosticKind) -> bool {
        analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.kind,
                SemanticDiagnosticKind::Import(import) if &import.kind == expected
            )
        })
    }

    #[test]
    fn import_with_base_completes() {
        let analysis = analyze_in_imports("cfg := import \"config.zti\"\ncfg.port");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn analyze_path_resolves_relative_import() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/importer.zt");
        let analysis = analyze_path(&path).expect("read importer.zt");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn import_without_base_reports_no_base_directory() {
        let analysis = analyze("cfg := import \"config.zti\"\ncfg.port");
        assert!(!analysis.is_thir_complete());
        assert!(has_import_diagnostic(
            &analysis,
            &ImportDiagnosticKind::NoBaseDirectory
        ));
    }

    #[test]
    fn import_missing_file_reports_file_not_found() {
        let analysis = analyze_in_imports("cfg := import \"nope.zti\"\ncfg");
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::FileNotFound { .. })
        )));
    }

    #[test]
    fn zt_import_data_module_completes() {
        let analysis = analyze_in_imports("m := import \"data_module.zt\"\nm.doubled");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn zt_import_transitive_completes() {
        // chain_top.zt -> chain_mid.zt -> config.zti.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/chain_top.zt");
        let analysis = analyze_path(&path).expect("read chain_top.zt");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn zt_import_function_module_completes() {
        // func_module.zt exports a function; the import must now succeed and
        // produce a complete THIR (no UnsupportedExport diagnostic).
        let analysis = analyze_in_imports("f := import \"func_module.zt\"\nf");
        assert!(
            analysis.is_thir_complete(),
            "expected complete THIR, got diagnostics: {:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn zt_import_cycle_is_reported() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/cycle_a.zt");
        let analysis = analyze_path(&path).expect("read cycle_a.zt");
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. })
        )));
    }

    #[test]
    fn bad_zti_reports_parse_error() {
        let analysis = analyze_in_imports("cfg := import \"bad.zti\"\ncfg");
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ParseError { .. })
        )));
    }

    #[test]
    fn empty_imported_list_still_completes() {
        // Empty `.zti` array → `List(InferVar)`; a free inference variable is
        // allowed in completed THIR.
        let analysis = analyze_in_imports("cfg := import \"empty_list.zti\"\ncfg");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn mixed_imported_list_still_completes() {
        // Heterogeneous `.zti` array → `List(Union(...))`.
        let analysis = analyze_in_imports("cfg := import \"mixed_list.zti\"\ncfg");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn tlc_is_some_for_complete_thir() {
        let analysis = analyze("x := 42\nx");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.tlc.is_some(),
            "expected TLC module for complete program"
        );
    }

    #[test]
    fn tlc_is_none_for_type_error() {
        let analysis = analyze("x :: Int = \"bad\"\nx");
        assert!(!analysis.is_thir_complete());
        assert!(
            analysis.tlc.is_none(),
            "expected no TLC module for type-error program"
        );
    }
}
