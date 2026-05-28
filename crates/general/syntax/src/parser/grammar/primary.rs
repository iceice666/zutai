use crate::SyntaxKind;

use super::{
    super::{CompletedMarker, Parser},
    Ctx,
};

/// Parse a primary (atom-level) expression or type.
pub(super) fn primary(p: &mut Parser, ctx: Ctx) -> Option<CompletedMarker> {
    match p.current() {
        SyntaxKind::INT | SyntaxKind::FLOAT | SyntaxKind::STRING | SyntaxKind::ATOM => {
            let m = p.start();
            p.bump_any();
            Some(m.complete(p, SyntaxKind::LITERAL))
        }
        SyntaxKind::KW_TRUE | SyntaxKind::KW_FALSE | SyntaxKind::KW_NONE => {
            let m = p.start();
            p.bump_any();
            Some(m.complete(p, SyntaxKind::LITERAL))
        }
        // IDENT and UNDERSCORE wrapped in LITERAL for Pratt precede compatibility.
        // M11 typed-AST layer will distinguish NameRef / WildcardExpr.
        SyntaxKind::IDENT | SyntaxKind::UNDERSCORE => {
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
        SyntaxKind::L_PAREN => {
            if ctx == Ctx::Type && p.nth_at(1, SyntaxKind::ATOM) {
                Some(super::types::variant_type_inner(p))
            } else {
                Some(paren_or_tuple(p, ctx))
            }
        }
        SyntaxKind::L_BRACE => {
            if ctx == Ctx::Type {
                Some(super::types::type_record_inner(p))
            } else {
                Some(brace_expr(p, ctx))
            }
        }
        SyntaxKind::L_BRACK => {
            if ctx == Ctx::Type {
                Some(super::types::type_union_inner(p))
            } else {
                Some(list_expr(p, ctx))
            }
        }
        SyntaxKind::BACKSLASH => Some(lambda_expr(p, ctx)),
        SyntaxKind::KW_IF => Some(if_expr(p, ctx)),
        SyntaxKind::KW_MATCH => Some(match_expr(p, ctx)),
        SyntaxKind::KW_IMPORT => Some(import_expr(p)),
        SyntaxKind::KW_TYPE => Some(super::types::type_form(p)),
        _ => None,
    }
}

// ── Parenthesised expression or tuple ────────────────────────────────────────

fn paren_or_tuple(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_PAREN);

    if p.at(SyntaxKind::R_PAREN) {
        p.error("expected expression");
        p.bump(SyntaxKind::R_PAREN);
        return m.complete(p, SyntaxKind::TUPLE_EXPR);
    }

    tuple_item(p, ctx);

    if p.eat(SyntaxKind::COMMA) {
        // More items → this is a tuple.
        while !p.at_eof() && !p.at(SyntaxKind::R_PAREN) {
            tuple_item(p, ctx);
            if !p.eat(SyntaxKind::COMMA) {
                break;
            }
        }
        p.expect(SyntaxKind::R_PAREN);
        m.complete(p, SyntaxKind::TUPLE_EXPR)
    } else {
        p.expect(SyntaxKind::R_PAREN);
        m.complete(p, SyntaxKind::PAREN_EXPR)
    }
}

/// One item inside `(...)`: either a named field (`IDENT = expr` → VALUE_FIELD)
/// or a positional expression (`expr` → TUPLE_ITEM).
fn tuple_item(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    if p.at(SyntaxKind::IDENT) && p.nth_at(1, SyntaxKind::EQ) {
        let m = p.start();
        p.bump(SyntaxKind::IDENT);
        p.bump(SyntaxKind::EQ);
        if super::exprs::expr_bp(p, 0, ctx).is_none() {
            p.error("expected expression for tuple field");
        }
        m.complete(p, SyntaxKind::VALUE_FIELD)
    } else {
        let m = p.start();
        if super::exprs::expr_bp(p, 0, ctx).is_none() {
            p.error("expected expression in tuple");
        }
        m.complete(p, SyntaxKind::TUPLE_ITEM)
    }
}

// ── Brace expression: value record or block ───────────────────────────────────
//
// Disambiguation (bounded lookahead from `{`):
//   { }                              → empty RECORD_EXPR
//   { IDENT (- IDENT)* =             → value RECORD_EXPR (field name then EQ)
//   otherwise                        → BLOCK
//
// Hyphenated field names are handled by scanning through adjacent MINUS IDENT
// pairs to find the EQ, matching the field_name() parser logic.

fn brace_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    if looks_like_record(p) {
        record_expr(p)
    } else {
        block_expr(p, ctx)
    }
}

