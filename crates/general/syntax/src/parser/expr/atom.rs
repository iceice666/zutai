use winnow::Parser;
use winnow::Result;
use winnow::combinator::{fail, peek};

use crate::ast::{
    Expr, FuncClause, GenStmt, HandleClause, ListItem, LocalBinding, RecordField, RecordItem,
    SelectField, TupleItem, ValueSpread,
};
use crate::span::Span;

use crate::parser::lex::{
    enter_delimiter, kw, parse_atom_name, parse_field_name, parse_ident, parse_import_source,
    parse_number_value, parse_string, parse_value_field_name, parse_value_ident, spanned, ws,
};
use crate::parser::pattern::parse_pattern;
use crate::parser::type_expr::parse_type_expr;
use winnow::token::take_while;

use super::{ExprOptions, fix_number_span};

pub fn parse_atom_expr(input: &mut &str) -> Result<Expr> {
    parse_atom_expr_with_options(input, ExprOptions::DEFAULT)
}

pub(super) fn parse_atom_expr_with_options(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    ws(input)?;

    if input.is_empty() {
        return fail.parse_next(input);
    }
    let first = input.chars().next().unwrap();

    match first {
        '"' => {
            let (s, span) = spanned(parse_string).parse_next(input)?;
            Ok(Expr::String { value: s, span })
        }
        '#' => {
            let (name, atom_span) = spanned(parse_atom_name).parse_next(input)?;
            // Try `#tag { ... }` and `#tag (...)` tagged-value forms. Save a
            // checkpoint so that if `{ ... }` is not a valid record/block (e.g.
            // it is a match clause block `{ | ... }`) we fall back to returning a
            // plain atom.
            let checkpoint = *input;
            take_while(0.., |c: char| c == ' ' || c == '\t').parse_next(input)?;
            if input.starts_with('{') {
                match parse_record_or_list(input, options) {
                    Ok(payload) => {
                        let span = atom_span.merge(payload.span());
                        return Ok(Expr::TaggedValue {
                            tag: name,
                            payload: Box::new(payload),
                            span,
                        });
                    }
                    Err(_) => {
                        *input = checkpoint;
                    }
                }
            } else if input.starts_with('(') {
                match parse_tagged_tuple_payload(input, options) {
                    Ok(payload) => {
                        let span = atom_span.merge(payload.span());
                        return Ok(Expr::TaggedValue {
                            tag: name,
                            payload: Box::new(payload),
                            span,
                        });
                    }
                    Err(_) => {
                        *input = checkpoint;
                    }
                }
            } else {
                *input = checkpoint;
            }
            Ok(Expr::Atom {
                name,
                span: atom_span,
            })
        }
        '\\' => parse_lambda(input, options),
        '(' => parse_tuple_or_group(input, options),
        '[' => parse_block_value(input, options),
        '{' => parse_record_or_list(input, options),
        '$' => {
            // `$ℓ` in value position is the universe-as-a-value, desugared to the
            // same node `type $ℓ` would produce (no `type` keyword needed).
            let ty = parse_type_expr(input)?;
            let span = ty.span();
            Ok(Expr::TypeForm {
                ty: Box::new(ty),
                span,
            })
        }
        '0'..='9' => {
            let (expr, span) = spanned(parse_number_value).parse_next(input)?;
            Ok(fix_number_span(expr, span))
        }
        '-' => {
            if input.len() > 1 {
                let next = input.chars().nth(1).unwrap_or(' ');
                if next.is_ascii_digit() {
                    let (expr, span) = spanned(parse_number_value).parse_next(input)?;
                    return Ok(fix_number_span(expr, span));
                }
            }
            fail.parse_next(input)
        }
        _ => {
            if input.starts_with("true")
                && let Ok((_, span)) = spanned(kw("true")).parse_next(input)
            {
                return Ok(Expr::True(span));
            }
            if input.starts_with("false")
                && let Ok((_, span)) = spanned(kw("false")).parse_next(input)
            {
                return Ok(Expr::False(span));
            }
            if input.starts_with("cond") && peek(kw("cond")).parse_next(input).is_ok() {
                return parse_cond(input, options);
            }
            if input.starts_with("if") && peek(kw("if")).parse_next(input).is_ok() {
                return parse_if(input, options);
            }
            if input.starts_with("match") && peek(kw("match")).parse_next(input).is_ok() {
                return parse_match(input, options);
            }
            if input.starts_with("type") && peek(kw("type")).parse_next(input).is_ok() {
                return parse_type_form(input);
            }
            if input.starts_with("import") && peek(kw("import")).parse_next(input).is_ok() {
                let (source, span) = spanned(|i: &mut &str| {
                    kw("import").parse_next(i)?;
                    ws(i)?;
                    parse_import_source(i)
                })
                .parse_next(input)?;
                return Ok(Expr::Import { source, span });
            }
            if input.starts_with("witness") && peek(kw("witness")).parse_next(input).is_ok() {
                return parse_witness_reflect(input);
            }
            if starts_generator(input) {
                return parse_generator(input, options);
            }
            if starts_select(input) {
                return parse_select(input, options);
            }
            if starts_perform(input) {
                return parse_perform(input, options);
            }
            if input.starts_with("handle") && peek(kw("handle")).parse_next(input).is_ok() {
                return parse_handle(input);
            }
            if starts_resume(input) {
                return parse_resume(input, options);
            }
            let (name, span) = spanned(parse_value_ident).parse_next(input)?;
            Ok(Expr::Ident { name, span })
        }
    }
}

