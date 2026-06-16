use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;
use winnow::token::take_till;

use crate::ast::{
    ConstraintMethod, Decl, File, MethodName, TypeParam, TypeParamBound, WitnessBody, WitnessField,
};
use crate::span::Span;

use super::expr::{parse_clause_block, parse_expr};
use super::lex::{kw, parse_ident, spanned, ws};
use super::pattern::parse_pattern;
use super::type_expr::{parse_type_atom, parse_type_expr};

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
    // Must be followed by `::`, `:=`, `@` (witness), or a pattern-then-`=` sequence
    tmp.starts_with("::") || tmp.starts_with(":=") || tmp.starts_with('@') || is_nosig_fn_start(tmp)
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

    // Witness: `Constraint @Target :: ...`
    if input.starts_with('@') {
        return parse_witness(input, name, name_span);
    }

    if input.starts_with(":=") {
        // Inferred binding
        ":=".parse_next(input)?;
        ws(input)?;
        let value = parse_expr(input)?;
        let span = name_span.merge(value.span());
        return Ok(Decl::Inferred { name, value, span });
    }

    if input.starts_with("::") {
        // Typed / TypeAlias / Function / Constraint
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

    // Constraint def: `Name :: [<params>] @Target { ... }`
    if input.starts_with('@') {
        return parse_constraint_body(input, name, name_span, params);
    }

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
    let first = parse_type_param(input)?;
    let mut params = vec![first];
    loop {
        ws(input)?;
        if !input.starts_with(',') {
            break;
        }
        ','.parse_next(input)?;
        ws(input)?;
        params.push(parse_type_param(input)?);
    }
    ws(input)?;
    '>'.parse_next(input)?;
    Ok(params)
}

