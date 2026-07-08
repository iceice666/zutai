use winnow::Parser;
use winnow::Result;
use winnow::combinator::fail;
use winnow::token::take_till;

use crate::ast::{
    ConstraintMethod, Decl, DeriveRecipe, Expr, File, FuncClause, ImportSource, MethodName,
    SelectField, TypeParam, TypeParamBound, UseItem, WitnessBody, WitnessField,
};
use crate::span::Span;

use super::expr::parse_expr;
use super::lex::{
    enter_delimiter, kw, parse_field_name, parse_ident, parse_value_field_name, parse_value_ident,
    spanned, ws,
};
use super::pattern::parse_pattern;
use super::type_expr::{parse_type_atom, parse_type_expr};

/// Top-level parse entry: `decl* final_expr?`.
///
/// Top-level declarations are parallel (letrec) and each ends in `;`. The file's
/// value is an optional trailing expression; an absent tail, or a tail followed
/// by `;`, yields `()` (Unit).
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
    let unit_at = decls.last().map(|d| d.span().end).unwrap_or(0) as usize;
    let final_expr = if input.is_empty() {
        // No trailing expression — the file's value is `()`.
        unit_expr(unit_at)
    } else {
        let expr = parse_expr(input)?;
        ws(input)?;
        if input.starts_with(';') {
            // `expr;` discards the value to `()`, forcing `expr` for its effects.
            ';'.parse_next(input)?;
            ws(input)?;
            let span = expr.span();
            Expr::Sequence {
                items: vec![expr, unit_expr(span.end as usize)],
                span,
            }
        } else {
            expr
        }
    };
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

/// `()` — the empty tuple, which is Unit.
fn unit_expr(at: usize) -> Expr {
    Expr::Tuple {
        items: vec![],
        span: Span::new(at, at),
    }
}

/// Consume the mandatory `;` that terminates a value-like top-level declaration.
fn require_term(input: &mut &str) -> Result<()> {
    ws(input)?;
    ';'.parse_next(input)?;
    Ok(())
}

/// Parse a declaration's right-hand-side expression at delimiter depth, so the
/// value may span multiple lines and is terminated by the declaration's `;`.
fn parse_decl_value(input: &mut &str) -> Result<Expr> {
    let _guard = enter_delimiter();
    parse_expr(input)
}

/// Cheap peek: does the current position look like the start of a top-level decl?
fn is_decl_start(input: &mut &str) -> bool {
    let mut tmp = *input;
    // Skip whitespace
    tmp = tmp.trim_start_matches(|c: char| c.is_whitespace());
    if kw("use").parse_next(&mut tmp).is_ok() {
        return true;
    }
    // Destructuring binding: `{ a; b; } ::=`. Scan the balanced brace group and
    // require `::=` after it — a trailing record/list final-expression has no
    // `::=`, so this stays disjoint from the file's value expression.
    if tmp.starts_with('{') {
        return brace_group_then_assign(tmp);
    }
    // Must start with an identifier
    if !tmp.starts_with(crate::ident::is_ident_start) {
        return false;
    }
    // Consume value identifier body (`'` is value-name-only).
    tmp = tmp.trim_start_matches(|c: char| crate::ident::is_ident_continue(c) || c == '\'');
    if tmp.starts_with('?') && !tmp.starts_with("?.") && !tmp.starts_with("??") {
        tmp = &tmp[1..];
    }
    // Skip ws
    tmp = tmp.trim_start_matches(|c: char| c.is_whitespace());
    // Must be followed by `::`, `@` (witness), or a pattern-then-`=` sequence.
    tmp.starts_with("::") || tmp.starts_with('@') || is_nosig_fn_start(tmp)
}

/// Peek whether `s` (starting at `{`) is a balanced brace group followed by
/// `::=`. Field lists contain only `name;` items (no strings or nested braces of
/// their own), so a byte-level brace scan is sufficient to find the close.
fn brace_group_then_assign(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else { return false };
    s[end..]
        .trim_start_matches(|c: char| c.is_whitespace())
        .starts_with("::=")
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
    if s.starts_with(crate::ident::is_ident_start) {
        // Ensure it's not a keyword-only expression starter
        return true;
    }
    false
}