// ---------------------------------------------------------------------------
// Lambda: `\pat+ . body`
// ---------------------------------------------------------------------------

fn parse_lambda(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned('\\').parse_next(input)?;

    let mut params = vec![];
    loop {
        ws(input)?;
        if input.starts_with('.') || input.starts_with("=>") {
            break;
        }
        let pat = parse_pattern(input)?;
        params.push(pat);
        ws(input)?;
        if input.starts_with('.') || input.starts_with("=>") {
            break;
        }
    }

    if params.is_empty() {
        return fail.parse_next(input);
    }

    if input.starts_with("=>") {
        "=>".parse_next(input)?;
    } else {
        '.'.parse_next(input)?;
        if !input.starts_with(|c: char| c.is_whitespace()) {
            return fail.parse_next(input);
        }
    }
    ws(input)?;

    let body = super::parse_expr_with_options(input, options)?;
    let span = start_span.merge(body.span());

    Ok(Expr::Lambda {
        params,
        body: Box::new(body),
        span,
    })
}

// ---------------------------------------------------------------------------
// Record or block: `{ ... }`
// ---------------------------------------------------------------------------

/// `{ ... }` is parallel: either a record (`name = e;` entries) or a list
/// (bare `e;` entries). `{}` is the empty record; `{;}` is the empty list.
fn parse_record_or_list(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (mut expr, span) =
        spanned(|i: &mut &str| parse_record_or_list_inner(i, options)).parse_next(input)?;
    match &mut expr {
        Expr::Record { span: s, .. } | Expr::List { span: s, .. } => *s = span,
        _ => {}
    }
    Ok(expr)
}

