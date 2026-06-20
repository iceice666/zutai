use winnow::Parser;
use winnow::Result;
use winnow::combinator::{alt, fail, opt, peek};
use winnow::token::one_of;

use crate::ast::{Pattern, RecordPatternField, TuplePatternItem};
use crate::span::Span;

use super::lex::{
    enter_delimiter, parse_atom_name, parse_bool_false, parse_bool_true, parse_field_name,
    parse_ident, parse_number_value, parse_string, spanned, ws,
};
use crate::ast::Expr;

/// Parse exactly one pattern atom. Caller drives repetition with the
/// appropriate terminator.
pub fn parse_pattern(input: &mut &str) -> Result<Pattern> {
    ws(input)?;
    alt((
        parse_tuple_pattern,
        parse_record_pattern,
        parse_wildcard,
        parse_pattern_atom,
        parse_pattern_literal,
        parse_pattern_ident,
    ))
    .parse_next(input)
}

fn parse_wildcard(input: &mut &str) -> Result<Pattern> {
    let (_, span) = spanned('_').parse_next(input)?;
    // Verify not followed by ident continuation
    if let Some(c) = input.chars().next()
        && (c.is_ascii_alphanumeric() || c == '_')
    {
        return fail.parse_next(input);
    }
    Ok(Pattern::Wildcard(span))
}

fn parse_pattern_atom(input: &mut &str) -> Result<Pattern> {
    let (name, atom_span) = spanned(parse_atom_name).parse_next(input)?;
    ws(input)?;
    if input.starts_with('{') {
        let (fields, rec_span) = spanned(parse_record_pattern_inner).parse_next(input)?;
        let span = atom_span.merge(rec_span);
        return Ok(Pattern::TaggedValue {
            tag: name,
            payload: fields,
            span,
        });
    }
    if input.starts_with('(') {
        let (fields, tuple_span) = spanned(parse_tagged_tuple_pattern_payload).parse_next(input)?;
        let span = atom_span.merge(tuple_span);
        return Ok(Pattern::TaggedValue {
            tag: name,
            payload: fields,
            span,
        });
    }
    Ok(Pattern::Atom {
        name,
        span: atom_span,
    })
}

fn parse_pattern_literal(input: &mut &str) -> Result<Pattern> {
    // bool literals first (they are keywords — must not be parsed as ident)
    if let Ok((_, span)) = spanned(parse_bool_true).parse_next(input) {
        return Ok(Pattern::True(span));
    }
    if let Ok((_, span)) = spanned(parse_bool_false).parse_next(input) {
        return Ok(Pattern::False(span));
    }
    // string
    if let Ok((s, span)) = spanned(parse_string).parse_next(input) {
        return Ok(Pattern::String { value: s, span });
    }
    // number (integer or float)
    let (expr, span) = spanned(parse_number_value).parse_next(input)?;
    let pat = match expr {
        Expr::Integer { value, .. } => Pattern::Integer { value, span },
        Expr::Float { value, .. } => Pattern::Float { value, span },
        _ => unreachable!(),
    };
    Ok(pat)
}

fn parse_pattern_ident(input: &mut &str) -> Result<Pattern> {
    let (name, span) = spanned(parse_ident).parse_next(input)?;
    Ok(Pattern::Ident { name, span })
}

// ---------------------------------------------------------------------------
// Tuple pattern: `()` or `(item, item, ...)`
// ---------------------------------------------------------------------------

pub fn parse_tuple_pattern(input: &mut &str) -> Result<Pattern> {
    ws(input)?;
    let start = Span::new(input.len(), input.len()); // placeholder; set below via spanned

    let ((items, span), _) = (spanned(parse_tuple_pattern_inner), ws).parse_next(input)?;
    let _ = start;
    Ok(Pattern::Tuple { items, span })
}

fn parse_tuple_pattern_inner(input: &mut &str) -> Result<Vec<TuplePatternItem>> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;

    if peek(opt(one_of(')'))).parse_next(input)?.is_some() && input.starts_with(')') {
        ')'.parse_next(input)?;
        return Ok(vec![]);
    }

    let first = parse_tuple_pattern_item(input)?;
    ws(input)?;

    if !input.starts_with(',') {
        // Single-element without comma → not a tuple; backtrack would happen
        // at the alt() level since we haven't committed. But we've consumed
        // '(' already, so fail.
        return fail.parse_next(input);
    }

    let mut items = vec![first];
    while input.starts_with(',') {
        ','.parse_next(input)?;
        ws(input)?;
        if input.starts_with(')') {
            break;
        }
        items.push(parse_tuple_pattern_item(input)?);
        ws(input)?;
    }
    ws(input)?;
    ')'.parse_next(input)?;
    Ok(items)
}

fn parse_tuple_pattern_item(input: &mut &str) -> Result<TuplePatternItem> {
    // Try named: `field_name '=' pattern`
    let checkpoint = *input;
    if let Ok(name) = parse_field_name(input) {
        ws(input)?;
        if input.starts_with('=') && !input.starts_with("==") {
            '='.parse_next(input)?;
            ws(input)?;
            let pat = parse_pattern(input)?;
            let span = pat.span();
            return Ok(TuplePatternItem::Named {
                name,
                pattern: pat,
                span,
            });
        }
    }
    // Restore and parse as positional
    *input = checkpoint;
    let pat = parse_pattern(input)?;
    Ok(TuplePatternItem::Positional(pat))
}

fn parse_tagged_tuple_pattern_payload(input: &mut &str) -> Result<Vec<RecordPatternField>> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;

    let mut fields = vec![];
    let mut index = 0usize;
    if !input.starts_with(')') {
        loop {
            let item = parse_tuple_pattern_item(input)?;
            match item {
                TuplePatternItem::Named {
                    name,
                    pattern,
                    span,
                } => fields.push(RecordPatternField {
                    name,
                    pattern,
                    span,
                }),
                TuplePatternItem::Positional(pattern) => {
                    let span = pattern.span();
                    fields.push(RecordPatternField {
                        name: index.to_string(),
                        pattern,
                        span,
                    });
                }
            }
            index += 1;
            ws(input)?;
            if input.starts_with(',') {
                ','.parse_next(input)?;
                ws(input)?;
                if input.starts_with(')') {
                    break;
                }
            } else {
                break;
            }
        }
    }
    ws(input)?;
    ')'.parse_next(input)?;
    Ok(fields)
}

// ---------------------------------------------------------------------------
// Record pattern: `{ field = pat; ... }`
// ---------------------------------------------------------------------------

pub fn parse_record_pattern(input: &mut &str) -> Result<Pattern> {
    ws(input)?;
    let (fields, span) = spanned(parse_record_pattern_inner).parse_next(input)?;
    Ok(Pattern::Record { fields, span })
}

fn parse_record_pattern_inner(input: &mut &str) -> Result<Vec<RecordPatternField>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut fields = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let name = parse_field_name(input)?;
        ws(input)?;
        // Must be `=` (not `==`)
        if input.starts_with('=') && !input.starts_with("==") {
            '='.parse_next(input)?;
        } else {
            return fail.parse_next(input);
        }
        ws(input)?;
        let pat = parse_pattern(input)?;
        let span = pat.span();
        ws(input)?;
        ';'.parse_next(input)?;
        fields.push(RecordPatternField {
            name,
            pattern: pat,
            span,
        });
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok(fields)
}
