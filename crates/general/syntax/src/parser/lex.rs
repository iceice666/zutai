use winnow::Parser;
use winnow::Result;
use winnow::ascii::digit1;
use winnow::combinator::{alt, fail, not, opt, peek, preceded, repeat};
use winnow::token::{one_of, take, take_till, take_while};

use std::cell::Cell;

use crate::ast::{Expr, ImportSource};
use crate::numlit::{NumberType, PostfixCheck, classify_postfix};
use crate::posit::parse_posit_literal;
use crate::span::Span;

thread_local! {
    /// Base pointer of the original input string; set once per `parse()` call.
    pub(crate) static BASE_PTR: Cell<usize> = const { Cell::new(0) };
    /// Delimiter depth: 0 = top level; >0 = inside `{}` / `[]` / `()`.
    pub(crate) static DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Furthest byte offset any parse branch advanced to during the current
    /// `parse()` call. Backtracking rewinds the winnow cursor, so this
    /// high-water mark is the best estimate of where parsing actually got
    /// stuck when the top-level parse fails.
    static FURTHEST: Cell<usize> = const { Cell::new(0) };
}

/// Record that a parse branch successfully advanced to `off`; the stored value
/// only ever increases.
fn bump_furthest(off: usize) {
    FURTHEST.with(|c| {
        if off > c.get() {
            c.set(off);
        }
    });
}

/// The furthest byte offset reached during the current parse.
pub(crate) fn furthest_offset() -> usize {
    FURTHEST.with(|c| c.get())
}

/// Reset the high-water mark; called once at the start of each parse.
pub(crate) fn reset_furthest() {
    FURTHEST.with(|c| c.set(0));
}

/// RAII guard that decrements the delimiter depth on drop.
pub struct DepthGuard;
impl Drop for DepthGuard {
    fn drop(&mut self) {
        DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

/// Increment delimiter depth and return a guard that decrements on drop.
pub fn enter_delimiter() -> DepthGuard {
    DEPTH.with(|c| c.set(c.get() + 1));
    DepthGuard
}

pub fn at_depth_0() -> bool {
    DEPTH.with(|c| c.get() == 0)
}

/// Whitespace for application: inline-only at depth 0 (newlines terminate),
/// full whitespace inside delimiters.
pub fn application_ws(input: &mut &str) -> Result<()> {
    if at_depth_0() {
        take_while(0.., is_inline_whitespace).parse_next(input)?;
    } else {
        ws(input)?;
    }
    Ok(())
}

fn current_offset(i: &&str) -> usize {
    let base = BASE_PTR.with(|c| c.get());
    if base == 0 {
        return 0;
    }
    i.as_ptr() as usize - base
}

// ---------------------------------------------------------------------------
// Whitespace and comments
// ---------------------------------------------------------------------------

fn is_inline_whitespace(c: char) -> bool {
    c.is_whitespace() && c != '\n'
}

fn skip_line_comment(input: &mut &str) -> Result<()> {
    // "--" not followed by "[" or "|" → consume until newline
    ("--", not(peek(one_of(['[', '|']))), take_till(0.., '\n'))
        .void()
        .parse_next(input)
}

fn skip_doc_comment(input: &mut &str) -> Result<()> {
    ("--|", take_till(0.., '\n')).void().parse_next(input)
}

fn skip_block_comment(input: &mut &str) -> Result<()> {
    "--[".parse_next(input)?;
    let mut depth = 1usize;
    loop {
        if input.is_empty() {
            return fail.parse_next(input);
        }
        if input.starts_with("]--") {
            *input = &input[3..];
            depth -= 1;
            if depth == 0 {
                return Ok(());
            }
        } else if input.starts_with("--[") {
            *input = &input[3..];
            depth += 1;
        } else {
            let mut chars = input.chars();
            let ch = chars.next().unwrap();
            *input = &input[ch.len_utf8()..];
        }
    }
}

fn skip_ws_or_comment(input: &mut &str) -> Result<()> {
    loop {
        // skip whitespace
        take_while(0.., |c: char| c.is_whitespace()).parse_next(input)?;
        // try comments (doc first, then block, then line — order matters!)
        if input.starts_with("--|") {
            skip_doc_comment(input)?;
        } else if input.starts_with("--[") {
            skip_block_comment(input)?;
        } else if input.starts_with("--") {
            skip_line_comment(input)?;
        } else {
            break;
        }
    }
    Ok(())
}

/// Consumes any mix of whitespace and comments (including newlines).
pub fn ws(input: &mut &str) -> Result<()> {
    skip_ws_or_comment(input)?;
    bump_furthest(current_offset(input));
    Ok(())
}

/// Consumes inline whitespace and comments that don't span lines.
/// Used for the lambda-dot rule.
pub fn inline_ws1(input: &mut &str) -> Result<()> {
    // Require at least one inline whitespace char
    one_of(|c: char| is_inline_whitespace(c)).parse_next(input)?;
    take_while(0.., is_inline_whitespace).parse_next(input)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Span helper
// ---------------------------------------------------------------------------

pub fn spanned<'i, P, T>(mut p: P) -> impl Parser<&'i str, (T, Span), winnow::error::ContextError>
where
    P: Parser<&'i str, T, winnow::error::ContextError>,
{
    move |i: &mut &'i str| {
        let start = current_offset(i);
        let result = p.parse_next(i)?;
        let end = current_offset(i);
        bump_furthest(end);
        Ok((result, Span::new(start, end)))
    }
}

// ---------------------------------------------------------------------------
// Keywords and operators
// ---------------------------------------------------------------------------

const RESERVED: &[&str] = &[
    "type", "match", "if", "then", "else", "import", "true", "false",
    // future-reserved
    "select", "perform", "handle", "with", "resume",
];

/// Match exactly the keyword `kw` and verify the next char is not an
/// identifier continuation (so "type" doesn't match "typeof").
pub fn kw<'i>(keyword: &'static str) -> impl Parser<&'i str, (), winnow::error::ContextError> {
    move |input: &mut &'i str| {
        (keyword, not(peek(one_of(crate::ident::is_ident_continue))))
            .void()
            .parse_next(input)
    }
}