fn parse_record_or_list_inner(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    '{'.parse_next(input)?;
    ws(input)?;

    // Bare `{}` is the empty record (the empty list is written `{;}`).
    if input.starts_with('}') {
        '}'.parse_next(input)?;
        return Ok(Expr::Record {
            items: vec![],
            span: Span::new(0, 0),
        });
    }

    let _guard = enter_delimiter();
    let mut record_items = vec![];
    let mut list_items = vec![];
    let mut spread_only = vec![];
    let mut kind = None;
    let mut empty_list_marker = false;
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        // A bare `;` is a separator with no element — this is what distinguishes
        // the empty list `{;}` from the empty record `{}`.
        if input.starts_with(';') {
            if kind == Some(ContainerKind::Record) {
                return fail.parse_next(input);
            }
            empty_list_marker = true;
            kind.get_or_insert(ContainerKind::List);
            ';'.parse_next(input)?;
            continue;
        }
        if input.starts_with('*') {
            let spread = parse_value_spread(input, options)?;
            ws(input)?;
            ';'.parse_next(input)?;
            match kind {
                Some(ContainerKind::Record) => record_items.push(RecordItem::Spread(spread)),
                Some(ContainerKind::List) => list_items.push(ListItem::Spread(spread)),
                None => spread_only.push(spread),
            }
            continue;
        }
        if looks_like_record_field(input) {
            if kind == Some(ContainerKind::List) {
                return fail.parse_next(input);
            }
            if kind.is_none() {
                record_items.extend(spread_only.drain(..).map(RecordItem::Spread));
                kind = Some(ContainerKind::Record);
            }
            record_items.push(RecordItem::Field(parse_record_field(input, options)?));
        } else {
            if kind == Some(ContainerKind::Record) {
                return fail.parse_next(input);
            }
            if kind.is_none() {
                list_items.extend(spread_only.drain(..).map(ListItem::Spread));
                kind = Some(ContainerKind::List);
            }
            let e = super::parse_expr_with_options(input, options)?;
            ws(input)?;
            ';'.parse_next(input)?;
            list_items.push(ListItem::Item(e));
        }
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    let span = Span::new(0, end_span.end as usize);
    match kind {
        Some(ContainerKind::Record) => Ok(Expr::Record {
            items: record_items,
            span,
        }),
        Some(ContainerKind::List) => Ok(Expr::List {
            items: list_items,
            span,
        }),
        None if !spread_only.is_empty() => Ok(Expr::SpreadOnly {
            spreads: spread_only,
            span,
        }),
        None if empty_list_marker => Ok(Expr::List {
            items: vec![],
            span,
        }),
        None => Ok(Expr::Record {
            items: vec![],
            span,
        }),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Record,
    List,
}

fn looks_like_record_field(input: &str) -> bool {
    let mut tmp = input;
    if parse_value_field_name(&mut tmp).is_ok() {
        tmp = tmp.trim_start_matches(|c: char| c.is_whitespace());
        tmp.starts_with('=') && !tmp.starts_with("==")
    } else {
        false
    }
}

fn parse_record_field(input: &mut &str, options: ExprOptions) -> Result<RecordField> {
    let (name, name_span) = spanned(parse_value_field_name).parse_next(input)?;
    ws(input)?;
    if input.starts_with('=') && !input.starts_with("==") {
        '='.parse_next(input)?;
    } else {
        return fail.parse_next(input);
    }
    ws(input)?;
    let value = if input.starts_with(';') {
        // Field-pun shorthand: `name =;` is sugar for `name = name;`.
        Expr::Ident {
            name: name.clone(),
            span: name_span,
        }
    } else {
        super::parse_expr_with_options(input, options)?
    };
    ws(input)?;
    ';'.parse_next(input)?;
    let span = name_span.merge(value.span());
    Ok(RecordField { name, value, span })
}

fn parse_value_spread(input: &mut &str, options: ExprOptions) -> Result<ValueSpread> {
    let (_, star_span) = spanned('*').parse_next(input)?;
    ws(input)?;
    let value = super::parse_expr_with_options(input, options)?;
    let span = star_span.merge(value.span());
    Ok(ValueSpread { value, span })
}

/// `[ ... ]` is a serial do-block: local bindings (`name := e;` /
/// `name : T = e;`) followed by an optional tail expression that is the block's
/// value. An absent tail, or a trailing `;` after the tail, yields `()` (Unit).
fn parse_block_value(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (mut expr, span) =
        spanned(|i: &mut &str| parse_block_inner(i, options)).parse_next(input)?;
    if let Expr::Block { span: s, .. } = &mut expr {
        *s = span;
    }
    Ok(expr)
}

fn parse_block_inner(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    '['.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut bindings = vec![];
    loop {
        ws(input)?;
        if input.starts_with(']') {
            break;
        }
        let binding_kind = {
            let mut tmp = *input;
            if let Ok(_name) = parse_value_ident(&mut tmp) {
                tmp = tmp.trim_start_matches(|c: char| c.is_whitespace());
                if tmp.starts_with(":=") {
                    Some(false)
                } else if tmp.starts_with(':') && !tmp.starts_with("::") {
                    Some(true)
                } else {
                    None
                }
            } else {
                None
            }
        };

        let Some(has_annotation) = binding_kind else {
            break;
        };
        let (name, name_span) = spanned(parse_value_ident).parse_next(input)?;
        ws(input)?;
        let annotation = if has_annotation {
            ':'.parse_next(input)?;
            ws(input)?;
            let ty = parse_type_expr(input)?;
            ws(input)?;
            '='.parse_next(input)?;
            Some(ty)
        } else {
            ":=".parse_next(input)?;
            None
        };
        ws(input)?;
        let value = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        ';'.parse_next(input)?;
        let span = name_span.merge(value.span());
        bindings.push(LocalBinding {
            name,
            annotation,
            value,
            span,
        });
    }

    // Optional result: a `;`-separated tail. A trailing `;` (or no tail at all)
    // discards the value to `()`.
    let mut items = vec![];
    let mut trailing_semi = false;
    ws(input)?;
    if !input.starts_with(']') {
        items.push(super::parse_expr_with_options(input, options)?);
        loop {
            ws(input)?;
            if !input.starts_with(';') {
                break;
            }
            ';'.parse_next(input)?;
            trailing_semi = true;
            ws(input)?;
            if input.starts_with(']') {
                break;
            }
            items.push(super::parse_expr_with_options(input, options)?);
            trailing_semi = false;
        }
    }
    let (_, end_span) = spanned(|i: &mut &str| ']'.parse_next(i)).parse_next(input)?;
    let result = build_block_result(items, trailing_semi, end_span.end as usize);
    Ok(Expr::Block {
        bindings,
        result: Box::new(result),
        span: Span::new(0, end_span.end as usize),
    })
}

/// `()` — the empty tuple, which is Unit.
fn unit_expr(end: usize) -> Expr {
    Expr::Tuple {
        items: vec![],
        span: Span::new(end, end),
    }
}

/// Build the value of a do-block from its tail expressions. A trailing `;`
/// forces the tail for its effects but yields `()`; an empty tail is `()`.
fn build_block_result(mut items: Vec<Expr>, trailing_semi: bool, end: usize) -> Expr {
    if trailing_semi {
        items.push(unit_expr(end));
    }
    match items.len() {
        0 => unit_expr(end),
        1 => items.pop().expect("len checked == 1"),
        _ => {
            let start = items
                .first()
                .map(Expr::span)
                .unwrap_or_else(|| Span::new(end, end));
            let span = start.merge(Span::new(end, end));
            Expr::Sequence { items, span }
        }
    }
}

// ---------------------------------------------------------------------------
// Tuple or group
// ---------------------------------------------------------------------------

fn parse_tuple_or_group(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;

    // Empty tuple `()`.
    if input.starts_with(')') {
        let (_, span) = spanned(|i: &mut &str| ')'.parse_next(i)).parse_next(input)?;
        return Ok(Expr::Tuple {
            items: vec![],
            span,
        });
    }

    // Parse the first item exactly once. A following `,` makes this a tuple; a
    // following `)` makes it a parenthesised group. Deciding from a single
    // lookahead token avoids re-parsing the inner expression: the previous
    // try-tuple-then-fall-back-to-group form parsed it twice per nesting level,
    // which is O(2^n) on inputs like `((((x))))`.
    let first = parse_tuple_item(input, options)?;
    ws(input)?;

    if !input.starts_with(',') {
        // Group: a single parenthesised expression. A bare named item with no
        // trailing comma (`(x = e)`) is neither a group nor a valid tuple.
        return match first {
            TupleItem::Positional(e) => {
                ')'.parse_next(input)?;
                Ok(e)
            }
            TupleItem::Named { .. } => fail.parse_next(input),
        };
    }

    let mut items = vec![first];
    while input.starts_with(',') {
        ','.parse_next(input)?;
        ws(input)?;
        if input.starts_with(')') {
            break;
        }
        items.push(parse_tuple_item(input, options)?);
        ws(input)?;
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| ')'.parse_next(i)).parse_next(input)?;
    let span = Span::new(0, end_span.end as usize);
    Ok(Expr::Tuple { items, span })
}

fn parse_tagged_tuple_payload(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;

    if input.starts_with(')') {
        let (_, span) = spanned(|i: &mut &str| ')'.parse_next(i)).parse_next(input)?;
        return Ok(Expr::Tuple {
            items: vec![],
            span,
        });
    }

    let mut items = vec![parse_tuple_item(input, options)?];
    ws(input)?;
    while input.starts_with(',') {
        ','.parse_next(input)?;
        ws(input)?;
        if input.starts_with(')') {
            break;
        }
        items.push(parse_tuple_item(input, options)?);
        ws(input)?;
    }
    let (_, end_span) = spanned(|i: &mut &str| ')'.parse_next(i)).parse_next(input)?;
    let span = Span::new(0, end_span.end as usize);
    Ok(Expr::Tuple { items, span })
}

fn parse_tuple_item(input: &mut &str, options: ExprOptions) -> Result<TupleItem> {
    let checkpoint = *input;
    if let Ok(name) = parse_value_field_name(input) {
        ws(input)?;
        if input.starts_with('=') && !input.starts_with("==") {
            '='.parse_next(input)?;
            ws(input)?;
            let value = super::parse_expr_with_options(input, options)?;
            let span = value.span();
            return Ok(TupleItem::Named { name, value, span });
        }
    }
    *input = checkpoint;
    let e = super::parse_expr_with_options(input, options)?;
    Ok(TupleItem::Positional(e))
}

// ---------------------------------------------------------------------------
// Generator sugar: `stream { yield expr; ... }`
// ---------------------------------------------------------------------------

fn starts_generator(input: &str) -> bool {
    let mut tmp = input;
    if kw("stream").parse_next(&mut tmp).is_err() || ws(&mut tmp).is_err() {
        return false;
    }
    if !tmp.starts_with('{') {
        return false;
    }
    let mut body = &tmp[1..];
    if ws(&mut body).is_err() {
        return false;
    }
    // `stream` stays contextual: a generator block begins with a `yield`
    // statement (the classic shell) or a guarded `if` (a conditional/recursive
    // generator, V3-G3). Anything else keeps `stream` an ordinary identifier so
    // `stream { field = value; }` remains plain function application; to force
    // application of a conditional, parenthesise: `stream ({ if … })`.
    let mut yield_probe = body;
    if kw("yield").parse_next(&mut yield_probe).is_ok() {
        return ws(&mut yield_probe).is_ok()
            && !matches!(yield_probe.chars().next(), Some('=' | ';' | '}') | None);
    }
    let mut if_probe = body;
    kw("if").parse_next(&mut if_probe).is_ok()
}

fn parse_generator(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("stream")).parse_next(input)?;
    ws(input)?;
    let (body, end_span) = parse_gen_brace_block(input, options)?;
    Ok(Expr::Generator {
        body,
        span: start_span.merge(end_span),
    })
}

