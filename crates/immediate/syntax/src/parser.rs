use rustc_hash::FxHashSet;

use winnow::Parser;
use winnow::Result;
use winnow::ascii::digit1;
use winnow::ascii::multispace0;
use winnow::combinator::{alt, delimited, eof, fail, not, opt, peek, preceded, repeat, terminated};
use winnow::token::{one_of, take, take_till, take_while};

use crate::ast::{Block, Pair, Value};

pub fn parse(input: &mut &str) -> Result<Block> {
    let block = (multispace0, parse_block, multispace0, eof)
        .map(|(_, block, _, _)| block)
        .parse_next(input)?;

    Ok(block)
}

fn is_atom_body_continuation(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_atom_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub fn parse_block(input: &mut &str) -> Result<Block> {
    let mut seen: FxHashSet<String> = FxHashSet::default();
    '{'.parse_next(input)?;

    let pairs: Vec<Pair> = repeat(0.., parse_pair).parse_next(input)?;
    multispace0.parse_next(input)?;
    '}'.parse_next(input)?;

    for pair in &pairs {
        if !seen.insert(pair.field_name.clone()) {
            fail.parse_next(input)?;
        }
    }

    Ok(Block(pairs))
}

pub fn parse_pair(input: &mut &str) -> Result<Pair> {
    (
        multispace0,
        parse_field_name,
        multispace0,
        '=',
        multispace0,
        parse_value,
        multispace0,
        ';',
    )
        .map(|(_, field_name, _, _, _, value, _, _)| Pair { field_name, value })
        .parse_next(input)
}

pub fn parse_array(input: &mut &str) -> Result<Value> {
    '['.parse_next(input)?;

    let values = repeat(
        0..,
        terminated(
            (multispace0, parse_value, multispace0).map(|(_, value, _)| value),
            ';',
        ),
    )
    .parse_next(input)?;

    multispace0.parse_next(input)?;
    ']'.parse_next(input)?;

    Ok(Value::Array(values))
}

pub fn parse_atom(input: &mut &str) -> Result<String> {
    '#'.parse_next(input)?;
    parse_atom_body(input)
}

pub fn parse_atom_body(input: &mut &str) -> Result<String> {
    let atom = (
        one_of(is_atom_start),
        take_while(0.., is_atom_body_continuation),
    )
        .take()
        .parse_next(input)?;

    Ok(atom.to_string())
}

pub fn parse_value(input: &mut &str) -> Result<Value> {
    alt((
        parse_string.map(Value::String),
        parse_atom.map(Value::Atom),
        parse_array,
        parse_block.map(Value::Block),
        parse_number,
        parse_true,
        parse_false,
        parse_none,
    ))
    .parse_next(input)
}

fn parse_word(input: &mut &str, keyword: &str) -> Result<()> {
    (keyword, not(peek(one_of(is_atom_body_continuation))))
        .void()
        .parse_next(input)
}

fn parse_true(input: &mut &str) -> Result<Value> {
    parse_word(input, "true")?;
    Ok(Value::True)
}

fn parse_false(input: &mut &str) -> Result<Value> {
    parse_word(input, "false")?;
    Ok(Value::False)
}

fn parse_none(input: &mut &str) -> Result<Value> {
    parse_word(input, "none")?;
    Ok(Value::None)
}

pub fn parse_field_name(input: &mut &str) -> Result<String> {
    parse_atom_body(input)
}

#[derive(Debug, Clone, Copy)]
enum StringFragment<'a> {
    Literal(&'a str),
    Escaped(char),
}

pub fn parse_string(input: &mut &str) -> Result<String> {
    delimited(
        '"',
        repeat(0.., parse_string_fragment).fold(String::new, |mut string, fragment| {
            match fragment {
                StringFragment::Literal(text) => string.push_str(text),
                StringFragment::Escaped(ch) => string.push(ch),
            }
            string
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
                fail.parse_next(input)?;
            }

            let lead = u32::from(first - 0xD800);
            let trail = u32::from(second - 0xDC00);
            match std::char::from_u32(0x10000 + ((lead << 10) | trail)) {
                Some(code_point) => Ok(code_point),
                None => fail.parse_next(input),
            }
        }
        0xDC00..=0xDFFF => fail.parse_next(input),
        other => match std::char::from_u32(u32::from(other)) {
            Some(code_point) => Ok(code_point),
            None => fail.parse_next(input),
        },
    }
}

fn parse_u16_hex_escape(input: &mut &str) -> Result<u16> {
    take(4usize)
        .verify_map(|hex: &str| u16::from_str_radix(hex, 16).ok())
        .parse_next(input)
}

pub fn parse_number(input: &mut &str) -> Result<Value> {
    let start = *input;

    let literal = (
        opt(one_of('-')),
        digit1,
        opt(('.', digit1)),
        opt((one_of(('e', 'E')), (opt(one_of(('+', '-'))), digit1))),
    )
        .take()
        .parse_next(input)?;

    let is_float = literal.contains('.') || literal.contains('e') || literal.contains('E');

    if is_float {
        match literal.parse::<f64>() {
            Ok(value) => Ok(Value::Float(value)),
            Err(_) => {
                *input = start;
                fail.parse_next(input)
            }
        }
    } else {
        match literal.parse::<i64>() {
            Ok(value) => Ok(Value::Integer(value)),
            Err(_) => {
                *input = start;
                fail.parse_next(input)
            }
        }
    }
}

#[cfg(test)]
mod tests {}
