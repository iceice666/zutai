use zutai_syntax::SyntaxKind;
use zutai_syntax::SyntaxNode;
use zutai_syntax::ast::operators::BinaryOp as SynBinOp;
use zutai_syntax::ast::tokens::{decode_atom, decode_float, decode_int, decode_string};

use crate::expr::{BinaryOp, HirArm, HirExprId, HirExprKind, ImportKind};
use crate::symbol::SymbolKind;
use crate::ty::LitVal;

use super::ctx::LowerCtx;
use super::pat::lower_pat;
use super::ty::lower_type;
use super::{LitClass, classify_literal};

/// Lower an expression-position CST node to a `HirExprId`.
pub(crate) fn lower_expr(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    match node.kind() {
        SyntaxKind::LITERAL => lower_literal_expr(ctx, node),
        SyntaxKind::PAREN_EXPR => lower_paren(ctx, node),
        SyntaxKind::TUPLE_EXPR => lower_tuple_expr(ctx, node),
        SyntaxKind::RECORD_EXPR => lower_record_expr(ctx, node),
        SyntaxKind::LIST_EXPR => lower_list_expr(ctx, node),
        SyntaxKind::LAMBDA_EXPR => lower_lambda(ctx, node),
        SyntaxKind::MATCH_EXPR => lower_match(ctx, node),
        SyntaxKind::IF_EXPR => lower_if(ctx, node),
        SyntaxKind::IMPORT_EXPR => lower_import(ctx, node),
        SyntaxKind::CALL_EXPR => lower_call(ctx, node),
        SyntaxKind::ACCESS_EXPR => lower_access(ctx, node),
        SyntaxKind::OPTIONAL_ACCESS_EXPR => lower_optional_access(ctx, node),
        SyntaxKind::BINARY_EXPR => lower_binary(ctx, node),
        SyntaxKind::PIPELINE_EXPR => lower_pipeline(ctx, node),
        SyntaxKind::BLOCK => lower_block(ctx, node),
        SyntaxKind::TYPE_FORM => {
            let ty_id = lower_type(ctx, node);
            let err = ctx.alloc_expr(HirExprKind::Error, range);
            ctx.alloc_expr(
                HirExprKind::Annot {
                    expr: err,
                    ty: ty_id,
                },
                range,
            )
        }
        _ => ctx.error_expr(range),
    }
}

// ── Literals ──────────────────────────────────────────────────────────────────

fn lower_literal_expr(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let Some(cls) = classify_literal(node) else {
        return ctx.error_expr(range);
    };
    match cls {
        LitClass::NameRef => {
            let name = ident_text(node);
            let sym_id = ctx.resolve_name(&name, range);
            ctx.alloc_expr(HirExprKind::Var(sym_id), range)
        }
        LitClass::Wildcard => {
            // Wildcard in expression position is meaningless; lower as Error
            ctx.error_expr(range)
        }
        LitClass::Int => {
            let val = int_val(node);
            ctx.alloc_expr(HirExprKind::Lit(LitVal::Int(val)), range)
        }
        LitClass::Float => {
            let val = float_val(node);
            ctx.alloc_expr(HirExprKind::Lit(LitVal::Float(val)), range)
        }
        LitClass::Str => {
            let val = string_val(node);
            ctx.alloc_expr(HirExprKind::Lit(LitVal::Text(val)), range)
        }
        LitClass::Atom => {
            let val = atom_val(node);
            ctx.alloc_expr(HirExprKind::Lit(LitVal::Atom(val)), range)
        }
        LitClass::Bool => {
            let is_true = node.text().to_string().trim() == "true";
            ctx.alloc_expr(HirExprKind::Lit(LitVal::Bool(is_true)), range)
        }
        LitClass::NoneLit => ctx.alloc_expr(HirExprKind::Lit(LitVal::None), range),
    }
}

// ── Parenthesised expression ──────────────────────────────────────────────────

