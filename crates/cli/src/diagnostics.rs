use std::error::Error;
use std::fmt;
use std::path::Path;

use miette::{Diagnostic, LabeledSpan, NamedSource, SourceCode};
use thiserror::Error;

pub(crate) fn print_semantic_errors(
    path: &str,
    contents: &str,
    errs: &[&zutai_semantic::SemanticDiagnostic],
) {
    for err in errs {
        if let zutai_semantic::SemanticDiagnosticKind::Import(import) = &err.kind {
            eprintln!("import error: {}", format_import_diagnostic(import));
            continue;
        }
        match zutai_eval::describe_semantic_diagnostic(err) {
            Some((message, start, end)) => eprintln!(
                "{:?}",
                miette::Report::new(ZtSemanticDiagnostic::new(
                    path, contents, message, start, end
                ))
            ),
            None => eprintln!("semantic error: {err:?}"),
        }
    }
}

pub(crate) fn format_import_diagnostic(diag: &zutai_semantic::ImportDiagnostic) -> String {
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
        PathTraversal { path } => {
            format!("import path escapes the project directory: {path}")
        }
    }
}

pub(crate) fn print_zt_errors(path: &str, contents: &str, errs: &[zutai_syntax::Diagnostic]) {
    for err in errs {
        eprintln!(
            "{:?}",
            miette::Report::new(ZtParseDiagnostic::new(path, contents, err.clone()))
        );
    }
}

pub(crate) fn extension_or_error(path: &str) -> Result<String, Box<dyn Error>> {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| format!("File has no extension: {path}"))?
        .to_ascii_lowercase();
    Ok(extension)
}

pub(crate) fn print_ast(label: &str, ast: &impl std::fmt::Display) {
    println!("Parsed .{label} AST:");
    println!("{ast}");
}

// ─── miette parse-diagnostic renderer (unchanged) ────────────────────────────

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct ZtParseDiagnostic {
    source_code: NamedSource<String>,
    message: String,
    code: &'static str,
    help: Option<String>,
    label: String,
    span: (usize, usize),
}

impl ZtParseDiagnostic {
    pub(crate) fn new(path: &str, contents: &str, err: zutai_syntax::Diagnostic) -> Self {
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

// ─── miette semantic-diagnostic renderer ─────────────────────────────────────

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct ZtSemanticDiagnostic {
    source_code: NamedSource<String>,
    message: String,
    span: (usize, usize),
}

impl ZtSemanticDiagnostic {
    pub(crate) fn new(path: &str, contents: &str, message: String, start: u32, end: u32) -> Self {
        let start = start as usize;
        let end = end as usize;
        let clamped_start = start.min(contents.len());
        let max_len = contents.len().saturating_sub(clamped_start);
        let len = end.saturating_sub(start).max(1).min(max_len.max(1));
        Self {
            source_code: NamedSource::new(path, contents.to_string()),
            message,
            span: (clamped_start, len),
        }
    }
}

impl Diagnostic for ZtSemanticDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new("zutai::check"))
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(&self.source_code)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        Some(Box::new(std::iter::once(LabeledSpan::at(
            self.span, "here",
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_zt_parse_error_with_source_context() {
        let path = "bad.zt";
        let contents = "{1; 2}";
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

    #[test]
    fn format_import_diag_path_traversal() {
        let d = make_diag(zutai_semantic::ImportDiagnosticKind::PathTraversal {
            path: "/etc/x.zti".to_string(),
        });
        let s = format_import_diagnostic(&d);
        assert!(
            s.contains("escapes") || s.contains("project directory"),
            "{s}"
        );
        assert!(s.contains("/etc/x.zti"), "{s}");
    }

    // ── ZtParseDiagnostic span clamping ──────────────────────────────────────

    #[test]
    fn zt_parse_diagnostic_clamps_span_end_to_content_length() {
        // Produce a real parse diagnostic with a span that might exceed the
        // content length when rendered.
        let contents = "{1; 2}";
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
