use crate::{SyntaxKind, token_set::TokenSet};

use super::{
    super::{CompletedMarker, Parser},
    exprs::type_expr,
    primary::field_name,
};

// Recovery set shared by both type_field and variant_field.
const TYPE_FIELD_RECOVERY: TokenSet =
    TokenSet::new(&[SyntaxKind::SEMI, SyntaxKind::R_BRACE, SyntaxKind::EOF]);

const TYPE_UNION_ITEM_RECOVERY: TokenSet =
    TokenSet::new(&[SyntaxKind::SEMI, SyntaxKind::R_BRACK, SyntaxKind::EOF]);

// ── Entry: `type { … }` / `type [ … ]` ───────────────────────────────────────

/// Parse `type { TypeField* }` or `type [ TypeUnionItem* ]` → TYPE_FORM.
pub(super) fn type_form(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::KW_TYPE);
    match p.current() {
        SyntaxKind::L_BRACE => {
            type_record_inner(p);
        }
        SyntaxKind::L_BRACK => {
            type_union_inner(p);
        }
        _ => {
            p.error("expected '{' or '[' after 'type'");
        }
    }
    m.complete(p, SyntaxKind::TYPE_FORM)
}

// ── Type record ───────────────────────────────────────────────────────────────

/// Parse `{ TypeField* }` → TYPE_RECORD. Called from type context `{` dispatch
/// and from `type_form`. The `{` has not been consumed yet.
pub(super) fn type_record_inner(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACE);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACE) {
        type_field(p);
    }
    p.expect(SyntaxKind::R_BRACE);
    m.complete(p, SyntaxKind::TYPE_RECORD)
}

fn type_field(p: &mut Parser) {
    let m = p.start();
    field_name(p);
    // Optional-field marker: `field? : Type` — the `?` attaches to the field name.
    p.eat(SyntaxKind::QUESTION);
    if !p.eat(SyntaxKind::COLON) {
        p.error("expected ':' in type field");
    }
    if type_expr(p).is_none() {
        p.error("expected type expression for field type");
    }
    // Ensure progress: skip to the next `;` or `}` if we didn't find a `;`.
    // Without this, mismatched sigils (e.g. `field = Type`) loop forever.
    if !p.eat(SyntaxKind::SEMI) {
        p.err_recover("expected ';' after type field", TYPE_FIELD_RECOVERY);
        p.eat(SyntaxKind::SEMI);
    }
    m.complete(p, SyntaxKind::TYPE_FIELD);
}

// ── Type union ────────────────────────────────────────────────────────────────

/// Parse `[ TypeUnionItem* ]` → TYPE_UNION. Called from type context `[` dispatch
/// and from `type_form`. The `[` has not been consumed yet.
pub(super) fn type_union_inner(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACK);
    while !p.at_eof() && !p.at(SyntaxKind::R_BRACK) {
        let im = p.start();
        if type_expr(p).is_none() {
            im.abandon(p);
            p.err_recover(
                format!("expected type expression in union, got {:?}", p.current()),
                TYPE_UNION_ITEM_RECOVERY,
            );
            p.eat(SyntaxKind::SEMI);
            continue;
        }
        // Ensure progress for comma-separated (wrong separator) unions.
        if !p.eat(SyntaxKind::SEMI) {
            p.err_recover("expected ';' after union item", TYPE_UNION_ITEM_RECOVERY);
            p.eat(SyntaxKind::SEMI);
        }
        im.complete(p, SyntaxKind::TYPE_UNION_ITEM);
    }
    p.expect(SyntaxKind::R_BRACK);
    m.complete(p, SyntaxKind::TYPE_UNION)
}