/// Consume operator string `op_str` and verify it's not followed by a char
/// that would extend it into a longer operator token.
fn is_op_continuation(c: char) -> bool {
    matches!(
        c,
        '=' | ':' | '?' | '.' | '>' | '<' | '|' | '&' | '+' | '-' | '*' | '/' | '%'
    )
}

pub fn op<'i>(op_str: &'static str) -> impl Parser<&'i str, (), winnow::error::ContextError> {
    move |input: &mut &'i str| {
        (op_str, not(peek(one_of(is_op_continuation))))
            .void()
            .parse_next(input)
    }
}

// ---------------------------------------------------------------------------
// Identifiers and field names
// ---------------------------------------------------------------------------

pub fn parse_atom_body(input: &mut &str) -> Result<String> {
    let body = (
        one_of(crate::ident::is_ident_start),
        take_while(0.., crate::ident::is_atom_continue),
    )
        .take()
        .parse_next(input)?;
    Ok(body.to_string())
}

/// Identifier: atom-body shape minus `-`; rejects reserved keywords.
pub fn parse_ident(input: &mut &str) -> Result<String> {
    let start = *input;
    let name = (
        one_of(crate::ident::is_ident_start),
        take_while(0.., crate::ident::is_ident_continue),
    )
        .take()
        .parse_next(input)?;

    if RESERVED.contains(&name) {
        *input = start;
        return fail.parse_next(input);
    }

    Ok(name.to_string())
}

fn parse_value_name_body<'i>(input: &mut &'i str) -> Result<&'i str> {
    let start = *input;
    (
        one_of(crate::ident::is_ident_start),
        take_while(0.., |c: char| {
            crate::ident::is_ident_continue(c) || c == '\''
        }),
    )
        .void()
        .parse_next(input)?;

    if input.starts_with('?') && !input.starts_with("?.") && !input.starts_with("??") {
        '?'.parse_next(input)?;
    }

    let consumed = start.len() - input.len();
    Ok(&start[..consumed])
}
/// Runtime value identifier: identifier shape plus value-only `'` and `?` suffixes.
pub fn parse_value_ident(input: &mut &str) -> Result<String> {
    let start = *input;
    let name = parse_value_name_body(input)?;

    if RESERVED.contains(&name) {
        *input = start;
        return fail.parse_next(input);
    }

    Ok(name.to_string())
}