fn parse_type_param(input: &mut &str) -> Result<TypeParam> {
    let (name, span) = spanned(parse_ident).parse_next(input)?;
    ws(input)?;
    // Check `::` before `:` (longer match first)
    if input.starts_with("::") {
        "::".parse_next(input)?;
        ws(input)?;
        let kind = parse_type_expr(input)?;
        let end_span = kind.span();
        return Ok(TypeParam {
            name,
            bounds: vec![],
            kind: Some(Box::new(kind)),
            span: span.merge(end_span),
        });
    }
    if input.starts_with(':') {
        ':'.parse_next(input)?;
        ws(input)?;
        let (first_name, first_span) = spanned(parse_ident).parse_next(input)?;
        let mut bounds = vec![TypeParamBound {
            name: first_name,
            span: first_span,
        }];
        let mut last_span = first_span;
        loop {
            ws(input)?;
            if !input.starts_with('+') {
                break;
            }
            '+'.parse_next(input)?;
            ws(input)?;
            let (bound_name, bound_span) = spanned(parse_ident).parse_next(input)?;
            bounds.push(TypeParamBound {
                name: bound_name,
                span: bound_span,
            });
            last_span = bound_span;
        }
        return Ok(TypeParam {
            name,
            bounds,
            kind: None,
            span: span.merge(last_span),
        });
    }
    Ok(TypeParam {
        name,
        bounds: vec![],
        kind: None,
        span,
    })
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

// ---------------------------------------------------------------------------
// Constraint definition: `Name :: [<params>] @Target { methods... } [derive]`
// ---------------------------------------------------------------------------

fn parse_constraint_body(
    input: &mut &str,
    name: String,
    name_span: Span,
    params: Vec<TypeParam>,
) -> Result<Decl> {
    '@'.parse_next(input)?;
    ws(input)?;
    let target = parse_type_atom(input)?;
    ws(input)?;
    '{'.parse_next(input)?;
    ws(input)?;
    let mut methods = vec![];
    while !input.starts_with('}') && !input.is_empty() {
        methods.push(parse_constraint_method(input)?);
        ws(input)?;
    }
    let (_, close_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    ws(input)?;
    let derivable = consume_contextual_derive(input);
    let span = name_span.merge(close_span);
    Ok(Decl::Constraint {
        name,
        params,
        target,
        methods,
        derivable,
        span,
    })
}

fn parse_constraint_method(input: &mut &str) -> Result<ConstraintMethod> {
    ws(input)?;
    let (name, name_span) = spanned(parse_method_name).parse_next(input)?;
    ws(input)?;
    // Optional `?` for optional method
    let optional = if input.starts_with('?') {
        let rest = &input[1..];
        if rest.starts_with(':') || rest.starts_with(|c: char| c.is_whitespace()) {
            '?'.parse_next(input)?;
            ws(input)?;
            true
        } else {
            false
        }
    } else {
        false
    };
    "::".parse_next(input)?;
    ws(input)?;
    // Optional method-level type params
    let params = if input.starts_with('<') {
        let p = parse_type_param_list(input)?;
        ws(input)?;
        p
    } else {
        vec![]
    };
    let sig = parse_type_expr(input)?;
    ws(input)?;
    // Optional default clause block
    let default = if input.starts_with('{') {
        parse_clause_block(input)?
    } else {
        vec![]
    };
    ws(input)?;
    ';'.parse_next(input)?;
    let end_span = if let Some(last) = default.last() {
        last.span
    } else {
        sig.span()
    };
    let span = name_span.merge(end_span);
    Ok(ConstraintMethod {
        name,
        optional,
        params,
        sig,
        default,
        span,
    })
}

// ---------------------------------------------------------------------------
// Witness: `Constraint @Target :: [<params>] { fields... }` or `:: derive`
// ---------------------------------------------------------------------------

fn parse_witness(input: &mut &str, constraint: String, constraint_span: Span) -> Result<Decl> {
    '@'.parse_next(input)?;
    ws(input)?;
    let target = parse_type_atom(input)?;
    ws(input)?;
    "::".parse_next(input)?;
    ws(input)?;
    // Optional conditional params
    let params = if input.starts_with('<') {
        let p = parse_type_param_list(input)?;
        ws(input)?;
        p
    } else {
        vec![]
    };
    // Body: `{ fields... }` or contextual `derive`
    let (body, body_span) = spanned(|input: &mut &str| {
        if input.starts_with('{') {
            let fields = parse_witness_fields(input)?;
            Ok(WitnessBody::Fields(fields))
        } else if consume_contextual_derive(input) {
            Ok(WitnessBody::Derive)
        } else {
            fail.parse_next(input)
        }
    })
    .parse_next(input)?;
    let span = constraint_span.merge(body_span);
    Ok(Decl::Witness {
        constraint,
        target,
        params,
        body,
        span,
    })
}

fn parse_witness_fields(input: &mut &str) -> Result<Vec<WitnessField>> {
    '{'.parse_next(input)?;
    ws(input)?;
    let mut fields = vec![];
    while !input.starts_with('}') && !input.is_empty() {
        let (field, _) = spanned(parse_witness_field).parse_next(input)?;
        fields.push(field);
        ws(input)?;
    }
    '}'.parse_next(input)?;
    Ok(fields)
}

fn parse_witness_field(input: &mut &str) -> Result<WitnessField> {
    let (name, name_span) = spanned(parse_method_name).parse_next(input)?;
    ws(input)?;
    '='.parse_next(input)?;
    ws(input)?;
    let value = parse_expr(input)?;
    let val_span = value.span();
    ws(input)?;
    if input.starts_with(';') {
        ';'.parse_next(input)?;
    }
    Ok(WitnessField {
        name,
        value,
        span: name_span.merge(val_span),
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn parse_method_name(input: &mut &str) -> Result<MethodName> {
    if input.starts_with('(') {
        '('.parse_next(input)?;
        let op: &str = take_till(0.., ')').parse_next(input)?;
        let name = op.trim().to_string();
        ')'.parse_next(input)?;
        Ok(MethodName::Operator(name))
    } else {
        Ok(MethodName::Ident(parse_ident(input)?))
    }
}

/// Consume `derive` as a contextual keyword (D4).
/// Only matches when `derive` is not followed by an ident-continuation char.
/// Returns true and advances `input` if consumed.
fn consume_contextual_derive(input: &mut &str) -> bool {
    let trimmed = input.trim_start_matches(|c: char| c.is_whitespace());
    if !trimmed.starts_with("derive") {
        return false;
    }
    let after = &trimmed["derive".len()..];
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }
    let leading_ws = input.len() - trimmed.len();
    *input = &input[leading_ws + "derive".len()..];
    true
}