/// Parse a `{`-delimited generator statement block, returning its statements and
/// the span of the closing brace.
fn parse_gen_brace_block(input: &mut &str, options: ExprOptions) -> Result<(Vec<GenStmt>, Span)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut stmts = Vec::new();
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        stmts.push(parse_gen_stmt(input, options)?);
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    Ok((stmts, end_span))
}

fn parse_gen_stmt(input: &mut &str, options: ExprOptions) -> Result<GenStmt> {
    // Conditional yield: `if cond then { … } [else { … }]`. The branches are
    // themselves generator-statement blocks (not expressions).
    let mut if_probe = *input;
    if kw("if").parse_next(&mut if_probe).is_ok() {
        let (_, start_span) = spanned(kw("if")).parse_next(input)?;
        ws(input)?;
        let cond = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        kw("then").parse_next(input)?;
        ws(input)?;
        let (then_body, then_end) = parse_gen_brace_block(input, options)?;
        let mut else_body = Vec::new();
        let mut end_span = then_end;
        let mut else_probe = *input;
        let _ = ws(&mut else_probe);
        if kw("else").parse_next(&mut else_probe).is_ok() {
            ws(input)?;
            kw("else").parse_next(input)?;
            ws(input)?;
            let (eb, ee) = parse_gen_brace_block(input, options)?;
            else_body = eb;
            end_span = ee;
        }
        return Ok(GenStmt::If {
            cond,
            then_body,
            else_body,
            span: start_span.merge(end_span),
        });
    }

    // `yield e;` or the delegating `yield from e;`.
    let (_, start_span) = spanned(kw("yield")).parse_next(input)?;
    ws(input)?;
    let mut from_probe = *input;
    if kw("from").parse_next(&mut from_probe).is_ok() {
        kw("from").parse_next(input)?;
        ws(input)?;
        let stream = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        let (_, semi) = spanned(|i: &mut &str| ';'.parse_next(i)).parse_next(input)?;
        return Ok(GenStmt::YieldFrom {
            stream,
            span: start_span.merge(semi),
        });
    }
    let value = super::parse_expr_with_options(input, options)?;
    ws(input)?;
    let (_, semi) = spanned(|i: &mut &str| ';'.parse_next(i)).parse_next(input)?;
    Ok(GenStmt::Yield {
        value,
        span: start_span.merge(semi),
    })
}

