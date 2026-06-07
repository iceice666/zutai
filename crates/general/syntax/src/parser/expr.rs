use winnow::Parser;
use winnow::Result;
use winnow::combinator::{fail, peek};

use crate::ast::{
    BinOp, Expr, FuncClause, LocalBinding, PipelineDir, RecordField, TupleItem, TypeExpr,
};
use crate::span::Span;

use super::lex::{
    application_ws, at_depth_0, enter_delimiter, kw, parse_atom_name, parse_field_name,
    parse_ident, parse_import_source, parse_number_value, parse_string, spanned, ws,
};
use super::pattern::parse_pattern;
use super::type_expr::parse_type_expr;

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

        // Mixed pipeline rejection
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
        let rhs = parse_coalesce_level(input)?; // right-recursive
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

        // Non-associative: another comparison at this level is an error.
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
        // `-` not followed by digit (that would be a negative number literal)
        let op_val = if input.starts_with('+') && !input.starts_with("+=") {
            '+'.parse_next(input)?;
            BinOp::Add
        } else if input.starts_with('-') && !input.starts_with("->") {
            // make sure it's a binary minus, not a negative number literal
            // A unary minus only appears as part of a number literal.
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
    // At depth 0, a leading newline terminates an expression.
    if at_depth_0() && input.starts_with('\n') {
        return false;
    }
    // Trim to the first non-whitespace character (but after the depth-0 newline check).
    let s = input.trim_start();
    if s.is_empty() {
        return false;
    }
    let c = s.chars().next().unwrap();
    // Stop tokens that end an application sequence
    if matches!(c, ';' | ')' | ']' | '}' | ',') {
        return false;
    }
    if c == '-' {
        return had_application_ws && s.chars().nth(1).is_some_and(|next| next.is_ascii_digit());
    }
    // Operators / punctuation that are not atom starts
    if matches!(
        c,
        '=' | ':' | '|' | '&' | '<' | '>' | '+' | '*' | '/' | '?' | '!'
    ) {
        return false;
    }
    // Keywords that terminate application context
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
        // At depth 0, only inline ws — newlines terminate the expression.
        // Inside delimiters, full ws (including newlines) is OK.
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
        // Do NOT consume ws here before checking — field access is tight.
        // Actually per spec field access `x.y` does not require spaces.
        // But `x . y` should also work (whitespace is insignificant outside strings).
        let saved = *input;
        ws(input)?;

        if input.starts_with("?.") {
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

// ---------------------------------------------------------------------------
// Atom-level expressions
// ---------------------------------------------------------------------------

pub fn parse_atom_expr(input: &mut &str) -> Result<Expr> {
    ws(input)?;

    // Lookahead to decide branch (avoids backtracking-induced double-parsing)
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
            let (name, span) = spanned(parse_atom_name).parse_next(input)?;
            Ok(Expr::Atom { name, span })
        }
        '\\' => parse_lambda(input),
        '(' => parse_tuple_or_group(input),
        '[' => parse_list_value(input),
        '{' => parse_record_or_block(input),
        '0'..='9' => {
            let (expr, span) = spanned(parse_number_value).parse_next(input)?;
            Ok(fix_number_span(expr, span))
        }
        '-' => {
            // negative number literal
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
            // Keywords (longest match first)
            if input.starts_with("true") {
                if let Ok((_, span)) = spanned(kw("true")).parse_next(input) {
                    return Ok(Expr::True(span));
                }
            }
            if input.starts_with("false") {
                if let Ok((_, span)) = spanned(kw("false")).parse_next(input) {
                    return Ok(Expr::False(span));
                }
            }
            if input.starts_with("if") {
                if let Ok(_) = peek(kw("if")).parse_next(input) {
                    return parse_if(input);
                }
            }
            if input.starts_with("match") {
                if let Ok(_) = peek(kw("match")).parse_next(input) {
                    return parse_match(input);
                }
            }
            if input.starts_with("import") {
                if let Ok(_) = peek(kw("import")).parse_next(input) {
                    return parse_import(input);
                }
            }
            if input.starts_with("type") {
                if let Ok(_) = peek(kw("type")).parse_next(input) {
                    return parse_type_form(input);
                }
            }
            // Identifier
            let (name, span) = spanned(parse_ident).parse_next(input)?;
            Ok(Expr::Ident { name, span })
        }
    }
}

