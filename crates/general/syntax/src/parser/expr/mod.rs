use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;

use crate::ast::{BinOp, Expr, TypeExpr};
use crate::span::Span;

use super::lex::{application_ws, at_depth_0, ws};

mod atom;

pub use atom::{parse_atom_expr, parse_clause_block};

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

pub fn parse_expr(input: &mut &str) -> Result<Expr> {
    ws(input)?;
    parse_pipeline_level(input)
}

// ---------------------------------------------------------------------------
// Level 9: pipeline `|>` and `<|`
// ---------------------------------------------------------------------------

fn parse_pipeline_level(input: &mut &str) -> Result<Expr> {
    use crate::ast::PipelineDir;
    let mut lhs = parse_coalesce_level(input)?;

    let mut last_dir: Option<PipelineDir> = None;

    loop {
        ws(input)?;
        let dir = if input.starts_with("|>") {
            Some(PipelineDir::Forward)
        } else if input.starts_with("<|") {
            Some(PipelineDir::Backward)
        } else {
            None
        };

        let Some(dir) = dir else { break };

        if let Some(last) = last_dir {
            if last != dir {
                return fail.parse_next(input);
            }
        }

        if dir == PipelineDir::Forward {
            "|>".parse_next(input)?;
        } else {
            "<|".parse_next(input)?;
        }
        ws(input)?;
        let rhs = parse_coalesce_level(input)?;
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

fn parse_coalesce_level(input: &mut &str) -> Result<Expr> {
    let lhs = parse_or_level(input)?;
    ws(input)?;
    if input.starts_with("??") {
        "??".parse_next(input)?;
        ws(input)?;
        let rhs = parse_coalesce_level(input)?;
        let span = lhs.span().merge(rhs.span());
        return Ok(Expr::Binary {
            op: BinOp::Coalesce,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        });
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 7: `||` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_or_level(input: &mut &str) -> Result<Expr> {
    let mut lhs = parse_and_level(input)?;
    loop {
        ws(input)?;
        if input.starts_with("||") {
            "||".parse_next(input)?;
            ws(input)?;
            let rhs = parse_and_level(input)?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        } else {
            break;
        }
    }
    Ok(lhs)
}

// ---------------------------------------------------------------------------
// Level 6: `&&` (left-assoc)
// ---------------------------------------------------------------------------

fn parse_and_level(input: &mut &str) -> Result<Expr> {
    let mut lhs = parse_compare_level(input)?;
    loop {
        ws(input)?;
        if input.starts_with("&&") {
            "&&".parse_next(input)?;
            ws(input)?;
            let rhs = parse_compare_level(input)?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        } else {
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

fn parse_compare_level(input: &mut &str) -> Result<Expr> {
    let lhs = parse_add_level(input)?;
    ws(input)?;

    let checkpoint = *input;
    if let Ok(op_val) = parse_compare_op(input) {
        ws(input)?;
        let rhs = parse_add_level(input)?;
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

fn parse_add_level(input: &mut &str) -> Result<Expr> {
    let mut lhs = parse_mul_level(input)?;
    loop {
        ws(input)?;
        let op_val = if input.starts_with('+') && !input.starts_with("+=") {
            '+'.parse_next(input)?;
            BinOp::Add
        } else if input.starts_with('-') && !input.starts_with("->") {
            '-'.parse_next(input)?;
            BinOp::Sub
        } else {
            break;
        };
        ws(input)?;
        let rhs = parse_mul_level(input)?;
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

fn parse_mul_level(input: &mut &str) -> Result<Expr> {
    let mut lhs = parse_application(input)?;
    loop {
        ws(input)?;
        let op_val = if input.starts_with('*') {
            '*'.parse_next(input)?;
            BinOp::Mul
        } else if input.starts_with('/') {
            '/'.parse_next(input)?;
            BinOp::Div
        } else {
            break;
        };
        ws(input)?;
        let rhs = parse_application(input)?;
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
    for kw in &["then", "else", "if ", "=>"] {
        if s.starts_with(kw) {
            return false;
        }
    }
    true
}

pub fn parse_application(input: &mut &str) -> Result<Expr> {
    let mut func = parse_postfix(input)?;

    loop {
        let saved = *input;
        application_ws(input)?;
        let had_application_ws = saved.len() != input.len();
        if can_start_atom(input, had_application_ws) {
            match parse_postfix(input) {
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

fn parse_postfix(input: &mut &str) -> Result<Expr> {
    let mut node = parse_atom_expr(input)?;

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
        } else {
            *input = saved;
            break;
        }
    }

    Ok(node)
}

pub(super) fn fix_number_span(expr: Expr, span: Span) -> Expr {
    match expr {
        Expr::Integer { value, .. } => Expr::Integer { value, span },
        Expr::Float { value, .. } => Expr::Float { value, span },
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
