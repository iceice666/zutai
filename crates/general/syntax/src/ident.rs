//! Shared Unicode (UAX #31) identifier character classes for general mode.

/// Identifier / atom / field-name start: `_` or a `XID_Start` scalar.
pub fn is_ident_start(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}

/// Identifier / field-name continuation: `_` or a `XID_Continue` scalar.
pub fn is_ident_continue(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_continue(c)
}

/// Atom-body continuation: identifier continuation plus `-`.
pub fn is_atom_continue(c: char) -> bool {
    is_ident_continue(c) || c == '-'
}
