//! Syntax support for Zutai general mode (`.zt`).
//!
//! This crate contains the parser and AST definitions for general-mode files.
//! See [`parse`] for the entry point.

pub mod ast;
pub mod diagnostic;
pub mod error;
pub mod line_index;
pub mod numlit;
pub mod parser;
pub mod posit;
pub mod span;
pub mod syntax;

mod display;

#[cfg(test)]
mod tests;

pub use ast::File;
pub use diagnostic::{
    Applicability, Diagnostic, DiagnosticFix, DiagnosticLabel, LabelStyle, Severity, TextEdit,
};
pub use error::{ParseError, ParseErrorKind};
pub use line_index::{LineCol, LineIndex, Utf16LineCol};
pub use span::Span;
pub use syntax::{
    SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, Token, ZutaiLang, parse_lossless, tokenize,
};

#[derive(Debug)]
pub struct Parse {
    syntax: SyntaxNode,
    ast: Option<File>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub struct AstParse {
    ast: Option<File>,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    pub fn syntax(&self) -> &SyntaxNode {
        &self.syntax
    }

    pub fn ast(&self) -> Option<&File> {
        self.ast.as_ref()
    }

    pub fn into_ast(self) -> Option<File> {
        self.ast
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

impl AstParse {
    pub fn ast(&self) -> Option<&File> {
        self.ast.as_ref()
    }

    pub fn into_ast(self) -> Option<File> {
        self.ast
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Parse a `.zt` source file.
///
/// Always returns a lossless concrete syntax tree. The lowered AST is present
/// when parsing succeeds, and diagnostics contain any syntax errors found.
pub fn parse(input: &str) -> Parse {
    let syntax = syntax::parse_lossless(input);
    let ast_parse = parse_ast_only(input);
    Parse {
        syntax,
        ast: ast_parse.ast,
        diagnostics: ast_parse.diagnostics,
    }
}

/// Parse a `.zt` source file into the AST without constructing the lossless
/// concrete syntax tree.
///
/// Use this entry point for compiler stages that only need the AST and
/// diagnostics. Use [`parse`] when callers need token-preserving syntax for
/// editor features, formatting, or concrete syntax inspection.
pub fn parse_ast_only(input: &str) -> AstParse {
    match parser::parse_ast(input) {
        Ok(ast) => AstParse {
            ast: Some(ast),
            diagnostics: Vec::new(),
        },
        Err(errors) => AstParse {
            ast: parser::parse_ast_without_common_diagnostics(input).ok(),
            diagnostics: errors
                .into_iter()
                .map(Diagnostic::from_parse_error)
                .collect(),
        },
    }
}
