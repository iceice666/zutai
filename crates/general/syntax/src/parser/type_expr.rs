use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;

use crate::ast::{TypeExpr, TypeRecordField, TypeTupleItem};
use crate::span::Span;

use super::lex::{
    enter_delimiter, op, parse_atom_name, parse_bool_false, parse_bool_true, parse_field_name,
    parse_ident, spanned, ws,
};

/// Entry for type-expression parsing (level 10: `->` right-assoc).
pub fn parse_type_expr(input: &mut &str) -> Result<TypeExpr> {
    ws(input)?;
    let lhs = parse_type_application(input)?;

    let checkpoint = *input;
    ws(input)?;
    if input.starts_with("->") && !input.starts_with("->>") {
        "->".parse_next(input)?;
        ws(input)?;
        let rhs = parse_type_expr(input)?; // right-recursive
        let span = lhs.span().merge(rhs.span());
        return Ok(TypeExpr::Arrow {
            from: Box::new(lhs),
            to: Box::new(rhs),
            span,
        });
    }
    *input = checkpoint;

    Ok(lhs)
}

/// Type constructor application, e.g. `List Int` or `Pair Text Int`.
fn parse_type_application(input: &mut &str) -> Result<TypeExpr> {
    let mut node = parse_type_postfix(input)?;

    loop {
        let checkpoint = *input;
        consume_inline_ws(input);
        if !starts_type_atom(input) {
            *input = checkpoint;
            break;
        }
        let arg = parse_type_postfix(input)?;
        let span = node.span().merge(arg.span());
        node = TypeExpr::Apply {
            func: Box::new(node),
            arg: Box::new(arg),
            span,
        };
    }

    Ok(node)
}

/// Level 1 in type context: field access, optional chaining, postfix `?`.
fn parse_type_postfix(input: &mut &str) -> Result<TypeExpr> {
    let mut node = parse_type_atom(input)?;

    loop {
        let checkpoint = *input;
        ws(input)?;
        if input.starts_with("?.") {
            "?.".parse_next(input)?;
            ws(input)?;
            let field = parse_field_name(input)?;
            let span = node.span().merge(Span::new(0, 0)); // approximate
            node = TypeExpr::Access {
                receiver: Box::new(node),
                field,
                span,
            };
        } else if input.starts_with('.') && !input.starts_with("..") {
            '.'.parse_next(input)?;
            ws(input)?;
            let field = parse_field_name(input)?;
            let span = node.span();
            node = TypeExpr::Access {
                receiver: Box::new(node),
                field,
                span,
            };
        } else if input.starts_with('?') && !input.starts_with("??") && !input.starts_with("?.") {
            // postfix `?` — only in type context
            let (_, q_span) = spanned(op("?")).parse_next(input)?;
            let span = node.span().merge(q_span);
            node = TypeExpr::Optional {
                inner: Box::new(node),
                span,
            };
        } else {
            *input = checkpoint;
            break;
        }
    }

    Ok(node)
}

/// Atom-level type: `{`, `[`, `(`, atom, ident, true/false, or ExprEscape.
fn parse_type_atom(input: &mut &str) -> Result<TypeExpr> {
    ws(input)?;

    if input.starts_with('{') {
        return parse_type_record(input);
    }
    if input.starts_with('[') {
        return parse_type_union(input);
    }
    if input.starts_with('(') {
        return parse_type_tuple(input);
    }
    if input.starts_with('#') {
        let (name, span) = spanned(parse_atom_name).parse_next(input)?;
        return Ok(TypeExpr::Atom { name, span });
    }
    if let Ok((_, span)) = spanned(parse_bool_true).parse_next(input) {
        return Ok(TypeExpr::True(span));
    }
    if let Ok((_, span)) = spanned(parse_bool_false).parse_next(input) {
        return Ok(TypeExpr::False(span));
    }
    if let Ok((name, span)) = spanned(parse_ident).parse_next(input) {
        return Ok(TypeExpr::Ident { name, span });
    }

    // Fall through to ExprEscape for things like type-application (`List Int`)
    // We parse a single application-level expression and wrap it.
    super::expr::parse_application_as_type_escape(input)
}

fn starts_type_atom(input: &str) -> bool {
    input.starts_with('[')
        || input.starts_with('(')
        || input.starts_with('#')
        || input
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
}

