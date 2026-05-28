use crate::SyntaxKind;

use super::{
    super::{CompletedMarker, Parser},
    primary::field_name,
};

/// Parse a full pattern. Returns `None` when the current token cannot start a pattern.
pub(crate) fn pattern(p: &mut Parser) -> Option<CompletedMarker> {
    match p.current() {
        SyntaxKind::UNDERSCORE => {
            let m = p.start();
            p.bump_any();
            Some(m.complete(p, SyntaxKind::WILDCARD_PATTERN))
        }
        SyntaxKind::INT
        | SyntaxKind::FLOAT
        | SyntaxKind::STRING
        | SyntaxKind::ATOM
        | SyntaxKind::KW_TRUE
        | SyntaxKind::KW_FALSE
        | SyntaxKind::KW_NONE
        | SyntaxKind::IDENT => {
            let m = p.start();
            p.bump_any();
            Some(m.complete(p, SyntaxKind::LITERAL))
        }
        // Negative literal: MINUS immediately adjacent (no trivia) to INT or FLOAT.
        SyntaxKind::MINUS
            if p.raw_adjacent() && matches!(p.nth(1), SyntaxKind::INT | SyntaxKind::FLOAT) =>
        {
            let m = p.start();
            p.bump(SyntaxKind::MINUS);
            p.bump_any();
            Some(m.complete(p, SyntaxKind::LITERAL))
        }
        SyntaxKind::L_PAREN => Some(tuple_pattern(p)),
        SyntaxKind::L_BRACE => Some(record_pattern(p)),
        _ => None,
    }
}

// ── Tuple / variant pattern ───────────────────────────────────────────────────
//
// Grammar: TuplePattern ::= "(" Atom ("," PatternField)* ")" | "()"
//
// A non-empty paren pattern MUST lead with an Atom tag (the variant discriminant).
// There is no positional-tuple pattern in v0.

fn tuple_pattern(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_PAREN);

    if !p.at(SyntaxKind::R_PAREN) {
        if !p.eat(SyntaxKind::ATOM) {
            p.error("expected atom tag (e.g. #tag) as first element of a variant pattern");
        }
        while p.eat(SyntaxKind::COMMA) {
            pattern_field(p);
        }
    }

    p.expect(SyntaxKind::R_PAREN);
    m.complete(p, SyntaxKind::TUPLE_PATTERN)
}

// ── Record pattern ────────────────────────────────────────────────────────────
//
// Grammar: RecordPattern ::= "{" (FieldName "=" Pattern ";")* "}"

fn record_pattern(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACE);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACE) {
        pattern_field(p);
        p.expect(SyntaxKind::SEMI);
    }
    p.expect(SyntaxKind::R_BRACE);
    m.complete(p, SyntaxKind::RECORD_PATTERN)
}

// ── Pattern field ─────────────────────────────────────────────────────────────
//
// Grammar: PatternField ::= FieldName "=" Pattern
// (No separator/terminator here — callers consume "," or ";".)

fn pattern_field(p: &mut Parser) {
    let m = p.start();
    field_name(p);
    if !p.eat(SyntaxKind::EQ) {
        p.error("expected '=' in pattern field");
    }
    if pattern(p).is_none() {
        p.error("expected pattern after '='");
    }
    m.complete(p, SyntaxKind::PATTERN_FIELD);
}