/// Value-level field name: runtime value-name suffixes without keyword rejection.
pub fn parse_value_field_name(input: &mut &str) -> Result<String> {
    parse_value_name_body(input).map(str::to_string)
}

/// Field name: identifier-like shape, without keyword rejection.
pub fn parse_field_name(input: &mut &str) -> Result<String> {
    let name = (
        one_of(crate::ident::is_ident_start),
        take_while(0.., crate::ident::is_ident_continue),
    )
        .take()
        .parse_next(input)?;
    Ok(name.to_string())
}

/// Atom: `#` followed by atom-body.
pub fn parse_atom_name(input: &mut &str) -> Result<String> {
    preceded('#', parse_atom_body).parse_next(input)
}

// ---------------------------------------------------------------------------
// Booleans
// ---------------------------------------------------------------------------

pub fn parse_bool_true(input: &mut &str) -> Result<bool> {
    kw("true").map(|_| true).parse_next(input)
}

pub fn parse_bool_false(input: &mut &str) -> Result<bool> {
    kw("false").map(|_| false).parse_next(input)
}

// ---------------------------------------------------------------------------
// Strings (lifted from immediate parser)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum StringFragment<'a> {
    Literal(&'a str),
    Escaped(char),
}

pub fn parse_string(input: &mut &str) -> Result<String> {
    use winnow::combinator::delimited;
    delimited(
        '"',
        repeat(0.., parse_string_fragment).fold(String::new, |mut s, frag| {
            match frag {
                StringFragment::Literal(t) => s.push_str(t),
                StringFragment::Escaped(c) => s.push(c),
            }
            s
        }),
        '"',
    )
    .parse_next(input)
}

fn parse_string_fragment<'a>(input: &mut &'a str) -> Result<StringFragment<'a>> {
    alt((
        parse_string_literal.map(StringFragment::Literal),
        parse_escaped_char.map(StringFragment::Escaped),
    ))
    .parse_next(input)
}

fn parse_string_literal<'a>(input: &mut &'a str) -> Result<&'a str> {
    take_till(1.., ['"', '\\']).parse_next(input)
}

fn parse_escaped_char(input: &mut &str) -> Result<char> {
    preceded(
        '\\',
        alt((
            parse_unicode_escape,
            '"'.value('\"'),
            '\\'.value('\\'),
            '/'.value('/'),
            'b'.value('\x08'),
            'f'.value('\x0C'),
            'n'.value('\n'),
            'r'.value('\r'),
            't'.value('\t'),
        )),
    )
    .parse_next(input)
}

fn parse_unicode_escape(input: &mut &str) -> Result<char> {
    let first = preceded('u', parse_u16_hex_escape).parse_next(input)?;
    match first {
        0xD800..=0xDBFF => {
            let second = preceded('\\', preceded('u', parse_u16_hex_escape)).parse_next(input)?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return fail.parse_next(input);
            }
            let lead = u32::from(first - 0xD800);
            let trail = u32::from(second - 0xDC00);
            match std::char::from_u32(0x10000 + ((lead << 10) | trail)) {
                Some(cp) => Ok(cp),
                None => fail.parse_next(input),
            }
        }
        0xDC00..=0xDFFF => fail.parse_next(input),
        other => match std::char::from_u32(u32::from(other)) {
            Some(cp) => Ok(cp),
            None => fail.parse_next(input),
        },
    }
}

fn parse_u16_hex_escape(input: &mut &str) -> Result<u16> {
    take(4usize)
        .verify_map(|hex: &str| u16::from_str_radix(hex, 16).ok())
        .parse_next(input)
}

// ---------------------------------------------------------------------------
// Numbers (lifted from immediate parser)
// ---------------------------------------------------------------------------