// ---------------------------------------------------------------------------
// If
// ---------------------------------------------------------------------------

fn parse_if(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("if")).parse_next(input)?;
    ws(input)?;
    let cond = super::parse_expr_with_options(input, options)?;
    ws(input)?;
    kw("then").parse_next(input)?;
    ws(input)?;
    let then_branch = super::parse_expr_with_options(input, options)?;
    ws(input)?;
    kw("else").parse_next(input)?;
    ws(input)?;
    let else_branch = super::parse_expr_with_options(input, options)?;
    let span = start_span.merge(else_branch.span());
    Ok(Expr::If {
        cond: Box::new(cond),
        then_branch: Box::new(then_branch),
        else_branch: Box::new(else_branch),
        span,
    })
}

// ---------------------------------------------------------------------------
// Cond
// ---------------------------------------------------------------------------

fn parse_cond(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("cond")).parse_next(input)?;
    ws(input)?;
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();

    let mut arms = Vec::new();
    let mut default = None;

    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }

        let checkpoint = *input;
        if input.starts_with('_') {
            '_'.parse_next(input)?;
            ws(input)?;
            if input.starts_with("=>") {
                "=>".parse_next(input)?;
                ws(input)?;
                let body = super::parse_expr_with_options(input, options)?;
                ws(input)?;
                ';'.parse_next(input)?;
                ws(input)?;
                if !input.starts_with('}') {
                    return fail.parse_next(input);
                }
                default = Some(body);
                break;
            }
            *input = checkpoint;
        }

        let cond = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        "=>".parse_next(input)?;
        ws(input)?;
        let body = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        ';'.parse_next(input)?;
        arms.push((cond, body));
    }

    ws(input)?;
    '}'.parse_next(input)?;

    if arms.is_empty() {
        return fail.parse_next(input);
    }
    let Some(mut expr) = default else {
        return fail.parse_next(input);
    };

    for (index, (cond, then_branch)) in arms.into_iter().enumerate().rev() {
        let span = if index == 0 {
            start_span.merge(expr.span())
        } else {
            cond.span().merge(expr.span())
        };
        expr = Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(expr),
            span,
        };
    }

    Ok(expr)
}

