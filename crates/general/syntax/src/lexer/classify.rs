use crate::SyntaxKind;

pub(crate) fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

pub(crate) fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

pub(crate) fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Continue predicate for atom bodies and field names: allows `-` in addition to ident chars.
pub(crate) fn is_atom_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

pub(crate) fn keyword_kind(s: &str) -> SyntaxKind {
    match s {
        "_" => SyntaxKind::UNDERSCORE,
        "type" => SyntaxKind::KW_TYPE,
        "match" => SyntaxKind::KW_MATCH,
        "if" => SyntaxKind::KW_IF,
        "then" => SyntaxKind::KW_THEN,
        "else" => SyntaxKind::KW_ELSE,
        "import" => SyntaxKind::KW_IMPORT,
        "true" => SyntaxKind::KW_TRUE,
        "false" => SyntaxKind::KW_FALSE,
        "none" => SyntaxKind::KW_NONE,
        "select" => SyntaxKind::KW_SELECT,
        _ => SyntaxKind::IDENT,
    }
}
