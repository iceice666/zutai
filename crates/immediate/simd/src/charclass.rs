pub(crate) fn is_structural(byte: u8) -> bool {
    matches!(byte, b'{' | b'}' | b'[' | b']' | b'=' | b';')
}

pub(crate) fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\n' | b'\r' | b'\t')
}

pub(crate) fn is_name_start(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}

pub(crate) fn is_name_continue(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_continue(c)
}

pub(crate) fn is_atom_continue(c: char) -> bool {
    is_name_continue(c) || c == '-'
}
