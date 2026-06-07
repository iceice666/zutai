//! Semantic analysis pipeline for Zutai general mode (`.zt`).
//!
//! This crate is the facade that wires parser AST lowering into HIR and then
//! THIR. It deliberately keeps stage results separate so callers can inspect
//! partial output when a later semantic phase fails or is not implemented yet.

#[derive(Debug)]
pub struct Analysis {
    pub ast: Option<zutai_syntax::File>,
    pub hir: Option<zutai_hir::LoweredHir>,
    pub thir: Option<zutai_thir::LoweredThir>,
    pub diagnostics: Vec<SemanticDiagnostic>,
    pub pass_reports: Vec<SemanticPassReport>,
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
    Thir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticDiagnosticKind {
    Parse(zutai_syntax::Diagnostic),
    Hir(zutai_hir::HirDiagnostic),
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
            matches!(diagnostic.stage, SemanticStage::Parse | SemanticStage::Hir)
        })
    }
}

pub fn analyze(input: &str) -> Analysis {
    analyze_with_options(input, AnalysisOptions::default())
}

pub fn analyze_with_options(input: &str, options: AnalysisOptions) -> Analysis {
    let parsed = zutai_syntax::parse(input);
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
        };
    }

    let Some(ast) = parsed.into_ast() else {
        return Analysis {
            ast: None,
            hir: None,
            thir: None,
            diagnostics: parse_diagnostics,
            pass_reports: Vec::new(),
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

    let thir = if hir.diagnostics.is_empty() {
        let lowered = zutai_thir::lower_hir_with_options(
            &hir.file,
            zutai_thir::ThirLowerOptions {
                run_passes: options.run_thir_passes,
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

    Analysis {
        ast: Some(ast),
        hir: Some(hir),
        thir,
        diagnostics,
        pass_reports,
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
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind,
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::TypeCheckerNotImplemented,
                    ..
                })
            )
        }));
    }
}
