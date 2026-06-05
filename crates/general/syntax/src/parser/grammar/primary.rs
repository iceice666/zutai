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
        SyntaxKind::L_PAREN => Some(paren_or_tuple(p, ctx)),
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
        p.bump(SyntaxKind::R_PAREN);
        return m.complete(p, SyntaxKind::TUPLE_EXPR);
    }

    maybe_node_comment_tuple_item(p, ctx);

    if p.eat(SyntaxKind::COMMA) {
        // More items → this is a tuple.
        while !p.at_eof() && !p.at(SyntaxKind::R_PAREN) {
            maybe_node_comment_tuple_item(p, ctx);
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

/// One item inside `(...)`: either a value named field (`IDENT = expr` or
/// `field-name = expr` → VALUE_FIELD), a type named field in type context
/// (`field-name : Type` → TYPE_TUPLE_FIELD), or a positional item
/// (→ TUPLE_ITEM).
fn tuple_item(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    if ctx == Ctx::Type && looks_like_type_named_field(p) {
        let m = p.start();
        field_name(p);
        p.bump(SyntaxKind::COLON);
        if super::exprs::type_expr(p).is_none() {
            p.error("expected type expression for tuple field");
        }
        m.complete(p, SyntaxKind::TYPE_TUPLE_FIELD)
    } else if looks_like_named_field(p) {
        let m = p.start();
        field_name(p);
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

fn maybe_node_comment_tuple_item(p: &mut Parser, ctx: Ctx) {
    if p.at(SyntaxKind::NODE_COMMENT) {
        let cm = p.start();
        p.bump(SyntaxKind::NODE_COMMENT);
        tuple_item(p, ctx);
        cm.complete(p, SyntaxKind::NODE_COMMENT_NODE);
    } else {
        tuple_item(p, ctx);
    }
}

/// True when the current token sequence looks like a named tuple field:
/// `IDENT (MINUS IDENT)* EQ` where hyphens are raw-adjacent.
///
/// This mirrors `looks_like_record` but scans from offset 0 (the current
/// IDENT rather than offset 1 inside a `{`).
fn looks_like_named_field(p: &Parser) -> bool {
    looks_like_field_with(p, SyntaxKind::EQ)
}

fn looks_like_type_named_field(p: &Parser) -> bool {
    looks_like_field_with(p, SyntaxKind::COLON)
}

fn looks_like_field_with(p: &Parser, delimiter: SyntaxKind) -> bool {
    if !p.at(SyntaxKind::IDENT) {
        return false;
    }
    let mut off = 0usize;
    while p.raw_adjacent_at(off)
        && p.nth_at(off + 1, SyntaxKind::MINUS)
        && p.raw_adjacent_at(off + 1)
        && p.nth_at(off + 2, SyntaxKind::IDENT)
    {
        off += 2;
        if off > 64 {
            return false;
        }
    }
    p.nth_at(off + 1, delimiter)
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
/// Leading `--/` node comment markers are skipped transparently.
fn looks_like_record(p: &Parser) -> bool {
    // p.nth(0) = `{`; scan starting at offset 1 (first token inside brace).
    // Skip any leading node comment markers.
    let mut off = 1usize;
    while p.nth_at(off, SyntaxKind::NODE_COMMENT) {
        off += 1;
        if off > 8 {
            return false;
        }
    }
    match p.nth(off) {
        SyntaxKind::R_BRACE => return true,
        SyntaxKind::IDENT => {}
        _ => return false,
    }
    // Scan past MINUS IDENT pairs while raw-adjacent (hyphenated field name).
    while p.raw_adjacent_at(off)          // IDENT at off is adjacent to MINUS at off+1
        && p.nth_at(off + 1, SyntaxKind::MINUS)
        && p.raw_adjacent_at(off + 1)     // MINUS at off+1 is adjacent to IDENT at off+2
        && p.nth_at(off + 2, SyntaxKind::IDENT)
    {
        off += 2;
        if off > 64 {
            return false;
        }
    }
    // After the field name, the next token must be `=`.
    p.nth_at(off + 1, SyntaxKind::EQ)
}

fn record_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACE);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACE) {
        if p.at(SyntaxKind::NODE_COMMENT) {
            let cm = p.start();
            p.bump(SyntaxKind::NODE_COMMENT);
            value_field(p);
            cm.complete(p, SyntaxKind::NODE_COMMENT_NODE);
        } else {
            value_field(p);
        }
    }
    p.expect(SyntaxKind::R_BRACE);
    m.complete(p, SyntaxKind::RECORD_EXPR)
}

fn value_field(p: &mut Parser) -> CompletedMarker {
    use crate::token_set::TokenSet;
    const VALUE_FIELD_RECOVERY: TokenSet =
        TokenSet::new(&[SyntaxKind::SEMI, SyntaxKind::R_BRACE, SyntaxKind::EOF]);
    let m = p.start();
    field_name(p);
    if !p.eat(SyntaxKind::EQ) {
        p.error("expected '=' in record field");
    }
    if super::exprs::expr_bp(p, 0, Ctx::Expr).is_none() {
        p.error("expected expression for field value");
    }
    // Ensure progress: skip to the next `;` or `}` if we didn't find a `;`.
    // Without this, wrong-separator records (e.g. `{ a = 1, b = 2 }`) loop forever.
    if !p.eat(SyntaxKind::SEMI) {
        p.err_recover("expected ';' after record field", VALUE_FIELD_RECOVERY);
        p.eat(SyntaxKind::SEMI);
    }
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
        if p.at(SyntaxKind::NODE_COMMENT) {
            let cm = p.start();
            p.bump(SyntaxKind::NODE_COMMENT);
            if !list_item(p, ctx) {
                cm.complete(p, SyntaxKind::NODE_COMMENT_NODE);
                break;
            }
            cm.complete(p, SyntaxKind::NODE_COMMENT_NODE);
        } else if !list_item(p, ctx) {
            break;
        }
    }
    p.expect(SyntaxKind::R_BRACK);
    m.complete(p, SyntaxKind::LIST_EXPR)
}

/// Returns `true` on success, `false` when no expression could be parsed
/// (caller should break the enclosing loop to avoid infinite progress stall).
fn list_item(p: &mut Parser, ctx: Ctx) -> bool {
    let im = p.start();
    if super::exprs::expr_bp(p, 0, ctx).is_none() {
        p.error("expected expression in list");
        im.abandon(p);
        return false;
    }
    p.expect(SyntaxKind::SEMI);
    im.complete(p, SyntaxKind::LIST_ITEM);
    true
}

// ── Lambda expression ─────────────────────────────────────────────────────────

fn lambda_expr(p: &mut Parser, ctx: Ctx) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::BACKSLASH);
    // Consume space-separated patterns until `=>` or `{`.
    // The `!p.at(L_BRACE)` guard is kept intentionally: it lets `\x { … }` (block body) work
    // without trying to parse `{` as a record pattern. A record pattern as the first lambda
    // param is grammatically ambiguous with the block-body form and is not used in any fixture.
    while !p.at_eof() && !p.at(SyntaxKind::FAT_ARROW) && !p.at(SyntaxKind::L_BRACE) {
        if super::patterns::pattern(p).is_none() {
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
    use crate::token_set::MATCH_CASE_RECOVERY;
    let m = p.start();
    if super::patterns::pattern(p).is_none() {
        p.err_recover(
            format!("expected pattern in match case, got {:?}", p.current()),
            MATCH_CASE_RECOVERY,
        );
    }
    if p.at(SyntaxKind::KW_IF) {
        let gm = p.start();
        p.bump(SyntaxKind::KW_IF);
        if super::exprs::expr(p).is_none() {
            p.error("expected expression in match guard");
        }
        gm.complete(p, SyntaxKind::GUARD);
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