// ---------------------------------------------------------------------------
// Match
// ---------------------------------------------------------------------------

fn parse_match(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("match")).parse_next(input)?;
    ws(input)?;
    let scrutinee = super::parse_expr_with_options(input, options)?;
    ws(input)?;
    let arms = parse_clause_block_with_options(input, options)?;
    let end_span = arms.last().map(|a| a.span).unwrap_or(scrutinee.span());
    let span = start_span.merge(end_span);
    Ok(Expr::Match {
        scrutinee: Box::new(scrutinee),
        arms,
        span,
    })
}

pub fn parse_clause_block(input: &mut &str) -> Result<Vec<FuncClause>> {
    parse_clause_block_with_options(input, ExprOptions::DEFAULT)
}

fn parse_clause_block_with_options(
    input: &mut &str,
    options: ExprOptions,
) -> Result<Vec<FuncClause>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut clauses = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        if !input.starts_with('|') {
            return fail.parse_next(input);
        }
        '|'.parse_next(input)?;
        ws(input)?;

        let mut patterns = vec![];
        loop {
            ws(input)?;
            if input.starts_with("if ")
                || input.starts_with("=>")
                || input.starts_with("if\t")
                || input.starts_with("if\n")
            {
                break;
            }
            let pat = parse_pattern(input)?;
            patterns.push(pat);
        }

        ws(input)?;
        let guard =
            if input.starts_with("if ") || input.starts_with("if\t") || input.starts_with("if\n") {
                kw("if").parse_next(input)?;
                ws(input)?;
                Some(super::parse_expr_with_options(input, options)?)
            } else {
                None
            };

        ws(input)?;
        "=>".parse_next(input)?;
        ws(input)?;
        let body = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        ';'.parse_next(input)?;
        let span = patterns
            .first()
            .map(|p| p.span())
            .unwrap_or(body.span())
            .merge(body.span());
        clauses.push(FuncClause {
            patterns,
            guard,
            body,
            span,
        });
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok(clauses)
}

// ---------------------------------------------------------------------------
// Type form: `type TypeExpr`
// ---------------------------------------------------------------------------

fn parse_type_form(input: &mut &str) -> Result<Expr> {
    let (_, start_span) = spanned(kw("type")).parse_next(input)?;
    ws(input)?;
    let ty = parse_type_expr(input)?;
    let span = start_span.merge(ty.span());
    Ok(Expr::TypeForm {
        ty: Box::new(ty),
        span,
    })
}

// ---------------------------------------------------------------------------
// Witness reflection: `witness Constraint @Type`
// ---------------------------------------------------------------------------