pub fn parse_top_decl(input: &mut &str) -> Result<Decl> {
    ws(input)?;

    let mut use_probe = *input;
    if kw("use").parse_next(&mut use_probe).is_ok() {
        return parse_use_decl(input);
    }

    // Destructuring binding: `{ a; b; } ::= value;`.
    if input.starts_with('{') {
        return parse_destructure_binding(input);
    }

    let (name, name_span) = spanned(parse_value_ident).parse_next(input)?;
    ws(input)?;

    let value_suffix_question = name.ends_with('?');

    // Witness: `Constraint @Target :: ...`
    if input.starts_with('@') {
        if value_suffix_question {
            return fail.parse_next(input);
        }
        return parse_witness(input, name, name_span);
    }

    if input.starts_with("::=") {
        // Inferred top-level binding
        "::=".parse_next(input)?;
        ws(input)?;
        let value = parse_decl_value(input)?;
        let span = name_span.merge(value.span());
        require_term(input)?;
        return Ok(Decl::Inferred { name, value, span });
    }

    if input.starts_with("::") {
        // Typed / TypeAlias / Function / Constraint
        "::".parse_next(input)?;
        ws(input)?;
        return parse_top_decl_after_sig(input, name, name_span, value_suffix_question);
    }

    // No-sig fn: one or more patterns followed by `=`
    parse_no_sig_fn(input, name, name_span)
}

/// `{ a; b; c } ::= value;` — a selective destructuring binding. The field list
/// reuses the select-field syntax (`name;` items); `value` is any record-valued
/// expression (commonly an imported module name).
fn parse_destructure_binding(input: &mut &str) -> Result<Decl> {
    let (fields, fields_span) = spanned(parse_destructure_fields).parse_next(input)?;
    ws(input)?;
    "::=".parse_next(input)?;
    ws(input)?;
    let value = parse_decl_value(input)?;
    let span = fields_span.merge(value.span());
    require_term(input)?;
    Ok(Decl::Destructure {
        fields,
        value,
        span,
    })
}

/// `use stdlib { num as n; text as t; }` — grouped static import sugar.
/// Each item expands during HIR lowering to one ordinary inferred import binding.
fn parse_use_decl(input: &mut &str) -> Result<Decl> {
    let (_, start_span) = spanned(kw("use")).parse_next(input)?;
    ws(input)?;
    let base = parse_use_path(input)?;
    ws(input)?;
    '{'.parse_next(input)?;
    let _guard = enter_delimiter();
    let mut items = vec![];
    loop {
        ws(input)?;
        if input.starts_with('}') {
            break;
        }
        let (member, member_span) = spanned(parse_field_name).parse_next(input)?;
        ws(input)?;
        let alias = if kw("as").parse_next(input).is_ok() {
            ws(input)?;
            parse_value_ident(input)?
        } else {
            member.clone()
        };
        ws(input)?;
        ';'.parse_next(input)?;
        let mut path = base.clone();
        path.push(member);
        items.push(UseItem {
            source: ImportSource::Path(path),
            alias,
            span: member_span,
        });
    }
    ws(input)?;
    let (_, end_span) = spanned(|i: &mut &str| '}'.parse_next(i)).parse_next(input)?;
    ws(input)?;
    if input.starts_with(';') {
        ';'.parse_next(input)?;
    }
    Ok(Decl::Use {
        items,
        span: start_span.merge(end_span),
    })
}

fn parse_use_path(input: &mut &str) -> Result<Vec<String>> {
    let mut parts = vec![parse_field_name(input)?];
    loop {
        if !input.starts_with('.') {
            break;
        }
        '.'.parse_next(input)?;
        parts.push(parse_field_name(input)?);
    }
    Ok(parts)
}