/// True when the tokens after `{` look like a value record.
/// Scans through an optional `IDENT (MINUS IDENT)*` field-name prefix
/// (respecting raw-adjacency for hyphens) and checks for a trailing `=`.
fn looks_like_record(p: &Parser) -> bool {
    // p.nth(0) = `{`; scan starting at offset 1 (first token inside brace).
    match p.nth(1) {
        SyntaxKind::R_BRACE => return true,
        SyntaxKind::IDENT => {}
        _ => return false,
    }
    // Scan past MINUS IDENT pairs while raw-adjacent (hyphenated field name).
    let mut off = 1usize; // currently at the leading IDENT
    while p.raw_adjacent_at(off)          // IDENT at off is adjacent to MINUS at off+1
        && p.nth_at(off + 1, SyntaxKind::MINUS)
        && p.raw_adjacent_at(off + 1)     // MINUS at off+1 is adjacent to IDENT at off+2
        && p.nth_at(off + 2, SyntaxKind::IDENT)
    {
        off += 2;
    }
    // After the field name, the next token must be `=`.
    p.nth_at(off + 1, SyntaxKind::EQ)
}

fn record_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACE);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACE) {
        value_field(p);
    }
    p.expect(SyntaxKind::R_BRACE);
    m.complete(p, SyntaxKind::RECORD_EXPR)
}

fn value_field(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    field_name(p);
    if !p.eat(SyntaxKind::EQ) {
        p.error("expected '=' in record field");
    }
    if super::exprs::expr_bp(p, 0, Ctx::Expr).is_none() {
        p.error("expected expression for field value");
    }
    p.expect(SyntaxKind::SEMI);
    m.complete(p, SyntaxKind::VALUE_FIELD)
}

/// Parse a field name: `IDENT (MINUS IDENT)*` where all tokens are raw-adjacent.
/// Produces a FIELD_NAME node. Emits an error if no IDENT is present.
pub(super) fn field_name(p: &mut Parser) -> Option<CompletedMarker> {
    if !p.at(SyntaxKind::IDENT) {
        p.error("expected field name");
        return None;
    }
    let m = p.start();
    p.bump(SyntaxKind::IDENT);
    // Consume adjacent `MINUS IDENT` pairs for hyphenated field names (M8).
    // prev_raw_adjacent(): was the IDENT we just consumed adjacent to the current MINUS?
    // raw_adjacent(): is the current MINUS adjacent to the next IDENT?
    while p.at(SyntaxKind::MINUS)
        && p.prev_raw_adjacent()
        && p.raw_adjacent()
        && p.nth_at(1, SyntaxKind::IDENT)
    {
        p.bump(SyntaxKind::MINUS);
        p.bump(SyntaxKind::IDENT);
    }
    Some(m.complete(p, SyntaxKind::FIELD_NAME))
}

// ── Block expression ──────────────────────────────────────────────────────────

/// Parse `{ (IDENT := expr ;)* expr }` as a BLOCK node.
pub(crate) fn block_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACE);
    block_body(p, ctx);
    p.expect(SyntaxKind::R_BRACE);
    m.complete(p, SyntaxKind::BLOCK)
}

/// Parse the body of a block (local bindings then a final expression), stopping
/// before `}`. Called by `block_expr` and will be reused by clause parsing in M7.
pub(crate) fn block_body(p: &mut Parser, ctx: Ctx) {
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACE) {
        if p.at(SyntaxKind::IDENT) && p.nth_at(1, SyntaxKind::COLON_EQ) {
            let bm = p.start();
            p.bump(SyntaxKind::IDENT);
            p.bump(SyntaxKind::COLON_EQ);
            if super::exprs::expr_bp(p, 0, ctx).is_none() {
                p.error("expected expression in local binding");
            }
            p.expect(SyntaxKind::SEMI);
            bm.complete(p, SyntaxKind::LOCAL_BINDING);
        } else {
            // Final expression — only one allowed.
            if super::exprs::expr_bp(p, 0, ctx).is_none() {
                p.error("expected expression in block body");
            }
            break;
        }
    }
}

// ── List expression ───────────────────────────────────────────────────────────

fn list_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACK);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACK) {
        let im = p.start();
        if super::exprs::expr_bp(p, 0, ctx).is_none() {
            p.error("expected expression in list");
            im.abandon(p);
            break;
        }
        p.expect(SyntaxKind::SEMI);
        im.complete(p, SyntaxKind::LIST_ITEM);
    }
    p.expect(SyntaxKind::R_BRACK);
    m.complete(p, SyntaxKind::LIST_EXPR)
}

// ── Lambda expression ─────────────────────────────────────────────────────────

