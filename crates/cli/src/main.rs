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
        "zt" => match zutai_syntax::parse(&contents) {
            Ok(ast) => print_ast("zt", &ast),
            Err(errs) => {
                print_zt_errors(&path, &contents, errs);
                std::process::exit(1);
            }
        },
        other => return Err(format!("Unsupported extension: {other}").into()),
    }

    Ok(())
}

fn print_zt_errors(path: &str, contents: &str, errs: Vec<zutai_syntax::ParseError>) {
    for err in errs {
        eprintln!(
            "{:?}",
            miette::Report::new(ZtParseDiagnostic::new(path, contents, err))
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
    help: Option<&'static str>,
    label: String,
    span: (usize, usize),
}

impl ZtParseDiagnostic {
    fn new(path: &str, contents: &str, err: zutai_syntax::ParseError) -> Self {
        let start = err.span.start as usize;
        let end = err.span.end as usize;
        let clamped_start = start.min(contents.len());
        let max_len = contents.len().saturating_sub(clamped_start);
        let len = end.saturating_sub(start).max(1).min(max_len.max(1));
        Self {
            source_code: NamedSource::new(path, contents.to_string()),
            message: err.message,
            code: err.kind.code(),
            help: err.kind.help(),
            label: err.kind.label().to_string(),
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
        let err = zutai_syntax::parse(contents)
            .expect_err("fixture should fail")
            .remove(0);

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