pub fn parse_number_value(input: &mut &str) -> Result<Expr> {
    let start = *input;

    let literal = (
        opt(one_of('-')),
        digit1,
        opt(('.', digit1)),
        opt((one_of(('e', 'E')), (opt(one_of(('+', '-'))), digit1))),
    )
        .take()
        .parse_next(input)?;

    let has_sign = literal.starts_with('-');
    let has_frac_or_exp = literal.contains('.') || literal.contains('e') || literal.contains('E');
    let span = Span::new(0, 0); // caller sets span via spanned()
    let postfix_start = *input;
    let postfix_run: &str =
        take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_').parse_next(input)?;

    match classify_postfix(postfix_run, has_sign, has_frac_or_exp) {
        PostfixCheck::Valid(NumberType::Posit(spec)) => match parse_posit_literal(spec, literal) {
            Some(literal) => Ok(Expr::Posit { literal, span }),
            None => {
                *input = start;
                fail.parse_next(input)
            }
        },
        PostfixCheck::Valid(postfix) if postfix.is_float() => match literal.parse::<f64>() {
            Ok(value) => {
                let value = if matches!(postfix, NumberType::F32) {
                    value as f32 as f64
                } else {
                    value
                };
                Ok(Expr::Float {
                    value,
                    postfix: Some(postfix),
                    span,
                })
            }
            Err(_) => {
                *input = start;
                fail.parse_next(input)
            }
        },
        PostfixCheck::Valid(postfix) => {
            if matches!(postfix, NumberType::U64) {
                match literal.parse::<u64>() {
                    Ok(bits) => Ok(Expr::Integer {
                        value: bits as i64,
                        postfix: Some(postfix),
                        span,
                    }),
                    Err(_) => {
                        *input = start;
                        fail.parse_next(input)
                    }
                }
            } else {
                match literal.parse::<i64>() {
                    Ok(value) => Ok(Expr::Integer {
                        value,
                        postfix: Some(postfix),
                        span,
                    }),
                    Err(_) => {
                        *input = start;
                        fail.parse_next(input)
                    }
                }
            }
        }
        PostfixCheck::Unknown | PostfixCheck::IntOnFloatBody | PostfixCheck::UnsignedNegative => {
            *input = postfix_start;
            if has_frac_or_exp {
                match literal.parse::<f64>() {
                    Ok(value) => Ok(Expr::Float {
                        value,
                        postfix: None,
                        span,
                    }),
                    Err(_) => {
                        *input = start;
                        fail.parse_next(input)
                    }
                }
            } else {
                match literal.parse::<i64>() {
                    Ok(value) => Ok(Expr::Integer {
                        value,
                        postfix: None,
                        span,
                    }),
                    Err(_) => {
                        *input = start;
                        fail.parse_next(input)
                    }
                }
            }
        }
        PostfixCheck::None => {
            if has_frac_or_exp {
                match literal.parse::<f64>() {
                    Ok(value) => Ok(Expr::Float {
                        value,
                        postfix: None,
                        span,
                    }),
                    Err(_) => {
                        *input = start;
                        fail.parse_next(input)
                    }
                }
            } else {
                match literal.parse::<i64>() {
                    Ok(value) => Ok(Expr::Integer {
                        value,
                        postfix: None,
                        span,
                    }),
                    Err(_) => {
                        *input = start;
                        fail.parse_next(input)
                    }
                }
            }
        }
    }
}

/// Parse import source: quoted string or bare dotted path.
pub fn parse_import_source(input: &mut &str) -> Result<ImportSource> {
    alt((
        parse_string.map(ImportSource::String),
        parse_import_path.map(ImportSource::Path),
    ))
    .parse_next(input)
}

fn parse_import_path(input: &mut &str) -> Result<Vec<String>> {
    let first = parse_field_name(input)?;
    let mut parts = vec![first];
    loop {
        if opt('.').parse_next(input)?.is_none() {
            break;
        }
        parts.push(parse_field_name(input)?);
    }
    Ok(parts)
}