fn parse_witness_reflect(input: &mut &str) -> Result<Expr> {
    let (_, start_span) = spanned(kw("witness")).parse_next(input)?;
    ws(input)?;
    let (constraint, _) = spanned(parse_ident).parse_next(input)?;
    ws(input)?;
    '@'.parse_next(input)?;
    ws(input)?;
    let target = parse_type_expr(input)?;
    let span = start_span.merge(target.span());
    Ok(Expr::WitnessReflect {
        constraint,
        target: Box::new(target),
        span,
    })
}

// ---------------------------------------------------------------------------
// V1 frontend forms
// ---------------------------------------------------------------------------

fn parse_select(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("select")).parse_next(input)?;
    ws(input)?;
    let receiver = super::parse_postfix_with_options(input, options)?;
    ws(input)?;
    let fields = parse_select_fields(input)?;
    let span = fields
        .last()
        .map(|field| start_span.merge(field.span))
        .unwrap_or_else(|| start_span.merge(receiver.span()));
    Ok(Expr::Select {
        receiver: Box::new(receiver),
        fields,
        span,
    })
}

fn starts_select(input: &str) -> bool {
    input.starts_with("select") && peek(kw("select")).parse_next(&mut &*input).is_ok()
}

pub(super) fn parse_select_fields(input: &mut &str) -> Result<Vec<SelectField>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut fields = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let (name, name_span) = spanned(parse_value_field_name).parse_next(input)?;
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

fn parse_perform(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(parse_perform_marker).parse_next(input)?;
    ws(input)?;
    let op = parse_effect_path(input)?;
    ws(input)?;
    let arg = super::parse_expr_with_options(input, options)?;
    let span = start_span.merge(arg.span());
    Ok(Expr::Perform {
        op,
        arg: Box::new(arg),
        span,
    })
}

fn starts_perform(input: &str) -> bool {
    input.starts_with('!') && !input.starts_with("!=")
        || input.starts_with("perform") && peek(kw("perform")).parse_next(&mut &*input).is_ok()
}

fn parse_perform_marker(input: &mut &str) -> Result<()> {
    if input.starts_with('!') && !input.starts_with("!=") {
        return '!'.void().parse_next(input);
    }
    kw("perform").parse_next(input)
}

fn parse_handle(input: &mut &str) -> Result<Expr> {
    let (_, start_span) = spanned(kw("handle")).parse_next(input)?;
    ws(input)?;
    let expr = super::parse_expr_with_options(input, ExprOptions::NO_RECORD_UPDATE)?;
    ws(input)?;
    kw("with").parse_next(input)?;
    ws(input)?;
    let clauses = parse_handle_clauses(input)?;
    let span = clauses
        .last()
        .map(|clause| start_span.merge(clause.span))
        .unwrap_or_else(|| start_span.merge(expr.span()));
    Ok(Expr::Handle {
        expr: Box::new(expr),
        clauses,
        span,
    })
}

fn parse_handle_clauses(input: &mut &str) -> Result<Vec<HandleClause>> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut clauses = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let (op, op_span) = spanned(parse_effect_path).parse_next(input)?;
        ws(input)?;
        '='.parse_next(input)?;
        ws(input)?;
        let body = super::parse_expr(input)?;
        ws(input)?;
        let (_, end_span) = spanned(|i: &mut &str| ';'.parse_next(i)).parse_next(input)?;
        clauses.push(HandleClause {
            op,
            body: Box::new(body),
            span: op_span.merge(end_span),
        });
    }
    ws(input)?;
    '}'.parse_next(input)?;
    Ok(clauses)
}

fn parse_resume(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(parse_resume_marker).parse_next(input)?;
    ws(input)?;
    let value = super::parse_expr_with_options(input, options)?;
    let span = start_span.merge(value.span());
    Ok(Expr::Resume {
        value: Box::new(value),
        span,
    })
}

fn starts_resume(input: &str) -> bool {
    input.starts_with('^')
        || input.starts_with("resume") && peek(kw("resume")).parse_next(&mut &*input).is_ok()
}

fn parse_resume_marker(input: &mut &str) -> Result<()> {
    if input.starts_with('^') {
        return '^'.void().parse_next(input);
    }
    kw("resume").parse_next(input)
}

fn parse_effect_path(input: &mut &str) -> Result<Vec<String>> {
    let first = parse_field_name(input)?;
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
    Ok(path)
}
