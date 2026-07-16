use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;

use crate::ast::{
    EffectOp, EffectRow, Level, RowSpread, RowTail, SelectField, TypeExpr, TypeRecordField,
    TypeTupleItem, UnionVariant,
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
    let base = parse_type_select_op(input)?;
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

// ---------------------------------------------------------------------------
// Type select operator `Type >>= { field; ... }` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_type_select_op(input: &mut &str) -> Result<TypeExpr> {
    let mut lhs = parse_type_application(input)?;
    loop {
        let checkpoint = *input;
        ws(input)?;
        if input.starts_with(">>=") {
            ">>=".parse_next(input)?;
            ws(input)?;
            let fields = parse_select_fields(input)?;
            let span = fields
                .last()
                .map(|field| lhs.span().merge(field.span))
                .unwrap_or_else(|| lhs.span());
            lhs = TypeExpr::Select {
                receiver: Box::new(lhs),
                fields,
                span,
            };
        } else {
            *input = checkpoint;
            break;
        }
    }
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
            let (field, field_span) = spanned(parse_field_name).parse_next(input)?;
            let span = node.span().merge(field_span);
            node = TypeExpr::Access {
                receiver: Box::new(node),
                field,
                span,
            };
        } else if input.starts_with('.') && !input.starts_with("..") {
            '.'.parse_next(input)?;
            ws(input)?;
            let (field, field_span) = spanned(parse_field_name).parse_next(input)?;
            let span = node.span().merge(field_span);
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

    if starts_type_select(input) {
        return parse_type_select(input);
    }
    if input.starts_with('{') {
        return parse_type_braced(input);
    }
    if input.starts_with("(<") {
        let checkpoint = *input;
        if let Ok(forall) = parse_forall_type(input) {
            return Ok(forall);
        }
        *input = checkpoint;
    }
    if input.starts_with('(') {
        return parse_type_tuple(input);
    }
    if input.starts_with('$') {
        return parse_universe_type(input);
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

fn parse_forall_type(input: &mut &str) -> Result<TypeExpr> {
    let ((params, body), span) = spanned(|input: &mut &str| {
        '('.parse_next(input)?;
        let _guard = enter_delimiter();
        ws(input)?;
        let params = super::decl::parse_type_param_list(input)?;
        ws(input)?;
        let body = parse_type_expr(input)?;
        ws(input)?;
        ')'.parse_next(input)?;
        Ok((params, body))
    })
    .parse_next(input)?;
    Ok(TypeExpr::ForAll {
        params,
        body: Box::new(body),
        span,
    })
}

/// `$ℓ` — a universe at an explicit level. `$` then a `LevelArg`
/// (`$0`, `$l`, or `$( Level )`). Bare atoms need no parens; `+`/`max`
/// compounds are parenthesized.
fn parse_universe_type(input: &mut &str) -> Result<TypeExpr> {
    let (level, span) = spanned(|input: &mut &str| {
        '$'.parse_next(input)?;
        parse_level_arg(input)
    })
    .parse_next(input)?;
    Ok(TypeExpr::UniverseType { level, span })
}

/// `LevelArg ::= IntLit | Ident | "(" Level ")"`. Also serves as `LevelAtom`.
fn parse_level_arg(input: &mut &str) -> Result<Level> {
    if input.starts_with('(') {
        '('.parse_next(input)?;
        let _guard = enter_delimiter();
        ws(input)?;
        let level = parse_level(input)?;
        ws(input)?;
        ')'.parse_next(input)?;
        return Ok(level);
    }
    if input.starts_with(|c: char| c.is_ascii_digit()) {
        let (value, span) = spanned(parse_level_int).parse_next(input)?;
        return Ok(Level::Known { value, span });
    }
    let (name, span) = spanned(parse_ident).parse_next(input)?;
    Ok(Level::Var { name, span })
}

/// `Level ::= "max" LevelArg LevelArg | LevelAtom ("+" IntLit)?`. Parsed only
/// inside `$( … )`. `max` is a contextual keyword here; `+` takes an integer
/// literal only (a non-literal addend, e.g. `l + m`, is a parse error).
fn parse_level(input: &mut &str) -> Result<Level> {
    ws(input)?;
    let checkpoint = *input;
    if kw("max").parse_next(input).is_ok() {
        ws(input)?;
        let left = parse_level_arg(input)?;
        ws(input)?;
        let right = parse_level_arg(input)?;
        let span = left.span().merge(right.span());
        return Ok(Level::Max {
            left: Box::new(left),
            right: Box::new(right),
            span,
        });
    }
    *input = checkpoint;

    let base = parse_level_arg(input)?;
    let after_base = *input;
    ws(input)?;
    if input.starts_with('+') {
        '+'.parse_next(input)?;
        ws(input)?;
        let (by, by_span) = spanned(parse_level_int).parse_next(input)?;
        let span = base.span().merge(by_span);
        return Ok(Level::Succ {
            base: Box::new(base),
            by,
            span,
        });
    }
    *input = after_base;
    Ok(base)
}

/// A non-negative integer literal used in level position (`$0`, `+ 2`).
fn parse_level_int(input: &mut &str) -> Result<u32> {
    let digits = winnow::token::take_while(1.., |c: char| c.is_ascii_digit()).parse_next(input)?;
    match digits.parse::<u32>() {
        Ok(value) => Ok(value),
        Err(_) => fail.parse_next(input),
    }
}

fn starts_type_atom(input: &str) -> bool {
    starts_braced_type_atom(input)
        || input.starts_with('(')
        || input.starts_with('#')
        || input.starts_with('$')
        || input
            .chars()
            .next()
            .is_some_and(crate::ident::is_ident_start)
}

fn starts_braced_type_atom(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('{') else {
        return false;
    };
    // Match clause blocks start with `{ | ... }`.
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
    let ((fields, spreads, tail), span) = spanned(parse_type_record_inner).parse_next(input)?;
    Ok(TypeExpr::Record {
        fields,
        spreads,
        tail,
        span,
    })
}

fn parse_type_record_inner(
    input: &mut &str,
) -> Result<(Vec<TypeRecordField>, Vec<RowSpread>, Option<RowTail>)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut fields = vec![];
    let mut spreads = vec![];
    let mut tail = None;
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        if input.starts_with('*') {
            spreads.push(parse_row_spread(input)?);
            ws(input)?;
            ';'.parse_next(input)?;
            continue;
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
    Ok((fields, spreads, tail))
}

// ---------------------------------------------------------------------------
// Type union: `{ #tag; ... }` or `{ #tag : Payload; ... }`
// ---------------------------------------------------------------------------

fn parse_type_union(input: &mut &str) -> Result<TypeExpr> {
    let ((variants, spreads, tail), span) = spanned(parse_type_union_inner).parse_next(input)?;
    Ok(TypeExpr::Union {
        variants,
        spreads,
        tail,
        span,
    })
}

fn parse_type_union_inner(
    input: &mut &str,
) -> Result<(Vec<UnionVariant>, Vec<RowSpread>, Option<RowTail>)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut variants = vec![];
    let mut spreads = vec![];
    let mut tail = None;
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        if input.starts_with('*') {
            spreads.push(parse_row_spread(input)?);
            ws(input)?;
            ';'.parse_next(input)?;
            continue;
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
    Ok((variants, spreads, tail))
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

fn parse_row_spread(input: &mut &str) -> Result<RowSpread> {
    let (_, start_span) = spanned('*').parse_next(input)?;
    ws(input)?;
    let (name, name_span) = spanned(parse_ident).parse_next(input)?;
    let mut path = vec![name];
    let mut span = start_span.merge(name_span);
    loop {
        let checkpoint = *input;
        ws(input)?;
        if input.starts_with('.') && !input.starts_with("..") {
            '.'.parse_next(input)?;
            ws(input)?;
            let (field, field_span) = spanned(parse_field_name).parse_next(input)?;
            path.push(field);
            span = span.merge(field_span);
        } else {
            *input = checkpoint;
            break;
        }
    }
    if path.len() > 1 {
        Ok(RowSpread::Qualified { path, span })
    } else {
        Ok(RowSpread::Named {
            name: path.pop().expect("path contains first segment"),
            span,
        })
    }
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

fn starts_type_select(input: &str) -> bool {
    input.starts_with("select")
        && winnow::combinator::peek(kw("select"))
            .parse_next(&mut &*input)
            .is_ok()
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
    let ((ops, spreads, tail), span) = spanned(parse_effect_row_inner).parse_next(input)?;
    Ok(EffectRow {
        ops,
        spreads,
        tail,
        span,
    })
}

fn parse_effect_row_inner(
    input: &mut &str,
) -> Result<(Vec<EffectOp>, Vec<RowSpread>, Option<RowTail>)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut ops = vec![];
    let mut spreads = vec![];
    let mut tail = None;
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        if input.starts_with('*') {
            let spread = parse_row_spread(input)?;
            ws(input)?;
            if input.starts_with(';') {
                ';'.parse_next(input)?;
            } else if input.starts_with(',') {
                ','.parse_next(input)?;
            } else {
                return fail.parse_next(input);
            }
            spreads.push(spread);
            continue;
        }
        // The final `...e` / `...` row tail is terminal.
        if input.starts_with("...") {
            let parsed_tail = parse_row_tail(input)?;
            ws(input)?;
            if input.starts_with('}') {
                tail = Some(parsed_tail);
                break;
            }
            let had_separator = if input.starts_with(';') {
                ';'.parse_next(input)?;
                true
            } else if input.starts_with(',') {
                ','.parse_next(input)?;
                true
            } else {
                false
            };
            ws(input)?;
            if !had_separator {
                return fail.parse_next(input);
            }
            if input.starts_with('}') {
                tail = Some(parsed_tail);
                break;
            }
            return fail.parse_next(input);
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
    Ok((ops, spreads, tail))
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