fn consume_inline_ws(input: &mut &str) {
    let trimmed = input.trim_start_matches([' ', '\t']);
    *input = trimmed;
}

// ---------------------------------------------------------------------------
// Type record: `{ field : TypeExpr; ... }` or `{ field? : TypeExpr; ... }`
// ---------------------------------------------------------------------------

fn parse_type_record(input: &mut &str) -> Result<TypeExpr> {
    let (fields, span) = spanned(parse_type_record_inner).parse_next(input)?;
    Ok(TypeExpr::Record { fields, span })
}

fn parse_type_record_inner(input: &mut &str) -> Result<Vec<TypeRecordField>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut fields = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let name_start = Span::new(0, 0);
        let name = parse_field_name(input)?;
        ws(input)?;

        // optional-field marker `?`
        let optional =
            if input.starts_with('?') && !input.starts_with("?.") && !input.starts_with("??") {
                '?'.parse_next(input)?;
                ws(input)?;
                true
            } else {
                false
            };

        // `:` (not `::`)
        if input.starts_with(':') && !input.starts_with("::") {
            ':'.parse_next(input)?;
        } else {
            return fail.parse_next(input);
        }
        ws(input)?;
        let ty = parse_type_expr(input)?;
        let span = name_start.merge(ty.span());
        ws(input)?;
        ';'.parse_next(input)?;
        fields.push(TypeRecordField {
            name,
            optional,
            ty,
            span,
        });
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok(fields)
}

// ---------------------------------------------------------------------------
// Type union: `[ TypeExpr; ... ]`
// ---------------------------------------------------------------------------

fn parse_type_union(input: &mut &str) -> Result<TypeExpr> {
    let (items, span) = spanned(parse_type_union_inner).parse_next(input)?;
    Ok(TypeExpr::Union { items, span })
}

fn parse_type_union_inner(input: &mut &str) -> Result<Vec<TypeExpr>> {
    '['.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut items = vec![];
    loop {
        ws(input)?;
        if input.starts_with(']') {
            break;
        }
        let ty = parse_type_expr(input)?;
        ws(input)?;
        ';'.parse_next(input)?;
        items.push(ty);
    }
    ws(input)?;
    ']'.parse_next(input)?;
    Ok(items)
}

// ---------------------------------------------------------------------------
// Type tuple: `()` or `(item, item, ...)`
// ---------------------------------------------------------------------------

fn parse_type_tuple(input: &mut &str) -> Result<TypeExpr> {
    let (items, span) = spanned(parse_type_tuple_inner).parse_next(input)?;
    Ok(TypeExpr::Tuple { items, span })
}

fn parse_type_tuple_inner(input: &mut &str) -> Result<Vec<TypeTupleItem>> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;

    if input.starts_with(')') {
        ')'.parse_next(input)?;
        return Ok(vec![]);
    }

    let first = parse_type_tuple_item(input)?;
    ws(input)?;

    if !input.starts_with(',') {
        // Single item without comma — treat as a grouped type, not a tuple type.
        // Actually per spec, a single-element (no comma) parenthesized type IS a
        // group. We succeed here but caller might wrap differently; return one item.
        ')'.parse_next(input)?;
        return Ok(vec![first]);
    }

    let mut items = vec![first];
    while input.starts_with(',') {
        ','.parse_next(input)?;
        ws(input)?;
        if input.starts_with(')') {
            break;
        }
        items.push(parse_type_tuple_item(input)?);
        ws(input)?;
    }
    ws(input)?;
    ')'.parse_next(input)?;
    Ok(items)
}

fn parse_type_tuple_item(input: &mut &str) -> Result<TypeTupleItem> {
    let checkpoint = *input;
    // Try named: `field_name ':' type`
    if let Ok(name) = parse_field_name(input) {
        ws(input)?;
        if input.starts_with(':') && !input.starts_with("::") {
            ':'.parse_next(input)?;
            ws(input)?;
            let ty = parse_type_expr(input)?;
            let span = ty.span();
            return Ok(TypeTupleItem::Named { name, ty, span });
        }
    }
    *input = checkpoint;
    let ty = parse_type_expr(input)?;
    Ok(TypeTupleItem::Positional(ty))
}
