use super::cursor::Cursor;
use crate::SyntaxKind;

/// Advance past a complete string literal (cursor must be at `"`).
/// Returns whether the string was well-formed. Always advances past all
/// consumed bytes so the token is lossless even on error.
pub(crate) fn scan_string(cursor: &mut Cursor<'_>) -> bool {
    debug_assert_eq!(cursor.peek(), Some(b'"'));
    cursor.bump(); // opening quote

    loop {
        match cursor.peek() {
            None => return false, // unclosed string
            Some(b'"') => {
                cursor.bump();
                return true;
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
                            // Invalid \uXXXX but keep scanning until closing quote
                            // to preserve losslessness; the real diagnostic comes later.
                        } else {
                            // High surrogate: expect \uXXXX continuation
                            // (simplified: skip surrogate-pair validation for lexer)
                        }
                    }
                    None => return false,
                    _ => {
                        cursor.bump();
                        // Invalid escape sequence; keep scanning.
                    }
                }
            }
            Some(b) if b < 0x20 => {
                // Unescaped control character — advance and flag as malformed.
                cursor.bump();
                return false;
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

/// Advance past an integer or float literal (cursor must be at an ASCII digit).
/// Returns `INT` or `FLOAT`. Never fails; leading-zero validation is left to
/// the parser/validator.
pub(crate) fn scan_number(cursor: &mut Cursor<'_>) -> SyntaxKind {
    debug_assert!(cursor.peek().is_some_and(|b| b.is_ascii_digit()));

    cursor.eat_while(|b| b.is_ascii_digit());

    let mut is_float = false;

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
        cursor.eat_while(|b| b.is_ascii_digit());
    }

    if is_float {
        SyntaxKind::FLOAT
    } else {
        SyntaxKind::INT
    }
}
