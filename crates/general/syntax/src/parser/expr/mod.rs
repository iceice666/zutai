use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;

use crate::ast::{BinOp, Expr, RecordField, TypeExpr};
use crate::span::Span;

use super::lex::{application_ws, at_depth_0, kw, ws};

mod atom;

pub use atom::{parse_atom_expr, parse_clause_block};

use atom::parse_atom_expr_with_options;

#[derive(Clone, Copy)]
pub(super) struct ExprOptions {
    allow_record_update: bool,
}

impl ExprOptions {
    const DEFAULT: Self = Self {
        allow_record_update: true,
    };
    const NO_RECORD_UPDATE: Self = Self {
        allow_record_update: false,
    };
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

pub fn parse_expr(input: &mut &str) -> Result<Expr> {
    parse_expr_with_options(input, ExprOptions::DEFAULT)
}

pub(super) fn parse_expr_with_options(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    ws(input)?;
    parse_pipeline_level(input, options)
}

// ---------------------------------------------------------------------------
// Level 9: pipeline `|>` and `<|`
// ---------------------------------------------------------------------------

fn parse_pipeline_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    use crate::ast::PipelineDir;
    let mut lhs = parse_coalesce_level(input, options)?;

    let mut last_dir: Option<PipelineDir> = None;

    loop {
        let ws_checkpoint = *input;
        ws(input)?;
        let dir = if input.starts_with("|>") {
            Some(PipelineDir::Forward)
        } else if input.starts_with("<|") {
            Some(PipelineDir::Backward)
        } else {
            None
        };

        let Some(dir) = dir else {
            *input = ws_checkpoint;
            break;
        };

        if let Some(last) = last_dir
            && last != dir
        {
            return fail.parse_next(input);
        }

        if dir == PipelineDir::Forward {
            "|>".parse_next(input)?;
        } else {
            "<|".parse_next(input)?;
        }
        ws(input)?;
        let rhs = parse_coalesce_level(input, options)?;
        let span = lhs.span().merge(rhs.span());
        lhs = Expr::Pipeline {
            dir,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };
        last_dir = Some(dir);
    }

    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 8: coalesce `??` (right-assoc)
// ---------------------------------------------------------------------------

fn parse_coalesce_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let lhs = parse_or_level(input, options)?;
    let checkpoint = *input;
    ws(input)?;
    if input.starts_with("??") {
        "??".parse_next(input)?;
        ws(input)?;
        let rhs = parse_coalesce_level(input, options)?;
        let span = lhs.span().merge(rhs.span());
        return Ok(Expr::Binary {
            op: BinOp::Coalesce,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        });
    }
    *input = checkpoint;
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 7: `||` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_or_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let mut lhs = parse_and_level(input, options)?;
    loop {
        let ws_checkpoint = *input;
        ws(input)?;
        if input.starts_with("||") {
            "||".parse_next(input)?;
            ws(input)?;
            let rhs = parse_and_level(input, options)?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        } else {
            *input = ws_checkpoint;
            break;
        }
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 6: `&&` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_and_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let mut lhs = parse_compare_level(input, options)?;
    loop {
        let ws_checkpoint = *input;
        ws(input)?;
        if input.starts_with("&&") {
            "&&".parse_next(input)?;
            ws(input)?;
            let rhs = parse_compare_level(input, options)?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        } else {
            *input = ws_checkpoint;
            break;
        }
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 5: comparison (non-associative)
// ---------------------------------------------------------------------------

fn parse_compare_op(input: &mut &str) -> Result<BinOp> {
    if input.starts_with("==") {
        "==".parse_next(input)?;
        return Ok(BinOp::Eq);
    }
    if input.starts_with("!=") {
        "!=".parse_next(input)?;
        return Ok(BinOp::Ne);
    }
    if input.starts_with("<=") {
        "<=".parse_next(input)?;
        return Ok(BinOp::Le);
    }
    if input.starts_with(">=") {
        ">=".parse_next(input)?;
        return Ok(BinOp::Ge);
    }
    if input.starts_with('<') && !input.starts_with("<|") {
        '<'.parse_next(input)?;
        return Ok(BinOp::Lt);
    }
    if input.starts_with('>') {
        '>'.parse_next(input)?;
        return Ok(BinOp::Gt);
    }
    fail.parse_next(input)
}

fn parse_compare_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let lhs = parse_add_level(input, options)?;

    let checkpoint = *input;
    ws(input)?;
    if let Ok(op_val) = parse_compare_op(input) {
        ws(input)?;
        let rhs = parse_add_level(input, options)?;
        let span = lhs.span().merge(rhs.span());
        let node = Expr::Binary {
            op: op_val,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };

        ws(input)?;
        let checkpoint2 = *input;
        if parse_compare_op(input).is_ok() {
            *input = checkpoint2;
            return fail.parse_next(input);
        }

        return Ok(node);
    }
    *input = checkpoint;
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 4: `+` `-` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_add_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let mut lhs = parse_mul_level(input, options)?;
    loop {
        let ws_checkpoint = *input;
        ws(input)?;
        let op_val = if input.starts_with('+') && !input.starts_with("+=") {
            '+'.parse_next(input)?;
            BinOp::Add
        } else if input.starts_with('-') && !input.starts_with("->") {
            '-'.parse_next(input)?;
            BinOp::Sub
        } else {
            *input = ws_checkpoint;
            break;
        };
        ws(input)?;
        let rhs = parse_mul_level(input, options)?;
        let span = lhs.span().merge(rhs.span());
        lhs = Expr::Binary {
            op: op_val,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 3: `*` `/` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_mul_level(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let mut lhs = parse_application_with_options(input, options)?;
    loop {
        let ws_checkpoint = *input;
        ws(input)?;
        let op_val = if input.starts_with('*') {
            '*'.parse_next(input)?;
            BinOp::Mul
        } else if input.starts_with('/') {
            '/'.parse_next(input)?;
            BinOp::Div
        } else {
            *input = ws_checkpoint;
            break;
        };
        ws(input)?;
        let rhs = parse_application_with_options(input, options)?;
        let span = lhs.span().merge(rhs.span());
        lhs = Expr::Binary {
            op: op_val,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 2: function application (left-assoc juxtaposition)
// ---------------------------------------------------------------------------

fn can_start_atom(input: &str, had_application_ws: bool) -> bool {
    if at_depth_0() && input.starts_with('\n') {
        return false;
    }
    let s = input.trim_start();
    if s.is_empty() {
        return false;
    }
    let c = s.chars().next().unwrap();
    if matches!(c, ';' | ')' | ']' | '}' | ',') {
        return false;
    }
    if c == '-' {
        return had_application_ws && s.chars().nth(1).is_some_and(|next| next.is_ascii_digit());
    }
    if matches!(
        c,
        '=' | ':' | '|' | '&' | '<' | '>' | '+' | '*' | '/' | '?' | '!'
    ) {
        return false;
    }
    for keyword in ["then", "else"] {
        let mut probe = s;
        if kw(keyword).parse_next(&mut probe).is_ok() {
            return false;
        }
    }
    if s.starts_with("if ") || s.starts_with("=>") {
        return false;
    }
    true
}

pub fn parse_application(input: &mut &str) -> Result<Expr> {
    parse_application_with_options(input, ExprOptions::DEFAULT)
}

fn parse_application_with_options(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let mut func = parse_postfix_with_options(input, options)?;

    loop {
        let saved = *input;
        application_ws(input)?;
        let had_application_ws = saved.len() != input.len();
        if can_start_atom(input, had_application_ws) {
            match parse_postfix_with_options(input, options) {
                Ok(arg) => {
                    let span = func.span().merge(arg.span());
                    func = Expr::Apply {
                        func: Box::new(func),
                        arg: Box::new(arg),
                        span,
                    };
                }
                Err(_) => {
                    *input = saved;
                    break;
                }
            }
        } else {
            *input = saved;
            break;
        }
    }

    Ok(func)
}

// ---------------------------------------------------------------------------
// Level 1: postfix `.field`, `?.field`
// ---------------------------------------------------------------------------

pub(super) fn parse_postfix_with_options(input: &mut &str, options: ExprOptions) -> Result<Expr> {
    let mut node = parse_atom_expr_with_options(input, options)?;

    loop {
        let saved = *input;
        ws(input)?;

        if input.starts_with("?.") {
            use super::lex::{parse_field_name, spanned};
            "?.".parse_next(input)?;
            ws(input)?;
            let (field, _) = spanned(parse_field_name).parse_next(input)?;
            let span = node.span();
            node = Expr::OptAccess {
                receiver: Box::new(node),
                field,
                span,
            };
        } else if input.starts_with('.') && !input.starts_with("..") {
            use super::lex::{parse_field_name, spanned};
            '.'.parse_next(input)?;
            ws(input)?;
            let (field, _) = spanned(parse_field_name).parse_next(input)?;
            let span = node.span();
            node = Expr::Access {
                receiver: Box::new(node),
                field,
                span,
            };
        } else if options.allow_record_update && kw("with").parse_next(input).is_ok() {
            ws(input)?;
            let (fields, closing_span) = parse_record_update_fields(input)?;
            let end_span = fields
                .last()
                .map(|field| field.span)
                .unwrap_or(closing_span);
            let span = node.span().merge(end_span);
            node = Expr::RecordUpdate {
                receiver: Box::new(node),
                fields,
                span,
            };
        } else {
            *input = saved;
            break;
        }
    }

    Ok(node)
}

fn parse_record_update_fields(input: &mut &str) -> Result<(Vec<RecordField>, Span)> {
    use super::lex::{enter_delimiter, parse_field_name, spanned};

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
            parse_expr(input)?
        };
        ws(input)?;
        ';'.parse_next(input)?;
        let span = name_span.merge(value.span());
        fields.push(RecordField { name, value, span });
    }
    ws(input)?;
    let (_, closing_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    Ok((fields, closing_span))
}

pub(super) fn fix_number_span(expr: Expr, span: Span) -> Expr {
    match expr {
        Expr::Integer { value, postfix, .. } => Expr::Integer {
            value,
            postfix,
            span,
        },
        Expr::Float { value, postfix, .. } => Expr::Float {
            value,
            postfix,
            span,
        },
        Expr::Posit { literal, .. } => Expr::Posit { literal, span },
        other => other,
    }
}

// ---------------------------------------------------------------------------
// ExprEscape for type context: parse an application as a TypeExpr::ExprEscape
// ---------------------------------------------------------------------------

pub fn parse_application_as_type_escape(input: &mut &str) -> Result<TypeExpr> {
    let expr = parse_application(input)?;
    Ok(TypeExpr::ExprEscape(Box::new(expr)))
}
