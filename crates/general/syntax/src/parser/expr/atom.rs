use winnow::Parser;
use winnow::Result;
use winnow::combinator::{fail, peek};

use crate::ast::{
    Expr, FuncClause, HandleClause, LocalBinding, RecordField, SelectField, TupleItem,
};
use crate::span::Span;

use crate::parser::lex::{
    enter_delimiter, kw, parse_atom_name, parse_field_name, parse_ident, parse_number_value,
    parse_string, spanned, ws,
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
                match parse_record_or_block(input, options) {
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
        '[' => parse_list_value(input, options),
        '{' => parse_record_or_block(input, options),
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
            if input.starts_with("if") && peek(kw("if")).parse_next(input).is_ok() {
                return parse_if(input, options);
            }
            if input.starts_with("match") && peek(kw("match")).parse_next(input).is_ok() {
                return parse_match(input, options);
            }
            if input.starts_with("type") && peek(kw("type")).parse_next(input).is_ok() {
                return parse_type_form(input);
            }
            if input.starts_with("witness") && peek(kw("witness")).parse_next(input).is_ok() {
                return parse_witness_reflect(input);
            }
            if starts_generator(input) {
                return parse_generator(input, options);
            }
            if input.starts_with("select") && peek(kw("select")).parse_next(input).is_ok() {
                return parse_select(input, options);
            }
            if input.starts_with("perform") && peek(kw("perform")).parse_next(input).is_ok() {
                return parse_perform(input, options);
            }
            if input.starts_with("handle") && peek(kw("handle")).parse_next(input).is_ok() {
                return parse_handle(input);
            }
            if input.starts_with("resume") && peek(kw("resume")).parse_next(input).is_ok() {
                return parse_resume(input, options);
            }
            let (name, span) = spanned(parse_ident).parse_next(input)?;
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

fn parse_record_or_block(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let start = *input;
    '{'.parse_next(input)?;
    ws(input)?;

    if input.starts_with('}') {
        let (_, span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
        let full_span = Span::new(start.len() - input.len(), span.end as usize);
        return Ok(Expr::Record {
            fields: vec![],
            span: full_span,
        });
    }

    let checkpoint = *input;
    let is_record = {
        let mut tmp = *input;
        if let Ok(_name) = parse_field_name(&mut tmp) {
            tmp = tmp.trim_start_matches(|c: char| c.is_whitespace());
            tmp.starts_with('=') && !tmp.starts_with("==")
        } else {
            false
        }
    };

    if is_record {
        *input = checkpoint;
        parse_record_value_tail(input, start, options)
    } else {
        *input = checkpoint;
        parse_block_expr_tail(input, start, options)
    }
}

fn parse_record_value_tail(input: &mut &str, _start: &str, options: ExprOptions) -> Result<Expr> {
    let _guard = enter_delimiter();
    let mut fields = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let (name, name_span) = spanned(parse_field_name).parse_next(input)?;
        ws(input)?;
        if input.starts_with('=') && !input.starts_with("==") {
            '='.parse_next(input)?;
        } else {
            return fail.parse_next(input);
        }
        ws(input)?;
        let value = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        ';'.parse_next(input)?;
        let span = name_span.merge(value.span());
        fields.push(RecordField { name, value, span });
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    let span = Span::new(0, end_span.end as usize);
    Ok(Expr::Record { fields, span })
}

fn parse_block_expr_tail(input: &mut &str, _start: &str, options: ExprOptions) -> Result<Expr> {
    let _guard = enter_delimiter();
    let mut bindings = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let checkpoint = *input;
        let binding_kind = {
            let mut tmp = *input;
            if let Ok(_name) = parse_ident(&mut tmp) {
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

        if let Some(has_annotation) = binding_kind {
            let (name, name_span) = spanned(parse_ident).parse_next(input)?;
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
        } else {
            *input = checkpoint;
            break;
        }
    }
    ws(input)?;
    let first = super::parse_expr_with_options(input, options)?;
    let mut items = vec![first];
    loop {
        ws(input)?;
        if !input.starts_with(';') {
            break;
        }
        ';'.parse_next(input)?;
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        items.push(super::parse_expr_with_options(input, options)?);
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    let result = if items.len() == 1 {
        items.pop().expect("one item checked above")
    } else {
        let start_span = items.first().map(Expr::span).unwrap_or(end_span);
        Expr::Sequence {
            items,
            span: start_span.merge(end_span),
        }
    };
    let span = result.span().merge(end_span);
    Ok(Expr::Block {
        bindings,
        result: Box::new(result),
        span,
    })
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
    if let Ok(name) = parse_field_name(input) {
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
// List
// ---------------------------------------------------------------------------

fn parse_list_value(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (items, span) = spanned(|i: &mut &str| parse_list_inner(i, options)).parse_next(input)?;
    Ok(Expr::List { items, span })
}

fn parse_list_inner(input: &mut &str, options: ExprOptions) -> Result<Vec<Expr>> {
    '['.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut items = vec![];
    loop {
        ws(input)?;
        if input.starts_with(']') {
            break;
        }
        let e = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        ';'.parse_next(input)?;
        items.push(e);
    }
    ws(input)?;
    ']'.parse_next(input)?;
    Ok(items)
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
    if ws(&mut body).is_err() || kw("yield").parse_next(&mut body).is_err() {
        return false;
    }
    ws(&mut body).is_ok() && !matches!(body.chars().next(), Some('=' | ';' | '}') | None)
}

fn parse_generator(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("stream")).parse_next(input)?;
    ws(input)?;
    let (yields, end_span) = parse_generator_block(input, options)?;
    Ok(Expr::Generator {
        yields,
        span: start_span.merge(end_span),
    })
}

fn parse_generator_block(input: &mut &str, options: ExprOptions) -> Result<(Vec<Expr>, Span)> {
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut yields = Vec::new();
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        kw("yield").parse_next(input)?;
        ws(input)?;
        let value = super::parse_expr_with_options(input, options)?;
        ws(input)?;
        ';'.parse_next(input)?;
        yields.push(value);
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    Ok((yields, end_span))
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

fn parse_perform(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let (_, start_span) = spanned(kw("perform")).parse_next(input)?;
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
    let (_, start_span) = spanned(kw("resume")).parse_next(input)?;
    ws(input)?;
    let value = super::parse_expr_with_options(input, options)?;
    let span = start_span.merge(value.span());
    Ok(Expr::Resume {
        value: Box::new(value),
        span,
    })
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