fn lower_paren(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let inner = expr_children(node).next();
    match inner {
        Some(inner) => lower_expr(ctx, &inner),
        None => ctx.error_expr(range),
    }
}

// ── Tuple / variant construction ──────────────────────────────────────────────

fn lower_tuple_expr(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    // First child should be a TUPLE_ITEM containing an atom (the tag).
    // Subsequent VALUE_FIELD children are named fields.
    let mut children = node.children();
    let first = children.next();
    let Some(first_node) = first else {
        return ctx.error_expr(range);
    };

    // Extract the atom tag from the first TUPLE_ITEM
    let tag = if first_node.kind() == SyntaxKind::TUPLE_ITEM {
        let atom_tok = first_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::ATOM);
        match atom_tok {
            Some(t) => t.text().trim_start_matches('#').to_string(),
            None => {
                // No atom tag — plain tuple without tag; not valid in Zutai v0
                return ctx.error_expr(range);
            }
        }
    } else {
        return ctx.error_expr(range);
    };

    // Remaining children are VALUE_FIELD nodes
    let mut fields = Vec::new();
    for child in children {
        if child.kind() == SyntaxKind::VALUE_FIELD {
            let fname = field_name_text(&child);
            let val_node = child
                .children()
                .find(|c| c.kind() != SyntaxKind::FIELD_NAME);
            let val_id = match val_node {
                Some(n) => lower_expr(ctx, &n),
                None => ctx.error_expr(child.text_range()),
            };
            fields.push((fname, val_id));
        }
    }
    ctx.alloc_expr(HirExprKind::Variant { tag, fields }, range)
}

// ── Record ────────────────────────────────────────────────────────────────────

fn lower_record_expr(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut fields = Vec::new();
    for child in node.children() {
        let child = skip_node_comment(child);
        if child.kind() == SyntaxKind::VALUE_FIELD {
            let fname = field_name_text(&child);
            let val_node = child
                .children()
                .find(|c| c.kind() != SyntaxKind::FIELD_NAME);
            let val_id = match val_node {
                Some(n) => lower_expr(ctx, &n),
                None => ctx.error_expr(child.text_range()),
            };
            fields.push((fname, val_id));
        }
    }
    ctx.alloc_expr(HirExprKind::Record { fields }, range)
}

// ── List ──────────────────────────────────────────────────────────────────────

fn lower_list_expr(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut items = Vec::new();
    for child in node.children() {
        let child = skip_node_comment(child);
        if child.kind() == SyntaxKind::LIST_ITEM {
            let item_node = child.children().next();
            let item_id = match item_node {
                Some(n) => lower_expr(ctx, &n),
                None => ctx.error_expr(child.text_range()),
            };
            items.push(item_id);
        }
    }
    ctx.alloc_expr(HirExprKind::List { items }, range)
}

// ── Lambda ────────────────────────────────────────────────────────────────────

pub(crate) fn lower_lambda(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    ctx.scopes.push_child();

    // Collect patterns (all Pattern-kind children before the body node)
    let mut params = Vec::new();
    let mut body_node: Option<SyntaxNode> = None;

    for child in node.children() {
        use SyntaxKind::*;
        match child.kind() {
            WILDCARD_PATTERN | LITERAL | TUPLE_PATTERN | RECORD_PATTERN => {
                let pat_id = lower_pat(ctx, &child);
                params.push(pat_id);
            }
            // Body: either a BLOCK or any expression (after FAT_ARROW)
            BLOCK => {
                body_node = Some(child);
                break;
            }
            kind if is_expr_kind(kind) => {
                body_node = Some(child);
                break;
            }
            _ => {}
        }
    }

    let body = match body_node {
        Some(n) => lower_expr(ctx, &n),
        None => ctx.error_expr(range),
    };
    ctx.scopes.pop();

    ctx.alloc_expr(HirExprKind::Lambda { params, body }, range)
}

// ── Match ─────────────────────────────────────────────────────────────────────

