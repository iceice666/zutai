use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use miette::{Diagnostic, LabeledSpan, NamedSource, SourceCode};
use thiserror::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let path = parse_file_arg()?;
    let ext = extension_or_error(&path)?;
    let contents = fs::read_to_string(&path)?;

    match ext.as_str() {
        "zti" => {
            let ast =
                zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
            print_ast("zti", &ast);
        }
        "zt" => {
            let analysis = zutai_semantic::analyze(&contents);
            let parse_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|diagnostic| match &diagnostic.kind {
                    zutai_semantic::SemanticDiagnosticKind::Parse(diagnostic) => {
                        Some(diagnostic.clone())
                    }
                    _ => None,
                })
                .collect();
            if !parse_errors.is_empty() {
                print_zt_errors(&path, &contents, &parse_errors);
                std::process::exit(1);
            }

            let hir_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.stage == zutai_semantic::SemanticStage::Hir)
                .collect();
            if !hir_errors.is_empty() {
                print_hir_errors(&hir_errors);
                std::process::exit(1);
            }

            if let Some(ast) = analysis.ast.as_ref() {
                print_ast("zt", ast);
            } else {
                eprintln!("parse produced no AST");
                std::process::exit(1);
            }
        }
        other => return Err(format!("Unsupported extension: {other}").into()),
    }

    Ok(())
}

fn print_hir_errors(errs: &[&zutai_semantic::SemanticDiagnostic]) {
    for err in errs {
        eprintln!("semantic error: {err:?}");
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

fn parse_file_arg() -> Result<String, Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let path = args.next().ok_or("usage: zutai-cli <path>")?;

    if args.next().is_some() {
        return Err("usage: zutai-cli <path>".into());
    }

    Ok(path)
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
