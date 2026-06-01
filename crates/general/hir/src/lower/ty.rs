use zutai_syntax::SyntaxKind;
use zutai_syntax::SyntaxNode;
use zutai_syntax::ast::{AstNode, nodes::Expr};

use crate::ty::{FieldKind, HirTypeId, HirTypeKind, LitVal};

use super::classify_literal;
use super::ctx::LowerCtx;

/// Lower a syntax node in type position to a `HirTypeId`.
///
/// The type node may be any of: TYPE_FORM (wrapper), TYPE_RECORD, TYPE_UNION,
/// VARIANT_TYPE, OPTIONAL_TYPE, FUNCTION_TYPE, CALL_EXPR (type app), or a LITERAL
/// (named type variable like `Int` or a type param `A`).
pub(crate) fn lower_type(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirTypeId {
    let range = node.text_range();
    match node.kind() {
        SyntaxKind::TYPE_FORM => {
            // TYPE_FORM wraps the actual type expression
            if let Some(inner) = node.children().next() {
                lower_type(ctx, &inner)
            } else {
                ctx.error_type(range)
            }
        }

        SyntaxKind::TYPE_RECORD => lower_type_record(ctx, node),

        SyntaxKind::TYPE_UNION => lower_type_union(ctx, node),

        SyntaxKind::VARIANT_TYPE => lower_variant_type(ctx, node),

        SyntaxKind::OPTIONAL_TYPE => {
            // `T?` — the child is the wrapped type expression
            if let Some(inner) = node.children().next() {
                let inner_id = lower_type(ctx, &inner);
                ctx.alloc_type(HirTypeKind::Optional(inner_id), range)
            } else {
                ctx.error_type(range)
            }
        }

        SyntaxKind::FUNCTION_TYPE => {
            // `A -> B` — two Expr children (type position)
            let mut children = node.children();
            let param = children
                .next()
                .map(|n| lower_type(ctx, &n))
                .unwrap_or_else(|| ctx.error_type(range));
            let ret = children
                .next()
                .map(|n| lower_type(ctx, &n))
                .unwrap_or_else(|| ctx.error_type(range));
            ctx.alloc_type(HirTypeKind::Function { param, ret }, range)
        }

        SyntaxKind::CALL_EXPR => {
            // Type application: `List T` or `Pair A B`
            let mut exprs = node.children().filter_map(|n| {
                if Expr::can_cast(n.kind()) {
                    Some(n)
                } else {
                    None
                }
            });
            let ctor = exprs
                .next()
                .map(|n| lower_type(ctx, &n))
                .unwrap_or_else(|| ctx.error_type(range));
            let arg = exprs
                .next()
                .map(|n| lower_type(ctx, &n))
                .unwrap_or_else(|| ctx.error_type(range));
            ctx.alloc_type(HirTypeKind::Apply { ctor, arg }, range)
        }

        SyntaxKind::LITERAL => {
            // Named type reference: `Int`, `Bool`, `Text`, type param `A`, etc.
            if let Some(cls) = classify_literal(node) {
                use super::LitClass;
                match cls {
                    LitClass::NameRef => {
                        let name = node.text().to_string().trim().to_string();
                        let sym_id = ctx.resolve_name(&name, range);
                        ctx.alloc_type(HirTypeKind::Var(sym_id), range)
                    }
                    LitClass::Atom => {
                        let atom = atom_text(node);
                        ctx.alloc_type(HirTypeKind::SingletonAtom(atom), range)
                    }
                    LitClass::Bool => {
                        let is_true = node.text().to_string().trim() == "true";
                        ctx.alloc_type(HirTypeKind::SingletonLit(LitVal::Bool(is_true)), range)
                    }
                    LitClass::NoneLit => {
                        ctx.alloc_type(HirTypeKind::SingletonLit(LitVal::None), range)
                    }
                    _ => ctx.error_type(range),
                }
            } else {
                ctx.error_type(range)
            }
        }

        SyntaxKind::PAREN_EXPR => {
            // Parenthesised type expression
            if let Some(inner) = node.children().next() {
                lower_type(ctx, &inner)
            } else {
                ctx.error_type(range)
            }
        }

        SyntaxKind::TUPLE_EXPR => {
            // `(#tag, field : T, ...)` as a type-position variant
            lower_variant_type_from_tuple(ctx, node)
        }

        _ => ctx.error_type(range),
    }
}

fn lower_type_record(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirTypeId {
    let range = node.text_range();
    let mut fields = Vec::new();
    for child in node.children() {
        if child.kind() == SyntaxKind::TYPE_FIELD {
            let (name, ty_id, fk) = lower_type_field(ctx, &child);
            fields.push((name, ty_id, fk));
        }
    }
    ctx.alloc_type(HirTypeKind::Record { fields }, range)
}

fn lower_type_field(ctx: &mut LowerCtx, node: &SyntaxNode) -> (String, HirTypeId, FieldKind) {
    let range = node.text_range();
    // TYPE_FIELD: FIELD_NAME QUESTION? COLON TypeExpr
    let name = field_name_text(node);
    let optional = node.children_with_tokens().any(|e| {
        e.as_token()
            .map(|t| t.kind() == SyntaxKind::QUESTION)
            .unwrap_or(false)
    });
    let fk = if optional {
        FieldKind::Optional
    } else {
        FieldKind::Required
    };
    let ty_id = node
        .children()
        .find(|c| c.kind() != SyntaxKind::FIELD_NAME)
        .map(|n| lower_type(ctx, &n))
        .unwrap_or_else(|| ctx.error_type(range));
    (name, ty_id, fk)
}

fn lower_type_union(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirTypeId {
    let range = node.text_range();
    let mut variants = Vec::new();
    for child in node.children() {
        if child.kind() == SyntaxKind::TYPE_UNION_ITEM {
            // Each union item wraps one type expression
            if let Some(inner) = child.children().next() {
                let ty_id = lower_type(ctx, &inner);
                variants.push(ty_id);
            }
        }
    }
    ctx.alloc_type(HirTypeKind::Union { variants }, range)
}

fn lower_variant_type(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirTypeId {
    let range = node.text_range();
    // VARIANT_TYPE: L_PAREN ATOM (COMMA VARIANT_FIELD)* R_PAREN
    let tag = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::ATOM)
        .map(|t| t.text().trim_start_matches('#').to_string())
        .unwrap_or_default();
    let mut fields = Vec::new();
    for child in node.children() {
        if child.kind() == SyntaxKind::VARIANT_FIELD {
            let name = field_name_text(&child);
            let ty_id = child
                .children()
                .find(|c| c.kind() != SyntaxKind::FIELD_NAME)
                .map(|n| lower_type(ctx, &n))
                .unwrap_or_else(|| ctx.error_type(range));
            fields.push((name, ty_id));
        }
    }
    ctx.alloc_type(HirTypeKind::Variant { tag, fields }, range)
}

fn lower_variant_type_from_tuple(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirTypeId {
    let range = node.text_range();
    // Handle `(#tag, field : T, ...)` in a type context (parsed as TUPLE_EXPR)
    // First TUPLE_ITEM: atom tag; remaining VALUE_FIELDs: named fields.
    let mut items = node.children();
    let tag = items
        .next()
        .and_then(|n| {
            if n.kind() == SyntaxKind::TUPLE_ITEM {
                n.children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::ATOM)
                    .map(|t| t.text().trim_start_matches('#').to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let mut fields = Vec::new();
    for child in items {
        if child.kind() == SyntaxKind::VALUE_FIELD {
            let name = field_name_text(&child);
            let ty_id = child
                .children()
                .find(|c| c.kind() != SyntaxKind::FIELD_NAME)
                .map(|n| lower_type(ctx, &n))
                .unwrap_or_else(|| ctx.error_type(range));
            fields.push((name, ty_id));
        }
    }
    ctx.alloc_type(HirTypeKind::Variant { tag, fields }, range)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn field_name_text(node: &SyntaxNode) -> String {
    node.children()
        .find(|c| c.kind() == SyntaxKind::FIELD_NAME)
        .map(|n| n.text().to_string())
        .unwrap_or_default()
}

fn atom_text(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::ATOM)
        .map(|t| t.text().trim_start_matches('#').to_string())
        .unwrap_or_default()
}