fn lower_match(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut exprs = expr_children(node);
    let scrutinee = match exprs.next() {
        Some(n) => lower_expr(ctx, &n),
        None => ctx.error_expr(range),
    };
    let arms = node
        .children()
        .filter(|c| c.kind() == SyntaxKind::MATCH_CASE)
        .map(|c| lower_match_case(ctx, &c))
        .collect();
    ctx.alloc_expr(HirExprKind::Match { scrutinee, arms }, range)
}

fn lower_match_case(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirArm {
    let range = node.text_range();
    ctx.scopes.push_child();

    // Children: Pattern, optional GUARD, then body Expr
    let pat_node = node.children().find(|c| is_pat_kind(c.kind()));
    let pat = match pat_node {
        Some(n) => lower_pat(ctx, &n),
        None => ctx.error_pat(range),
    };

    let guard = node
        .children()
        .find(|c| c.kind() == SyntaxKind::GUARD)
        .and_then(|g| g.children().next())
        .map(|cond| lower_expr(ctx, &cond));

    let body_node = node.children().find(|c| is_expr_kind(c.kind()));
    let body = match body_node {
        Some(n) => lower_expr(ctx, &n),
        None => ctx.error_expr(range),
    };

    ctx.scopes.pop();
    HirArm { pat, guard, body }
}

// ── If ────────────────────────────────────────────────────────────────────────

fn lower_if(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut exprs = expr_children(node);
    let cond = exprs
        .next()
        .map(|n| lower_expr(ctx, &n))
        .unwrap_or_else(|| ctx.error_expr(range));
    let then_ = exprs
        .next()
        .map(|n| lower_expr(ctx, &n))
        .unwrap_or_else(|| ctx.error_expr(range));
    let else_ = exprs
        .next()
        .map(|n| lower_expr(ctx, &n))
        .unwrap_or_else(|| ctx.error_expr(range));
    ctx.alloc_expr(HirExprKind::If { cond, then_, else_ }, range)
}

// ── Import ────────────────────────────────────────────────────────────────────

fn lower_import(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    // IMPORT_EXPR: KW_IMPORT IMPORT_PATH(STRING)
    let path_node = node
        .children()
        .find(|c| c.kind() == SyntaxKind::IMPORT_PATH);
    let path = path_node
        .and_then(|n| {
            n.children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == SyntaxKind::STRING)
                .and_then(|t| decode_string(&t))
        })
        .unwrap_or_default();
    let kind = if path.ends_with(".zti") {
        ImportKind::Zti
    } else {
        ImportKind::Zt
    };
    ctx.alloc_expr(HirExprKind::Import { path, kind }, range)
}

// ── Application ───────────────────────────────────────────────────────────────

fn lower_call(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut exprs = expr_children(node);
    let fun = exprs
        .next()
        .map(|n| lower_expr(ctx, &n))
        .unwrap_or_else(|| ctx.error_expr(range));
    let arg = exprs
        .next()
        .map(|n| lower_expr(ctx, &n))
        .unwrap_or_else(|| ctx.error_expr(range));
    ctx.alloc_expr(HirExprKind::Apply { fun, arg }, range)
}

// ── Field access ──────────────────────────────────────────────────────────────

fn lower_access(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let value_node = expr_children(node).next();
    let value = match value_node {
        Some(n) => lower_expr(ctx, &n),
        None => ctx.error_expr(range),
    };
    let label = field_name_text(node);
    ctx.alloc_expr(HirExprKind::Field { value, label }, range)
}