fn lambda_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::BACKSLASH);
    // Consume space-separated simple patterns until `=>` or `{`.
    while !p.at_eof() && !p.at(SyntaxKind::FAT_ARROW) && !p.at(SyntaxKind::L_BRACE) {
        if simple_pattern(p).is_none() {
            break;
        }
    }
    if p.eat(SyntaxKind::FAT_ARROW) {
        if super::exprs::expr_bp(p, 0, ctx).is_none() {
            p.error("expected expression after '=>' in lambda");
        }
    } else if p.at(SyntaxKind::L_BRACE) {
        block_expr(p, ctx);
    } else {
        p.error("expected '=>' or '{' in lambda expression");
    }
    m.complete(p, SyntaxKind::LAMBDA_EXPR)
}

/// Parse a simple pattern (no struct/tuple decomposition beyond parentheses).
/// Used in match cases and lambda parameter lists.
pub(crate) fn simple_pattern(p: &mut Parser) -> Option<CompletedMarker> {
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
        SyntaxKind::MINUS
            if p.raw_adjacent() && matches!(p.nth(1), SyntaxKind::INT | SyntaxKind::FLOAT) =>
        {
            let m = p.start();
            p.bump(SyntaxKind::MINUS);
            p.bump_any();
            Some(m.complete(p, SyntaxKind::LITERAL))
        }
        // Parenthesised pattern: covers single-atom variants `(#tag)` and
        // multi-field variants `(#tag, field = pat)` needed by M6.
        SyntaxKind::L_PAREN => {
            let m = p.start();
            p.bump(SyntaxKind::L_PAREN);
            while !p.at_eof() && !p.at(SyntaxKind::R_PAREN) {
                if p.at(SyntaxKind::IDENT) && p.nth_at(1, SyntaxKind::EQ) {
                    // Named field: IDENT = pattern
                    let fm = p.start();
                    p.bump(SyntaxKind::IDENT);
                    p.bump(SyntaxKind::EQ);
                    if simple_pattern(p).is_none() {
                        p.error("expected pattern after '='");
                    }
                    fm.complete(p, SyntaxKind::PATTERN_FIELD);
                } else if simple_pattern(p).is_none() {
                    p.error(format!("unexpected token in pattern: {:?}", p.current()));
                    p.bump_any();
                }
                if !p.eat(SyntaxKind::COMMA) {
                    break;
                }
            }
            p.expect(SyntaxKind::R_PAREN);
            Some(m.complete(p, SyntaxKind::TUPLE_PATTERN))
        }
        _ => None,
    }
}

// ── If expression ─────────────────────────────────────────────────────────────

fn if_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::KW_IF);
    if super::exprs::expr_bp(p, 0, ctx).is_none() {
        p.error("expected condition in 'if'");
    }
    p.expect(SyntaxKind::KW_THEN);
    if super::exprs::expr_bp(p, 0, ctx).is_none() {
        p.error("expected expression after 'then'");
    }
    p.expect(SyntaxKind::KW_ELSE);
    if super::exprs::expr_bp(p, 0, ctx).is_none() {
        p.error("expected expression after 'else'");
    }
    m.complete(p, SyntaxKind::IF_EXPR)
}

// ── Match expression ──────────────────────────────────────────────────────────

fn match_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::KW_MATCH);
    if super::exprs::expr_bp(p, 0, ctx).is_none() {
        p.error("expected expression after 'match'");
    }
    p.expect(SyntaxKind::L_BRACE);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACE) {
        match_case(p, ctx);
    }
    p.expect(SyntaxKind::R_BRACE);
    m.complete(p, SyntaxKind::MATCH_EXPR)
}

fn match_case(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    if simple_pattern(p).is_none() {
        p.error(format!(
            "expected pattern in match case, got {:?}",
            p.current()
        ));
        // Skip to `=>` or `}` for recovery.
        while !p.at_eof()
            && !p.at(SyntaxKind::FAT_ARROW)
            && !p.at(SyntaxKind::SEMI)
            && !p.at(SyntaxKind::R_BRACE)
        {
            p.bump_any();
        }
    }
    p.expect(SyntaxKind::FAT_ARROW);
    if super::exprs::expr_bp(p, 0, ctx).is_none() {
        p.error("expected expression in match arm");
    }
    p.expect(SyntaxKind::SEMI);
    m.complete(p, SyntaxKind::MATCH_CASE)
}

// ── Import expression ─────────────────────────────────────────────────────────

fn import_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::KW_IMPORT);
    let pm = p.start();
    if !p.eat(SyntaxKind::STRING) {
        p.error("expected string path after 'import'");
    }
    pm.complete(p, SyntaxKind::IMPORT_PATH);
    m.complete(p, SyntaxKind::IMPORT_EXPR)
}
