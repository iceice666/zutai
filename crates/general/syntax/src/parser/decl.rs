use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;

use crate::ast::{Decl, File, TypeParam};
use crate::span::Span;

use super::expr::{parse_clause_block, parse_expr};
use super::lex::{kw, parse_ident, spanned, ws};
use super::pattern::parse_pattern;
use super::type_expr::parse_type_expr;

/// Top-level parse entry: `decl* final_expr`.
pub fn parse_file(input: &mut &str) -> Result<File> {
    ws(input)?;

    let mut decls = vec![];
    loop {
        let checkpoint = *input;
        if is_decl_start(input) {
            match parse_top_decl(input) {
                Ok(d) => {
                    decls.push(d);
                    ws(input)?;
                }
                Err(_) => {
                    *input = checkpoint;
                    break;
                }
            }
        } else {
            break;
        }
    }

    ws(input)?;
    let final_expr = parse_expr(input)?;
    ws(input)?;

    let span = decls
        .first()
        .map(|d| d.span())
        .unwrap_or(final_expr.span())
        .merge(final_expr.span());

    Ok(File {
        decls,
        final_expr,
        span,
    })
}

/// Cheap peek: does the current position look like the start of a top-level decl?
fn is_decl_start(input: &mut &str) -> bool {
    let mut tmp = *input;
    // Skip whitespace
    while tmp.starts_with(|c: char| c.is_whitespace()) {
        tmp = &tmp[1..];
    }
    // Must start with an identifier
    if !tmp.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return false;
    }
    // Consume ident
    while tmp.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        tmp = &tmp[1..];
    }
    // Skip ws
    while tmp.starts_with(|c: char| c.is_whitespace()) {
        tmp = &tmp[1..];
    }
    // Must be followed by `::`, `:=`, or a pattern-then-`=` sequence
    tmp.starts_with("::") || tmp.starts_with(":=") || is_nosig_fn_start(tmp)
}

/// Peek whether the text (after an ident and ws) looks like a no-sig fn:
/// pattern(s) followed by `=` (not `==`).
fn is_nosig_fn_start(s: &str) -> bool {
    // Simplification: if the first non-ws token after the name is `(` or `#` or
    // an ident/`_`, and later there's a bare `=` at depth 0, it's a no-sig fn.
    // We only need to avoid false-positives here; failures fall through to final expr.
    if s.starts_with('(') || s.starts_with('#') || s.starts_with('_') {
        return true;
    }
    if s.starts_with(|c: char| c.is_ascii_alphabetic()) {
        // Ensure it's not a keyword-only expression starter
        return true;
    }
    false
}

pub fn parse_top_decl(input: &mut &str) -> Result<Decl> {
    ws(input)?;
    let (name, name_span) = spanned(parse_ident).parse_next(input)?;
    ws(input)?;

    if input.starts_with(":=") {
        // Inferred binding
        ":=".parse_next(input)?;
        ws(input)?;
        let value = parse_expr(input)?;
        let span = name_span.merge(value.span());
        return Ok(Decl::Inferred { name, value, span });
    }

    if input.starts_with("::") {
        // Typed / TypeAlias / Function
        "::".parse_next(input)?;
        ws(input)?;
        return parse_top_decl_after_sig(input, name, name_span);
    }

    // No-sig fn: one or more patterns followed by `=`
    parse_no_sig_fn(input, name, name_span)
}

fn parse_top_decl_after_sig(input: &mut &str, name: String, name_span: Span) -> Result<Decl> {
    // Optional type-param list `<A, B, ...>` — only legal here
    let params = if input.starts_with('<') {
        parse_type_param_list(input)?
    } else {
        vec![]
    };
    ws(input)?;

    // Type alias: `:: [<params>] type TypeExpr`
    if input.starts_with("type") {
        if let Ok(_) = kw("type").parse_next(input) {
            ws(input)?;
            let ty = parse_type_expr(input)?;
            let span = name_span.merge(ty.span());
            return Ok(Decl::TypeAlias {
                name,
                params,
                ty,
                span,
            });
        }
    }

    // Typed binding or function: parse TypeExpr, then peek `=` or `{`
    let sig = parse_type_expr(input)?;
    ws(input)?;

    if input.starts_with('=') && !input.starts_with("==") {
        // Typed value binding
        if !params.is_empty() {
            return fail.parse_next(input); // params not valid for typed binding
        }
        '='.parse_next(input)?;
        ws(input)?;
        let value = parse_expr(input)?;
        let span = name_span.merge(value.span());
        return Ok(Decl::Typed {
            name,
            ty: sig,
            value,
            span,
        });
    }

    if input.starts_with('{') {
        // Function with clause block
        let clauses = parse_clause_block(input)?;
        let end_span = clauses.last().map(|c| c.span).unwrap_or(sig.span());
        let span = name_span.merge(end_span);
        return Ok(Decl::Function {
            name,
            params,
            sig,
            clauses,
            span,
        });
    }

    fail.parse_next(input)
}

fn parse_type_param_list(input: &mut &str) -> Result<Vec<TypeParam>> {
    '<'.parse_next(input)?;
    ws(input)?;
    let (first_name, first_span) = spanned(parse_ident).parse_next(input)?;
    let mut params = vec![TypeParam {
        name: first_name,
        span: first_span,
    }];
    loop {
        ws(input)?;
        if !input.starts_with(',') {
            break;
        }
        ','.parse_next(input)?;
        ws(input)?;
        let (name, span) = spanned(parse_ident).parse_next(input)?;
        params.push(TypeParam { name, span });
    }
    ws(input)?;
    '>'.parse_next(input)?;
    Ok(params)
}

fn parse_no_sig_fn(input: &mut &str, name: String, name_span: Span) -> Result<Decl> {
    let mut patterns = vec![];
    loop {
        ws(input)?;
        if input.starts_with('=') && !input.starts_with("==") {
            break;
        }
        match parse_pattern(input) {
            Ok(p) => patterns.push(p),
            Err(_) => return fail.parse_next(input),
        }
    }
    if patterns.is_empty() {
        return fail.parse_next(input);
    }
    '='.parse_next(input)?;
    ws(input)?;
    let body = parse_expr(input)?;
    let span = name_span.merge(body.span());
    Ok(Decl::NoSigFn {
        name,
        patterns,
        body,
        span,
    })
}
