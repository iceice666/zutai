use super::cursor::Cursor;
use crate::SyntaxKind;

/// Advance past a complete string literal (cursor must be at `"`).
/// Returns the first error message encountered, or `None` if well-formed.
/// Always advances past all consumed bytes so the token is lossless even on error.
pub(crate) fn scan_string(cursor: &mut Cursor<'_>) -> Option<&'static str> {
    debug_assert_eq!(cursor.peek(), Some(b'"'));
    cursor.bump(); // opening quote

    let mut error: Option<&'static str> = None;
    loop {
        match cursor.peek() {
            None => return error.or(Some("unterminated string literal")),
            Some(b'"') => {
                cursor.bump();
                return error;
            }
            Some(b'\\') => {
                cursor.bump();
                match cursor.peek() {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                        cursor.bump();
                    }
                    Some(b'u') => {
                        cursor.bump();
                        if !scan_u16_hex(cursor) {
                            error.get_or_insert("invalid unicode escape sequence");
                        }
                    }
                    None => return error.or(Some("unterminated string literal")),
                    _ => {
                        cursor.bump();
                        error.get_or_insert("invalid escape sequence");
                    }
                }
            }
            // Control character terminates the string (newlines cannot span a literal).
            Some(b) if b < 0x20 => {
                cursor.bump();
                return error.or(Some("control character in string literal"));
            }
            Some(b) if b.is_ascii() => {
                cursor.bump();
            }
            Some(_) => {
                cursor.bump_char();
            }
        }
    }
}

/// Re-validate the text of an already-tokenized STRING token.
/// Returns the first error message, or `None` if well-formed.
pub(crate) fn validate_string(text: &str) -> Option<&'static str> {
    scan_string(&mut Cursor::new(text))
}

fn scan_u16_hex(cursor: &mut Cursor<'_>) -> bool {
    for _ in 0..4 {
        match cursor.peek() {
            Some(b) if b.is_ascii_hexdigit() => {
                cursor.bump();
            }
            _ => return false,
        }
    }
    true
}

fn scan_number_core(cursor: &mut Cursor<'_>) -> (SyntaxKind, Option<&'static str>) {
    cursor.eat_while(|b| b.is_ascii_digit());

    let mut is_float = false;
    let mut error: Option<&'static str> = None;

    // Fractional part: only consume `.` if followed by a digit to avoid
    // stealing the `.` in `1.foo` (field access) or `1..` (two DOT tokens).
    if cursor.peek() == Some(b'.') && cursor.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
        is_float = true;
        cursor.bump(); // `.`
        cursor.eat_while(|b| b.is_ascii_digit());
    }

    // Exponent
    if matches!(cursor.peek(), Some(b'e' | b'E')) {
        is_float = true;
        cursor.bump();
        if matches!(cursor.peek(), Some(b'+' | b'-')) {
            cursor.bump();
        }
        let exp_start = cursor.pos();
        cursor.eat_while(|b| b.is_ascii_digit());
        if cursor.pos() == exp_start {
            error = Some("exponent has no digits");
        }
    }

    let kind = if is_float {
        SyntaxKind::FLOAT
    } else {
        SyntaxKind::INT
    };
    (kind, error)
}

/// Advance past an integer or float literal (cursor must be at an ASCII digit).
/// Returns `INT` or `FLOAT`. Never fails; leading-zero validation is left to
/// the parser/validator.
pub(crate) fn scan_number(cursor: &mut Cursor<'_>) -> SyntaxKind {
    debug_assert!(cursor.peek().is_some_and(|b| b.is_ascii_digit()));
    scan_number_core(cursor).0
}

/// Re-validate the text of an already-tokenized INT or FLOAT token.
/// Returns an error message if malformed, or `None` if well-formed.
pub(crate) fn validate_number(text: &str) -> Option<&'static str> {
    scan_number_core(&mut Cursor::new(text)).1
}