/// Sugar: `e?.field` → `match e { none => none; v => v.field; }`
fn lower_optional_access(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let value_node = expr_children(node).next();
    let scrutinee = match value_node {
        Some(n) => lower_expr(ctx, &n),
        None => return ctx.error_expr(range),
    };
    let label = field_name_text(node);

    // Build: match scrutinee { none => none; __v => __v.field }
    let none_pat = ctx.alloc_pat(crate::pat::HirPatKind::Literal(LitVal::None), range);
    let none_expr = ctx.alloc_expr(HirExprKind::Lit(LitVal::None), range);

    let sym_id = ctx.define_sym("__opt_chain".to_string(), SymbolKind::Local, range);
    let bind_pat = ctx.alloc_pat(crate::pat::HirPatKind::Bind(sym_id), range);
    let var_expr = ctx.alloc_expr(HirExprKind::Var(sym_id), range);
    let field_expr = ctx.alloc_expr(
        HirExprKind::Field {
            value: var_expr,
            label,
        },
        range,
    );

    let arms = vec![
        HirArm {
            pat: none_pat,
            guard: None,
            body: none_expr,
        },
        HirArm {
            pat: bind_pat,
            guard: None,
            body: field_expr,
        },
    ];
    ctx.alloc_expr(HirExprKind::Match { scrutinee, arms }, range)
}

// ── Binary operators + sugar ──────────────────────────────────────────────────

fn lower_binary(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut exprs = expr_children(node);
    let lhs_node = exprs.next();
    let rhs_node = exprs.next();

    // Determine the operator from the token
    let op_kind = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
        .filter(|t| {
            use SyntaxKind::*;
            matches!(
                t.kind(),
                PLUS | MINUS
                    | STAR
                    | SLASH
                    | EQ_EQ
                    | BANG_EQ
                    | LT
                    | LT_EQ
                    | GT
                    | GT_EQ
                    | AMP_AMP
                    | PIPE_PIPE
                    | QUESTION_QUESTION
            )
        });

    let syn_op = op_kind.and_then(|t| SynBinOp::from_kind(t.kind()));

    match syn_op {
        // `??` desugars to Match: `a ?? b` → `match a { none => b; v => v }`
        Some(SynBinOp::Coalesce) => {
            let scrutinee = lhs_node
                .map(|n| lower_expr(ctx, &n))
                .unwrap_or_else(|| ctx.error_expr(range));
            let default_expr = rhs_node
                .map(|n| lower_expr(ctx, &n))
                .unwrap_or_else(|| ctx.error_expr(range));

            let none_pat = ctx.alloc_pat(crate::pat::HirPatKind::Literal(LitVal::None), range);
            let sym_id = ctx.define_sym("__coal".to_string(), SymbolKind::Local, range);
            let bind_pat = ctx.alloc_pat(crate::pat::HirPatKind::Bind(sym_id), range);
            let var_expr = ctx.alloc_expr(HirExprKind::Var(sym_id), range);

            let arms = vec![
                HirArm {
                    pat: none_pat,
                    guard: None,
                    body: default_expr,
                },
                HirArm {
                    pat: bind_pat,
                    guard: None,
                    body: var_expr,
                },
            ];
            ctx.alloc_expr(HirExprKind::Match { scrutinee, arms }, range)
        }
        Some(syn_op) => {
            // All other operators → BinOp node
            let hir_op = BinaryOp::from_syntax(syn_op).unwrap();
            let lhs = lhs_node
                .map(|n| lower_expr(ctx, &n))
                .unwrap_or_else(|| ctx.error_expr(range));
            let rhs = rhs_node
                .map(|n| lower_expr(ctx, &n))
                .unwrap_or_else(|| ctx.error_expr(range));
            ctx.alloc_expr(
                HirExprKind::BinOp {
                    op: hir_op,
                    lhs,
                    rhs,
                },
                range,
            )
        }
        None => ctx.error_expr(range),
    }
}

// ── Pipeline (sugar) ──────────────────────────────────────────────────────────

/// Sugar: `e |> f` → `Apply(f, e)`;  `f <| e` → `Apply(f, e)`.
fn lower_pipeline(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    let mut exprs: Vec<_> = expr_children(node).collect();
    if exprs.len() < 2 {
        return ctx.error_expr(range);
    }
    let is_forward = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::PIPE_ARROW);

    let (lhs_node, rhs_node) = (exprs.remove(0), exprs.remove(0));
    let (fun_node, arg_node) = if is_forward {
        (rhs_node, lhs_node) // `e |> f` → f(e)
    } else {
        (lhs_node, rhs_node) // `f <| e` → f(e)
    };
    let fun = lower_expr(ctx, &fun_node);
    let arg = lower_expr(ctx, &arg_node);
    ctx.alloc_expr(HirExprKind::Apply { fun, arg }, range)
}

