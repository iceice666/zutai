use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;

use crate::ast::{
    EffectOp, EffectRow, RowTail, SelectField, TypeExpr, TypeRecordField, TypeTupleItem,
    UnionVariant,
};
use crate::span::Span;

use super::lex::{
    enter_delimiter, kw, op, parse_atom_name, parse_bool_false, parse_bool_true, parse_field_name,
    parse_ident, spanned, ws,
};

/// Entry for type-expression parsing (level 10: `->` right-assoc).
pub fn parse_type_expr(input: &mut &str) -> Result<TypeExpr> {
    ws(input)?;
    let lhs = parse_type_effect(input)?;

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

fn parse_type_effect(input: &mut &str) -> Result<TypeExpr> {
    let base = parse_type_application(input)?;
    let checkpoint = *input;
    ws(input)?;
    if !input.starts_with('!') || input.starts_with("!=") {
        *input = checkpoint;
        return Ok(base);
    }

    '!'.parse_next(input)?;
    ws(input)?;
    let effects = parse_effect_row(input)?;
    let span = base.span().merge(effects.span);
    Ok(TypeExpr::Effect {
        base: Box::new(base),
        effects,
        span,
    })
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

/// Atom-level type: `{`, `(`, atom, ident, true/false, or ExprEscape.
pub(super) fn parse_type_atom(input: &mut &str) -> Result<TypeExpr> {
    ws(input)?;

    if input.starts_with("select")
        && winnow::combinator::peek(kw("select"))
            .parse_next(input)
            .is_ok()
    {
        return parse_type_select(input);
    }
    if input.starts_with('{') {
        return parse_type_braced(input);
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
    starts_braced_type_atom(input)
        || input.starts_with('(')
        || input.starts_with('#')
        || input
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
}

fn starts_braced_type_atom(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('{') else {
        return false;
    };
    // Function and method clause blocks start with `{ | ... }` after a signature.
    // Do not let type-application parsing consume those as record/union type args.
    !rest.trim_start().starts_with('|')
}

fn consume_inline_ws(input: &mut &str) {
    let trimmed = input.trim_start_matches([' ', '\t']);
    *input = trimmed;
}

// ---------------------------------------------------------------------------
// Type braces: record `{ field : TypeExpr; ... }` or
// union `{ #tag; #tag : Payload; ... }`.
// ---------------------------------------------------------------------------

fn parse_type_braced(input: &mut &str) -> Result<TypeExpr> {
    let checkpoint = *input;
    if let Ok(record) = parse_type_record(input) {
        return Ok(record);
    }
    *input = checkpoint;
    parse_type_union(input)
}

// ---------------------------------------------------------------------------
// Type record: `{ field : TypeExpr; ... }` or `{ field? : TypeExpr; ... }`
// ---------------------------------------------------------------------------

fn parse_type_record(input: &mut &str) -> Result<TypeExpr> {
    let ((fields, tail), span) = spanned(parse_type_record_inner).parse_next(input)?;
    Ok(TypeExpr::Record { fields, tail, span })
}

fn parse_type_record_inner(input: &mut &str) -> Result<(Vec<TypeRecordField>, Option<RowTail>)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut fields = vec![];
    let mut tail = None;
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        if input.starts_with("...") {
            if tail.is_some() {
                return fail.parse_next(input);
            }
            let parsed_tail = parse_row_tail(input)?;
            if let RowTail::Named { name, .. } = &parsed_tail
                && fields
                    .iter()
                    .any(|field: &TypeRecordField| field.name == *name)
            {
                return fail.parse_next(input);
            }
            tail = Some(parsed_tail);
            ws(input)?;
            ';'.parse_next(input)?;
            ws(input)?;
            if !input.starts_with('}') {
                return fail.parse_next(input);
            }
            continue;
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
    Ok((fields, tail))
}

// ---------------------------------------------------------------------------
// Type union: `{ #tag; ... }` or `{ #tag : Payload; ... }`
// ---------------------------------------------------------------------------

fn parse_type_union(input: &mut &str) -> Result<TypeExpr> {
    let ((variants, tail), span) = spanned(parse_type_union_inner).parse_next(input)?;
    Ok(TypeExpr::Union {
        variants,
        tail,
        span,
    })
}

fn parse_type_union_inner(input: &mut &str) -> Result<(Vec<UnionVariant>, Option<RowTail>)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut variants = vec![];
    let mut tail = None;
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        if input.starts_with("...") {
            if tail.is_some() {
                return fail.parse_next(input);
            }
            tail = Some(parse_row_tail(input)?);
            ws(input)?;
            ';'.parse_next(input)?;
            continue;
        }
        if !input.starts_with('#') {
            return fail.parse_next(input);
        }

        let (name, name_span) = spanned(parse_atom_name).parse_next(input)?;
        ws(input)?;
        let payload = if input.starts_with(':') && !input.starts_with("::") {
            ':'.parse_next(input)?;
            ws(input)?;
            let payload = if input.starts_with('(') {
                parse_type_union_positional_payload(input)?
            } else {
                parse_type_expr(input)?
            };
            if matches!(&payload, TypeExpr::Record { tail: Some(_), .. }) {
                return fail.parse_next(input);
            }
            Some(Box::new(payload))
        } else {
            None
        };
        ws(input)?;
        let (_, end_span) = spanned(|i: &mut &str| ';'.parse_next(i)).parse_next(input)?;
        let span = name_span.merge(end_span);
        variants.push(UnionVariant {
            name,
            payload,
            span,
        });
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok((variants, tail))
}

// ---------------------------------------------------------------------------
// Type tuple: `()` or `(item, item, ...)`
// A single positional element with no comma — `(T)` — is a grouped/parenthesized
// type and is unwrapped to the inner type directly (per spec). A single *named*
// element `(field : T)` is kept as a 1-element Tuple.
// ---------------------------------------------------------------------------

/// Inner result from parsing the contents of a `(...)` type:
///   - the item list
///   - whether a comma separator was seen (determines group-vs-tuple)
fn parse_type_tuple_inner(input: &mut &str) -> Result<(Vec<TypeTupleItem>, bool)> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;

    if input.starts_with(')') {
        ')'.parse_next(input)?;
        return Ok((vec![], false));
    }

    let first = parse_type_tuple_item(input)?;
    ws(input)?;

    if !input.starts_with(',') {
        // No comma: single-item parens.  Per spec, a single *positional* item
        // in parens is a grouped type (not a tuple).  A single *named* item
        // has no group meaning and is kept as a 1-element tuple.
        ')'.parse_next(input)?;
        return Ok((vec![first], false));
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
    Ok((items, true))
}

fn parse_type_tuple(input: &mut &str) -> Result<TypeExpr> {
    let ((items, comma_seen), span) = spanned(parse_type_tuple_inner).parse_next(input)?;

    // Single positional item with no comma: `(T)` — a grouped/parenthesized
    // type.  Unwrap to the inner type; the parens are pure grouping.
    if !comma_seen && matches!(items.as_slice(), [TypeTupleItem::Positional(_)]) {
        let TypeTupleItem::Positional(inner) = items.into_iter().next().expect("checked above")
        else {
            unreachable!()
        };
        return Ok(inner);
    }

    Ok(TypeExpr::Tuple { items, span })
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

fn parse_type_union_positional_payload(input: &mut &str) -> Result<TypeExpr> {
    let ((items, _comma_seen), span) = spanned(parse_type_tuple_inner).parse_next(input)?;
    Ok(TypeExpr::Tuple { items, span })
}

fn parse_row_tail(input: &mut &str) -> Result<RowTail> {
    let (_, start_span) = spanned("...").parse_next(input)?;
    if let Ok((name, name_span)) = spanned(parse_ident).parse_next(input) {
        return Ok(RowTail::Named {
            name,
            span: start_span.merge(name_span),
        });
    }
    Ok(RowTail::Anonymous { span: start_span })
}

fn parse_type_select(input: &mut &str) -> Result<TypeExpr> {
    let (_, start_span) = spanned(kw("select")).parse_next(input)?;
    ws(input)?;
    let receiver = parse_type_postfix(input)?;
    ws(input)?;
    let fields = parse_select_fields(input)?;
    let span = fields
        .last()
        .map(|field| start_span.merge(field.span))
        .unwrap_or_else(|| start_span.merge(receiver.span()));
    Ok(TypeExpr::Select {
        receiver: Box::new(receiver),
        fields,
        span,
    })
}

fn parse_select_fields(input: &mut &str) -> Result<Vec<SelectField>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut fields = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let (name, name_span) = spanned(parse_field_name).parse_next(input)?;
        ws(input)?;
        ';'.parse_next(input)?;
        fields.push(SelectField {
            name,
            span: name_span,
        });
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok(fields)
}

fn parse_effect_row(input: &mut &str) -> Result<EffectRow> {
    let (ops, span) = spanned(parse_effect_row_inner).parse_next(input)?;
    Ok(EffectRow { ops, span })
}

fn parse_effect_row_inner(input: &mut &str) -> Result<Vec<EffectOp>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut ops = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        ops.push(parse_effect_op(input)?);
        ws(input)?;
        if input.starts_with(',') {
            ','.parse_next(input)?;
        } else if input.starts_with(';') {
            ';'.parse_next(input)?;
        } else if !input.starts_with('}') {
            return fail.parse_next(input);
        }
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok(ops)
}

fn parse_effect_op(input: &mut &str) -> Result<EffectOp> {
    let start = *input;
    let (first, first_span) = spanned(parse_field_name).parse_next(input)?;
    let mut path = vec![first];
    loop {
        let checkpoint = *input;
        ws(input)?;
        if input.starts_with('.') && !input.starts_with("..") {
            '.'.parse_next(input)?;
            ws(input)?;
            path.push(parse_field_name(input)?);
        } else {
            *input = checkpoint;
            break;
        }
    }
    ws(input)?;
    if input.starts_with(':') && !input.starts_with("::") {
        ':'.parse_next(input)?;
        ws(input)?;
        let signature = parse_type_expr(input)?;
        let span = first_span.merge(signature.span());
        return Ok(EffectOp {
            path,
            payload: None,
            signature: Some(Box::new(signature)),
            span,
        });
    }
    if input.starts_with(',') || input.starts_with(';') || input.starts_with('}') {
        return Ok(EffectOp {
            path,
            payload: None,
            signature: None,
            span: first_span,
        });
    }
    let payload = parse_type_postfix(input)?;
    let span = first_span.merge(payload.span());
    if start.len() == input.len() {
        return fail.parse_next(input);
    }
    Ok(EffectOp {
        path,
        payload: Some(Box::new(payload)),
        signature: None,
        span,
    })
}
