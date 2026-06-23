pub mod decl;
mod diagnostics;
pub mod expr;
pub mod lex;
pub mod pattern;
pub mod type_expr;

use winnow::Parser;
use winnow::combinator::eof;

use crate::ast::File;
use crate::error::ParseError;
use crate::span::Span;

use self::decl::parse_file;
use self::diagnostics::collect_common_diagnostics;
use self::lex::ws;

pub(crate) fn parse_ast(input: &str) -> Result<File, Vec<ParseError>> {
    let diagnostics = collect_common_diagnostics(input);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    parse_ast_without_common_diagnostics(input)
}

pub(crate) fn parse_ast_without_common_diagnostics(input: &str) -> Result<File, Vec<ParseError>> {
    lex::BASE_PTR.with(|c| c.set(input.as_ptr() as usize));
    lex::DEPTH.with(|c| c.set(0));
    lex::reset_furthest();
    let mut s = input;
    let result = (parse_file, ws, eof)
        .map(|(file, _, _)| file)
        .parse_next(&mut s);

    match result {
        Ok(file) => Ok(file),
        Err(_) => {
            // Backtracking rewinds the winnow cursor to the start of the last
            // failed alternative, so `input.len() - s.len()` usually points at
            // a construct's opening token rather than where parsing actually
            // got stuck. The furthest-progress high-water mark is a far better
            // estimate of the offending location.
            let consumed = input.len() - s.len();
            let offset = lex::furthest_offset().max(consumed).min(input.len());
            let (span, message) = describe_parse_failure(input, offset);
            Err(vec![ParseError::new(span, message)])
        }
    }
}

/// Build a human-readable message and a pointed span for a generic parse
/// failure at `offset`, by inspecting the token the parser stalled on.
fn describe_parse_failure(input: &str, offset: usize) -> (Span, String) {
    let token_start = skip_trivia(input, offset.min(input.len()));

    if token_start >= input.len() {
        // Only whitespace/comments remain: the input ended too early.
        let end = input.trim_end().len();
        let start = end.saturating_sub(1);
        return (
            Span::new(start, end.max(1)),
            "unexpected end of input".to_string(),
        );
    }

    let token = token_at(input, token_start);
    let span = Span::new(token_start, token_start + token.len());
    (span, format!("unexpected `{}`", escape_token(&token)))
}

/// Advance past whitespace, line comments, and block comments starting at `pos`.
fn skip_trivia(input: &str, pos: usize) -> usize {
    let bytes = input.as_bytes();
    let mut i = pos;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let rest = &input[i..];
        if rest.starts_with("--[") {
            // Block comment: scan to the matching `]--`, else to end.
            match rest.find("]--") {
                Some(n) => i += n + 3,
                None => return input.len(),
            }
        } else if rest.starts_with("--") {
            match rest.find('\n') {
                Some(n) => i += n + 1,
                None => return input.len(),
            }
        } else {
            return i;
        }
    }
}

/// Extract a single token-like slice beginning at `start` (a non-trivia byte).
fn token_at(input: &str, start: usize) -> String {
    let rest = &input[start..];
    let first = rest.chars().next().unwrap_or('\0');
    let len = if first.is_alphanumeric() || first == '_' {
        rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len())
    } else if first == '#' {
        let body = rest[1..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .map(|n| n + 1)
            .unwrap_or(rest.len());
        body.max(first.len_utf8())
    } else if "{}[]();,".contains(first) {
        first.len_utf8()
    } else {
        // Operator-like run: stop at whitespace, identifiers, or delimiters.
        rest.find(|c: char| {
            c.is_whitespace() || c.is_alphanumeric() || c == '_' || "{}[]();,".contains(c)
        })
        .unwrap_or(rest.len())
        .max(first.len_utf8())
    };
    let len = clamp_char_boundary(rest, len.min(24));
    rest[..len].to_string()
}

/// Largest char-boundary offset in `s` that is `<= max`.
fn clamp_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut m = max;
    while m > 0 && !s.is_char_boundary(m) {
        m -= 1;
    }
    m
}

/// Render control characters in a token snippet so messages stay on one line.
fn escape_token(token: &str) -> String {
    token
        .chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            other => other.to_string(),
        })
        .collect()
}
