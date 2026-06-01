use zutai_syntax::SyntaxKind;
use zutai_syntax::SyntaxNode;
use zutai_syntax::ast::tokens::{decode_atom, decode_float, decode_int, decode_string};

use crate::pat::HirPatId;
use crate::pat::HirPatKind;
use crate::symbol::SymbolKind;
use crate::ty::LitVal;

use super::ctx::LowerCtx;
use super::{LitClass, classify_literal};

/// Lower a pattern CST node to a `HirPatId`.
///
/// Introduces binding patterns into the current scope as `SymbolKind::Local`.
pub(crate) fn lower_pat(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirPatId {
    let range = node.text_range();
    match node.kind() {
        SyntaxKind::WILDCARD_PATTERN => ctx.alloc_pat(HirPatKind::Wildcard, range),

        SyntaxKind::LITERAL => lower_literal_pat(ctx, node),

        SyntaxKind::TUPLE_PATTERN => lower_tuple_pat(ctx, node),

        SyntaxKind::RECORD_PATTERN => lower_record_pat(ctx, node),

        _ => ctx.error_pat(range),
    }
}

fn lower_literal_pat(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirPatId {
    let range = node.text_range();
    let Some(cls) = classify_literal(node) else {
        return ctx.error_pat(range);
    };
    match cls {
        LitClass::NameRef => {
            // Identifier in pattern position → binding
            let name = ident_text(node);
            let sym_id = ctx.define_sym(name, SymbolKind::Local, range);
            ctx.alloc_pat(HirPatKind::Bind(sym_id), range)
        }
        LitClass::Wildcard => ctx.alloc_pat(HirPatKind::Wildcard, range),
        LitClass::Int => {
            let val = int_val(node);
            ctx.alloc_pat(HirPatKind::Literal(LitVal::Int(val)), range)
        }
        LitClass::Float => {
            let val = float_val(node);
            ctx.alloc_pat(HirPatKind::Literal(LitVal::Float(val)), range)
        }
        LitClass::Str => {
            let val = string_val(node);
            ctx.alloc_pat(HirPatKind::Literal(LitVal::Text(val)), range)
        }
        LitClass::Atom => {
            let val = atom_val(node);
            ctx.alloc_pat(HirPatKind::Literal(LitVal::Atom(val)), range)
        }
        LitClass::Bool => {
            let is_true = node.text().to_string().trim() == "true";
            ctx.alloc_pat(HirPatKind::Literal(LitVal::Bool(is_true)), range)
        }
        LitClass::NoneLit => ctx.alloc_pat(HirPatKind::Literal(LitVal::None), range),
    }
}

fn lower_tuple_pat(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirPatId {
    let range = node.text_range();
    // TUPLE_PATTERN: L_PAREN ATOM (COMMA PATTERN_FIELD)* R_PAREN
    let tag = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::ATOM)
        .map(|t| t.text().trim_start_matches('#').to_string())
        .unwrap_or_default();
    let mut fields = Vec::new();
    for child in node.children() {
        if child.kind() == SyntaxKind::PATTERN_FIELD {
            let fname = field_name_text(&child);
            let pat_id = child
                .children()
                .find(|c| c.kind() != SyntaxKind::FIELD_NAME)
                .map(|n| lower_pat(ctx, &n))
                .unwrap_or_else(|| ctx.error_pat(range));
            fields.push((fname, pat_id));
        }
    }
    ctx.alloc_pat(HirPatKind::Variant { tag, fields }, range)
}

fn lower_record_pat(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirPatId {
    let range = node.text_range();
    let mut fields = Vec::new();
    for child in node.children() {
        if child.kind() == SyntaxKind::PATTERN_FIELD {
            let fname = field_name_text(&child);
            let pat_id = child
                .children()
                .find(|c| c.kind() != SyntaxKind::FIELD_NAME)
                .map(|n| lower_pat(ctx, &n))
                .unwrap_or_else(|| ctx.error_pat(range));
            fields.push((fname, pat_id));
        }
    }
    ctx.alloc_pat(HirPatKind::Record { fields }, range)
}

// ── Token decoders ────────────────────────────────────────────────────────────

fn ident_text(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::IDENT)
        .map(|t| t.text().to_string())
        .unwrap_or_default()
}

fn int_val(node: &SyntaxNode) -> i64 {
    let negative = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::MINUS);
    let raw = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::INT)
        .and_then(|t| decode_int(&t))
        .unwrap_or(0);
    if negative { -raw } else { raw }
}

fn float_val(node: &SyntaxNode) -> f64 {
    let negative = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::MINUS);
    let raw = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::FLOAT)
        .and_then(|t| decode_float(&t))
        .unwrap_or(0.0);
    if negative { -raw } else { raw }
}

fn string_val(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::STRING)
        .and_then(|t| decode_string(&t))
        .unwrap_or_default()
}

fn atom_val(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::ATOM)
        .map(|t| decode_atom(&t).to_string())
        .unwrap_or_default()
}

fn field_name_text(node: &SyntaxNode) -> String {
    node.children()
        .find(|c| c.kind() == SyntaxKind::FIELD_NAME)
        .map(|n| n.text().to_string())
        .unwrap_or_default()
}