fn fix_number_span(expr: Expr, span: Span) -> Expr {
    match expr {
        Expr::Integer { value, .. } => Expr::Integer { value, span },
        Expr::Float { value, .. } => Expr::Float { value, span },
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Lambda: `\pat+ . body`  (lambda-dot rule: whitespace required around `.`)
// ---------------------------------------------------------------------------

fn parse_lambda(input: &mut &str) -> Result<Expr> {
    let _start_len = input.len();
    let (_, start_span) = spanned('\\').parse_next(input)?;

    let mut params = vec![];
    loop {
        ws(input)?;
        // Stop when we see `.` — that's the lambda dot
        if input.starts_with('.') {
            break;
        }
        let pat = parse_pattern(input)?;
        params.push(pat);
        // Require at least one inline whitespace (or end of params → `.`)
        // Peek: if next non-trivial char is `.` we're done
        ws(input)?;
        if input.starts_with('.') {
            break;
        }
    }

    if params.is_empty() {
        return fail.parse_next(input);
    }

    // Enforce: `.` must be preceded by whitespace (already consumed above)
    // Enforce: whitespace after `.` before body
    '.'.parse_next(input)?;
    // Require whitespace after `.`
    if !input.starts_with(|c: char| c.is_whitespace()) {
        return fail.parse_next(input);
    }
    ws(input)?;

    let body = parse_expr(input)?;
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

fn parse_record_or_block(input: &mut &str) -> Result<Expr> {
    let start = *input;
    '{'.parse_next(input)?;
    ws(input)?;

    if input.starts_with('}') {
        // Empty record
        let (_, span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
        let full_span = Span::new(start.len() - input.len(), span.end as usize);
        return Ok(Expr::Record {
            fields: vec![],
            span: full_span,
        });
    }

    // Peek: `field_name '='` (not `==`) → value record
    // otherwise → block expression
    let checkpoint = *input;
    let is_record = {
        let mut tmp = *input;
        let looks_like_record = if let Ok(_name) = parse_field_name(&mut tmp) {
            // skip ws
            while tmp.starts_with(|c: char| c.is_whitespace()) {
                tmp = &tmp[1..];
            }
            tmp.starts_with('=') && !tmp.starts_with("==")
        } else {
            false
        };
        looks_like_record
    };

    if is_record {
        *input = checkpoint;
        parse_record_value_tail(input, start)
    } else {
        *input = checkpoint;
        parse_block_expr_tail(input, start)
    }
}

fn parse_record_value_tail(input: &mut &str, _start: &str) -> Result<Expr> {
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
        let value = parse_expr(input)?;
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

fn parse_block_expr_tail(input: &mut &str, _start: &str) -> Result<Expr> {
    let _guard = enter_delimiter();
    let mut bindings = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        // Check for local binding: `ident ':=' expr ';'`
        let checkpoint = *input;
        let is_binding = {
            let mut tmp = *input;
            let ok = if let Ok(_name) = parse_ident(&mut tmp) {
                while tmp.starts_with(|c: char| c.is_whitespace()) {
                    tmp = &tmp[1..];
                }
                tmp.starts_with(":=")
            } else {
                false
            };
            ok
        };

        if is_binding {
            let (name, name_span) = spanned(parse_ident).parse_next(input)?;
            ws(input)?;
            ":=".parse_next(input)?;
            ws(input)?;
            let value = parse_expr(input)?;
            ws(input)?;
            ';'.parse_next(input)?;
            let span = name_span.merge(value.span());
            bindings.push(LocalBinding { name, value, span });
        } else {
            *input = checkpoint;
            break;
        }
    }
    ws(input)?;
    let result = parse_expr(input)?;
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
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

fn parse_tuple_or_group(input: &mut &str) -> Result<Expr> {
    let checkpoint = *input;

    // Try tuple
    match parse_tuple(input) {
        Ok(e) => return Ok(e),
        Err(_) => *input = checkpoint,
    }

    parse_group(input)
}

fn parse_tuple(input: &mut &str) -> Result<Expr> {
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

    let first = parse_tuple_item(input)?;
    ws(input)?;

    if !input.starts_with(',') {
        return fail.parse_next(input);
    }

    let mut items = vec![first];
    while input.starts_with(',') {
        ','.parse_next(input)?;
        ws(input)?;
        if input.starts_with(')') {
            break;
        }
        items.push(parse_tuple_item(input)?);
        ws(input)?;
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| ')'.parse_next(i)).parse_next(input)?;
    let span = Span::new(0, end_span.end as usize); // approximate
    Ok(Expr::Tuple { items, span })
}

fn parse_tuple_item(input: &mut &str) -> Result<TupleItem> {
    let checkpoint = *input;
    // Named: `field_name '=' expr`
    if let Ok(name) = parse_field_name(input) {
        ws(input)?;
        if input.starts_with('=') && !input.starts_with("==") {
            '='.parse_next(input)?;
            ws(input)?;
            let value = parse_expr(input)?;
            let span = value.span();
            return Ok(TupleItem::Named { name, value, span });
        }
    }
    *input = checkpoint;
    let e = parse_expr(input)?;
    Ok(TupleItem::Positional(e))
}

fn parse_group(input: &mut &str) -> Result<Expr> {
    '('.parse_next(input)?;
    let _guard = enter_delimiter();
    ws(input)?;
    let e = parse_expr(input)?;
    ws(input)?;
    ')'.parse_next(input)?;
    Ok(e)
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

fn parse_list_value(input: &mut &str) -> Result<Expr> {
    let (items, span) = spanned(parse_list_inner).parse_next(input)?;
    Ok(Expr::List { items, span })
}

fn parse_list_inner(input: &mut &str) -> Result<Vec<Expr>> {
    '['.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut items = vec![];
    loop {
        ws(input)?;
        if input.starts_with(']') {
            break;
        }
        let e = parse_expr(input)?;
        ws(input)?;
        ';'.parse_next(input)?;
        items.push(e);
    }
    ws(input)?;
    ']'.parse_next(input)?;
    Ok(items)
}

// ---------------------------------------------------------------------------
// If
// ---------------------------------------------------------------------------

fn parse_if(input: &mut &str) -> Result<Expr> {
    let (_, start_span) = spanned(kw("if")).parse_next(input)?;
    ws(input)?;
    let cond = parse_expr(input)?;
    ws(input)?;
    kw("then").parse_next(input)?;
    ws(input)?;
    let then_branch = parse_expr(input)?;
    ws(input)?;
    kw("else").parse_next(input)?;
    ws(input)?;
    let else_branch = parse_expr(input)?;
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

fn parse_match(input: &mut &str) -> Result<Expr> {
    let (_, start_span) = spanned(kw("match")).parse_next(input)?;
    ws(input)?;
    let scrutinee = parse_expr(input)?;
    ws(input)?;
    let arms = parse_clause_block(input)?;
    let end_span = arms.last().map(|a| a.span).unwrap_or(scrutinee.span());
    let span = start_span.merge(end_span);
    Ok(Expr::Match {
        scrutinee: Box::new(scrutinee),
        arms,
        span,
    })
}

/// Parse `{ | pat+ (if guard)? => body; ... }`. Shared with M2 function bodies.
pub fn parse_clause_block(input: &mut &str) -> Result<Vec<FuncClause>> {
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

        // Parse one or more patterns until `if` or `=>`
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
                Some(parse_expr(input)?)
            } else {
                None
            };

        ws(input)?;
        "=>".parse_next(input)?;
        ws(input)?;
        let body = parse_expr(input)?;
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
// Import
// ---------------------------------------------------------------------------

fn parse_import(input: &mut &str) -> Result<Expr> {
    let (_, start_span) = spanned(kw("import")).parse_next(input)?;
    ws(input)?;
    let (source, src_span) = spanned(parse_import_source).parse_next(input)?;
    let span = start_span.merge(src_span);
    Ok(Expr::Import { source, span })
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
// ExprEscape for type context: parse an application as a TypeExpr::ExprEscape
// ---------------------------------------------------------------------------

pub fn parse_application_as_type_escape(input: &mut &str) -> Result<TypeExpr> {
    let expr = parse_application(input)?;
    Ok(TypeExpr::ExprEscape(Box::new(expr)))
}
