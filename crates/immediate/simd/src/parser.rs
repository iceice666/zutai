use rustc_hash::FxHashSet;
use zutai_types::{Block, Pair, Value};

use crate::charclass::{is_atom_continue, is_name_continue, is_name_start};
use crate::error::{ParseError, ParseErrorKind, error};
use crate::string_scan::{StringSpecialFinder, select_string_special_finder};

pub(crate) struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    significant: &'a [usize],
    cursor: usize,
    pos: usize,
    find_string_special: StringSpecialFinder,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(input: &'a str, significant: &'a [usize]) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            significant,
            cursor: 0,
            pos: 0,
            find_string_special: select_string_special_finder(),
        }
    }

    pub(crate) fn parse_document(mut self) -> Result<Block, ParseError> {
        self.skip_to_next_significant();
        let block = self.parse_block()?;
        self.skip_to_next_significant();

        if self.pos == self.bytes.len() {
            Ok(block)
        } else {
            Err(error(self.pos, ParseErrorKind::TrailingData))
        }
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect_byte(b'{', "`{`")?;
        let mut pairs = Vec::new();
        let mut seen = FxHashSet::default();

        loop {
            self.skip_to_next_significant();
            match self.peek_byte() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Block(pairs));
                }
                Some(_) => {
                    let pair = self.parse_pair()?;
                    if !seen.insert(pair.field_name.clone()) {
                        return Err(error(
                            self.pos,
                            ParseErrorKind::DuplicateField(pair.field_name),
                        ));
                    }
                    pairs.push(pair);
                }
                None => {
                    return Err(error(
                        self.pos,
                        ParseErrorKind::Expected { expected: "`}`" },
                    ));
                }
            }
        }
    }

    fn parse_pair(&mut self) -> Result<Pair, ParseError> {
        self.skip_to_next_significant();
        let field_name = self.parse_name()?;
        self.skip_to_next_significant();
        self.expect_byte(b'=', "`=`")?;
        self.skip_to_next_significant();
        let value = self.parse_value()?;
        self.skip_to_next_significant();
        self.expect_byte(b';', "`;`")?;

        Ok(Pair { field_name, value })
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.expect_byte(b'[', "`[`")?;
        let mut values = Vec::new();

        loop {
            self.skip_to_next_significant();
            match self.peek_byte() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(values));
                }
                Some(_) => {
                    values.push(self.parse_value()?);
                    self.skip_to_next_significant();
                    self.expect_byte(b';', "`;`")?;
                }
                None => {
                    return Err(error(
                        self.pos,
                        ParseErrorKind::Expected { expected: "`]`" },
                    ));
                }
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        match self.peek_byte() {
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b'#') => self.parse_atom().map(Value::Atom),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_block().map(Value::Block),
            Some(b't') => self.parse_keyword("true", Value::True),
            Some(b'f') => self.parse_keyword("false", Value::False),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) | None => Err(error(
                self.pos,
                ParseErrorKind::Expected {
                    expected: "immediate value",
                },
            )),
        }
    }

    fn parse_name(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        let Some(first) = self.peek_byte() else {
            return Err(error(
                start,
                ParseErrorKind::Expected {
                    expected: "field name",
                },
            ));
        };
        if !is_name_start(first) {
            return Err(error(
                start,
                ParseErrorKind::Expected {
                    expected: "field name",
                },
            ));
        }

        self.pos += 1;
        while let Some(byte) = self.peek_byte() {
            if !is_name_continue(byte) {
                break;
            }
            self.pos += 1;
        }
        if self.peek_byte() == Some(b'-') {
            return Err(error(
                self.pos,
                ParseErrorKind::Expected {
                    expected: "field name without `-`",
                },
            ));
        }

        Ok(self.input[start..self.pos].to_string())
    }

    fn parse_atom(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        self.expect_byte(b'#', "`#`")?;

        let Some(first) = self.peek_byte() else {
            return Err(error(start, ParseErrorKind::InvalidAtom));
        };
        if !is_name_start(first) {
            return Err(error(start, ParseErrorKind::InvalidAtom));
        }

        let atom_start = self.pos;
        self.pos += 1;
        while let Some(byte) = self.peek_byte() {
            if !is_atom_continue(byte) {
                break;
            }
            self.pos += 1;
        }

        Ok(self.input[atom_start..self.pos].to_string())
    }

    fn parse_keyword(&mut self, keyword: &'static str, value: Value) -> Result<Value, ParseError> {
        let end = self.pos + keyword.len();
        if self.input.get(self.pos..end) != Some(keyword) {
            return Err(error(
                self.pos,
                ParseErrorKind::Expected { expected: keyword },
            ));
        }
        if self
            .bytes
            .get(end)
            .is_some_and(|byte| is_atom_continue(*byte))
        {
            return Err(error(
                self.pos,
                ParseErrorKind::Expected { expected: keyword },
            ));
        }

        self.pos = end;
        Ok(value)
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        let string_start = self.pos;
        self.expect_byte(b'"', "`\"`")?;
        let mut output = String::new();
        let mut literal_start = self.pos;

        let find_special = self.find_string_special;
        loop {
            let Some(offset) = find_special(&self.bytes[self.pos..]) else {
                return Err(error(string_start, ParseErrorKind::UnclosedString));
            };
            self.pos += offset;
            match self.bytes[self.pos] {
                b'"' => {
                    output.push_str(&self.input[literal_start..self.pos]);
                    self.pos += 1;
                    return Ok(output);
                }
                b'\\' => {
                    output.push_str(&self.input[literal_start..self.pos]);
                    self.pos += 1;
                    let escaped = self.parse_escape()?;
                    output.push(escaped);
                    literal_start = self.pos;
                }
                _ => return Err(error(self.pos, ParseErrorKind::InvalidString)),
            }
        }
    }

    fn parse_escape(&mut self) -> Result<char, ParseError> {
        let escape_offset = self.pos.saturating_sub(1);
        let Some(byte) = self.peek_byte() else {
            return Err(error(escape_offset, ParseErrorKind::InvalidEscape));
        };
        self.pos += 1;

        match byte {
            b'"' => Ok('"'),
            b'\\' => Ok('\\'),
            b'/' => Ok('/'),
            b'b' => Ok('\u{0008}'),
            b'f' => Ok('\u{000c}'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'u' => self.parse_unicode_escape(escape_offset),
            _ => Err(error(escape_offset, ParseErrorKind::InvalidEscape)),
        }
    }

    fn parse_unicode_escape(&mut self, escape_offset: usize) -> Result<char, ParseError> {
        let first = self.parse_u16_hex(escape_offset)?;

        match first {
            0xD800..=0xDBFF => {
                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                    return Err(error(escape_offset, ParseErrorKind::InvalidEscape));
                }
                self.pos += 2;
                let second = self.parse_u16_hex(escape_offset)?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(error(escape_offset, ParseErrorKind::InvalidEscape));
                }

                let lead = u32::from(first - 0xD800);
                let trail = u32::from(second - 0xDC00);
                char::from_u32(0x10000 + ((lead << 10) | trail))
                    .ok_or_else(|| error(escape_offset, ParseErrorKind::InvalidEscape))
            }
            0xDC00..=0xDFFF => Err(error(escape_offset, ParseErrorKind::InvalidEscape)),
            other => char::from_u32(u32::from(other))
                .ok_or_else(|| error(escape_offset, ParseErrorKind::InvalidEscape)),
        }
    }

    fn parse_u16_hex(&mut self, escape_offset: usize) -> Result<u16, ParseError> {
        let end = self.pos + 4;
        let Some(hex) = self.input.get(self.pos..end) else {
            return Err(error(escape_offset, ParseErrorKind::InvalidEscape));
        };
        if !hex.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(error(escape_offset, ParseErrorKind::InvalidEscape));
        }

        self.pos = end;
        u16::from_str_radix(hex, 16)
            .map_err(|_| error(escape_offset, ParseErrorKind::InvalidEscape))
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;

        if self.peek_byte() == Some(b'-') {
            self.pos += 1;
        }

        match self.peek_byte() {
            Some(b'0') => {
                self.pos += 1;
                if self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(error(start, ParseErrorKind::InvalidNumber));
                }
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(error(start, ParseErrorKind::InvalidNumber)),
        }

        let mut is_float = false;
        if self.peek_byte() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(error(start, ParseErrorKind::InvalidNumber));
            }
            while self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }

        if matches!(self.peek_byte(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek_byte(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(error(start, ParseErrorKind::InvalidNumber));
            }
            while self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }

        let literal = &self.input[start..self.pos];
        if is_float {
            let value = literal
                .parse::<f64>()
                .map_err(|_| error(start, ParseErrorKind::InvalidNumber))?;
            if value.is_finite() {
                Ok(Value::Float(value))
            } else {
                Err(error(start, ParseErrorKind::InvalidNumber))
            }
        } else {
            literal
                .parse::<i64>()
                .map(Value::Integer)
                .map_err(|_| error(start, ParseErrorKind::InvalidNumber))
        }
    }

    fn expect_byte(&mut self, byte: u8, expected: &'static str) -> Result<(), ParseError> {
        if self.peek_byte() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(error(self.pos, ParseErrorKind::Expected { expected }))
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_to_next_significant(&mut self) {
        let sig = self.significant;
        let mut cursor = self.cursor;
        while cursor < sig.len() && sig[cursor] < self.pos {
            cursor += 1;
        }
        self.cursor = cursor;
        self.pos = sig.get(cursor).copied().unwrap_or(self.bytes.len());
    }
}