fn parse_destructure_fields(input: &mut &str) -> Result<Vec<SelectField>> {
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

fn parse_top_decl_after_sig(
    input: &mut &str,
    name: String,
    name_span: Span,
    value_suffix_question: bool,
) -> Result<Decl> {
    // Optional type-param list `<A, B, ...>` — only legal here
    let params = if input.starts_with('<') {
        parse_type_param_list(input)?
    } else {
        vec![]
    };
    ws(input)?;

    // Constraint def: `Name :: [<params>] @Target { ... }`
    if input.starts_with('@') {
        if value_suffix_question {
            return fail.parse_next(input);
        }
        return parse_constraint_body(input, name, name_span, params);
    }

    // Type alias: `:: [<params>] type TypeExpr`
    if input.starts_with("type") && kw("type").parse_next(input).is_ok() {
        if value_suffix_question {
            return fail.parse_next(input);
        }
        ws(input)?;
        let ty = parse_type_expr(input)?;
        let span = name_span.merge(ty.span());
        require_term(input)?;
        return Ok(Decl::TypeAlias {
            name,
            params,
            ty,
            span,
        });
    }

    // Typed binding or function: parse TypeExpr, then peek `=`.
    let sig = parse_type_expr(input)?;
    ws(input)?;

    // Function with equals-prefixed clauses:
    //
    //     name :: A -> B
    //       = pat => body;
    //
    // Try this before typed value parsing because both forms start with `=`.
    // If the text after `=` is not a pattern list followed by `=>`, restore
    // and parse it as `name :: Type = expr`.
    let function_checkpoint = *input;
    if let Ok(clauses) = parse_function_clauses(input) {
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
    *input = function_checkpoint;

    if input.starts_with('=') && !input.starts_with("==") {
        // Typed value binding
        if !params.is_empty() {
            return fail.parse_next(input); // params not valid for typed binding
        }
        '='.parse_next(input)?;
        ws(input)?;
        let value = parse_decl_value(input)?;
        let span = name_span.merge(value.span());
        require_term(input)?;
        return Ok(Decl::Typed {
            name,
            ty: sig,
            value,
            span,
        });
    }

    fail.parse_next(input)
}

pub(super) fn parse_type_param_list(input: &mut &str) -> Result<Vec<TypeParam>> {
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
    // Universe-level binder: `$l`. Carries no bounds or kind annotation.
    if input.starts_with('$') {
        let (name, span) = spanned(parse_level_binder).parse_next(input)?;
        return Ok(TypeParam {
            name,
            is_level: true,
            bounds: vec![],
            kind: None,
            span,
        });
    }
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
            is_level: false,
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
            is_level: false,
            bounds,
            kind: None,
            span: span.merge(last_span),
        });
    }
    Ok(TypeParam {
        name,
        is_level: false,
        bounds: vec![],
        kind: None,
        span,
    })
}

/// Parse a level binder name `$l` (sigil stripped). A `$`-prefixed binder may
/// not carry `:` bounds or `::` kind annotations.
fn parse_level_binder(input: &mut &str) -> Result<String> {
    '$'.parse_next(input)?;
    parse_ident(input)
}
fn parse_function_clauses(input: &mut &str) -> Result<Vec<FuncClause>> {
    let mut clauses = vec![];
    loop {
        ws(input)?;
        if !input.starts_with('=') || input.starts_with("==") {
            break;
        }
        clauses.push(parse_function_clause(input)?);
    }
    if clauses.is_empty() {
        return fail.parse_next(input);
    }
    Ok(clauses)
}

fn parse_function_clause(input: &mut &str) -> Result<FuncClause> {
    '='.parse_next(input)?;
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
    if patterns.is_empty() {
        return fail.parse_next(input);
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
    if input.starts_with(';') {
        ';'.parse_next(input)?;
    }
    let span = patterns
        .first()
        .map(|p| p.span())
        .unwrap_or(body.span())
        .merge(body.span());
    Ok(FuncClause {
        patterns,
        guard,
        body,
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
    let body = parse_decl_value(input)?;
    let span = name_span.merge(body.span());
    require_term(input)?;
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
    let (derivable, recipe) = parse_derive_recipe(input)?;
    let span = recipe
        .as_ref()
        .map(|recipe| name_span.merge(recipe.span))
        .unwrap_or_else(|| name_span.merge(close_span));
    Ok(Decl::Constraint {
        name,
        params,
        target,
        methods,
        derivable,
        recipe,
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
    // Optional default clauses.
    let default = if input.starts_with('=') && !input.starts_with("==") {
        parse_function_clauses(input)?
    } else {
        vec![]
    };
    ws(input)?;
    if default.is_empty() || input.starts_with(';') {
        ';'.parse_next(input)?;
    }
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

fn parse_derive_recipe(input: &mut &str) -> Result<(bool, Option<DeriveRecipe>)> {
    if !consume_contextual_derive(input) {
        return Ok((false, None));
    }
    ws(input)?;
    if !input.starts_with('=') || input.starts_with("==") {
        return Ok((true, None));
    }
    '='.parse_next(input)?;
    ws(input)?;
    let params = if input.starts_with('<') {
        parse_type_param_list(input)?
    } else {
        Vec::new()
    };
    ws(input)?;
    "=>".parse_next(input)?;
    ws(input)?;
    let body = parse_expr(input)?;
    let span = body.span();
    Ok((true, Some(DeriveRecipe { params, body, span })))
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
        .is_some_and(crate::ident::is_ident_continue)
    {
        return false;
    }
    let leading_ws = input.len() - trimmed.len();
    *input = &input[leading_ws + "derive".len()..];
    true
}