// ── Block ─────────────────────────────────────────────────────────────────────

pub(crate) fn lower_block(ctx: &mut LowerCtx, node: &SyntaxNode) -> HirExprId {
    let range = node.text_range();
    // BLOCK: LOCAL_BINDING* final_Expr
    let bindings: Vec<_> = node
        .children()
        .filter(|c| c.kind() == SyntaxKind::LOCAL_BINDING)
        .collect();
    let final_node = node.children().find(|c| is_expr_kind(c.kind()));

    if bindings.is_empty() {
        return match final_node {
            Some(n) => lower_expr(ctx, &n),
            None => ctx.error_expr(range),
        };
    }

    // Sequential bindings: each binding's value is lowered before the name is
    // visible (no self-reference within the RHS).
    ctx.scopes.push_child();
    let result = lower_block_bindings(ctx, &bindings, final_node.as_ref(), range);
    ctx.scopes.pop();
    result
}

fn lower_block_bindings(
    ctx: &mut LowerCtx,
    bindings: &[SyntaxNode],
    final_node: Option<&SyntaxNode>,
    fallback_range: text_size::TextRange,
) -> HirExprId {
    if bindings.is_empty() {
        return match final_node {
            Some(n) => lower_expr(ctx, &n),
            None => ctx.error_expr(fallback_range),
        };
    }
    let binding = &bindings[0];
    let rest = &bindings[1..];
    let range = binding.text_range();

    let name_tok = binding
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::IDENT);
    let Some(name_tok) = name_tok else {
        return lower_block_bindings(ctx, rest, final_node, fallback_range);
    };
    let name = name_tok.text().to_string();
    let name_range = name_tok.text_range();

    // Lower value BEFORE defining name (sequential semantics)
    let value_node = binding.children().find(|c| is_expr_kind(c.kind()));
    let value = match value_node {
        Some(n) => lower_expr(ctx, &n),
        None => ctx.error_expr(range),
    };

    let sym_id = ctx.define_sym(name, SymbolKind::Local, name_range);
    let body = lower_block_bindings(ctx, rest, final_node, fallback_range);

    ctx.alloc_expr(
        HirExprKind::Let {
            name: sym_id,
            ty: None,
            value,
            body,
        },
        range,
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn expr_children(node: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> + '_ {
    node.children().filter(|c| is_expr_kind(c.kind()))
}

pub(crate) fn is_expr_kind_pub(kind: SyntaxKind) -> bool {
    is_expr_kind(kind)
}

fn is_expr_kind(kind: SyntaxKind) -> bool {
    use SyntaxKind::*;
    matches!(
        kind,
        LITERAL
            | PAREN_EXPR
            | TUPLE_EXPR
            | RECORD_EXPR
            | LIST_EXPR
            | LAMBDA_EXPR
            | MATCH_EXPR
            | IF_EXPR
            | IMPORT_EXPR
            | CALL_EXPR
            | ACCESS_EXPR
            | OPTIONAL_ACCESS_EXPR
            | BINARY_EXPR
            | PIPELINE_EXPR
            | BLOCK
            | TYPE_FORM
    )
}

fn is_pat_kind(kind: SyntaxKind) -> bool {
    use SyntaxKind::*;
    matches!(
        kind,
        WILDCARD_PATTERN | LITERAL | TUPLE_PATTERN | RECORD_PATTERN
    )
}

fn skip_node_comment(node: SyntaxNode) -> SyntaxNode {
    if node.kind() == SyntaxKind::NODE_COMMENT_NODE {
        node.children().next().unwrap_or(node)
    } else {
        node
    }
}

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
