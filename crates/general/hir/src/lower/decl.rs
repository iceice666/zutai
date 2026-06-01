use zutai_syntax::SyntaxKind;
use zutai_syntax::SyntaxNode;
use zutai_syntax::ast::AstNode;
use zutai_syntax::ast::nodes::{Clause, FuncDecl, TopDecl};

use crate::decl::HirDecl;
use crate::expr::{HirArm, HirExprId, HirExprKind};
use crate::pat::HirPatKind;
use crate::symbol::{SymbolId, SymbolKind};

use super::ctx::LowerCtx;
use super::expr::{lower_block, lower_expr};
use super::pat::lower_pat;
use super::ty::lower_type;

// ── File-level entry point ────────────────────────────────────────────────────

/// Lower all top-level declarations and the file's final expression.
///
/// Two-phase:
/// 1. Collect all top-level names → SymbolIds (enables mutual recursion in bodies).
/// 2. Lower each declaration body.
pub(crate) fn lower_file_decls(ctx: &mut LowerCtx, root: &SyntaxNode) {
    use zutai_syntax::ast::nodes::File;
    let file_scope = ctx.scopes.current_id();
    let Some(file_node) = File::cast(root.clone()) else {
        return;
    };

    // Pre-populate built-in type names so references like `Int`, `Text`, `Bool`
    // don't generate E0020. The zero-range sentinel marks these as built-ins.
    let builtin_range = root.text_range();
    for name in ["Type", "Int", "Float", "Text", "Bool", "List"] {
        ctx.define_sym_in(
            file_scope,
            name.to_string(),
            SymbolKind::TypeDef,
            builtin_range,
        );
    }

    // Phase 1: register all top-level names in the file scope
    for decl in file_node.decls() {
        let (name, kind, range) = decl_name_kind(&decl);
        ctx.define_sym_in(file_scope, name, kind, range);
    }

    // Phase 2: lower each body (names already in scope → mutual recursion works)
    for decl in file_node.decls() {
        if let Some(decl_id) = lower_top_decl(ctx, &decl) {
            ctx.top_decls.push(decl_id);
        }
    }

    // Find the file's final expression (last Expr child at the file level)
    let final_node = root
        .children()
        .filter(|c| super::expr::is_expr_kind_pub(c.kind()))
        .last();
    ctx.final_expr = Some(match final_node {
        Some(n) => lower_expr(ctx, &n),
        None => ctx.error_expr(root.text_range()),
    });
}

fn decl_name_kind(decl: &TopDecl) -> (String, SymbolKind, text_size::TextRange) {
    match decl {
        TopDecl::Inferred(b) => {
            let tok = b.name_token().unwrap();
            (tok.text().to_string(), SymbolKind::Value, tok.text_range())
        }
        TopDecl::Annotated(b) => {
            let tok = b.name_token().unwrap();
            (tok.text().to_string(), SymbolKind::Value, tok.text_range())
        }
        TopDecl::Func(f) => {
            let tok = f.name_token().unwrap();
            let kind = if f
                .syntax()
                .children()
                .any(|c| c.kind() == SyntaxKind::TYPE_FORM)
            {
                SymbolKind::TypeDef
            } else {
                SymbolKind::Function
            };
            (tok.text().to_string(), kind, tok.text_range())
        }
    }
}

fn lower_top_decl(ctx: &mut LowerCtx, decl: &TopDecl) -> Option<crate::decl::HirDeclId> {
    match decl {
        TopDecl::Inferred(b) => {
            let name = b.name_token()?.text().to_string();
            let sym_id = ctx.scopes.resolve(&name)?;
            let body = lower_expr(ctx, b.value()?.syntax());
            Some(ctx.alloc_decl(HirDecl::Value {
                name: sym_id,
                ty: None,
                body,
            }))
        }
        TopDecl::Annotated(b) => {
            let name = b.name_token()?.text().to_string();
            let sym_id = ctx.scopes.resolve(&name)?;
            let ty_id = b.ty().map(|ty| lower_type(ctx, ty.syntax()));
            let body = lower_expr(ctx, b.value()?.syntax());
            Some(ctx.alloc_decl(HirDecl::Value {
                name: sym_id,
                ty: ty_id,
                body,
            }))
        }
        TopDecl::Func(f) => lower_func_decl(ctx, f),
    }
}

fn lower_func_decl(ctx: &mut LowerCtx, f: &FuncDecl) -> Option<crate::decl::HirDeclId> {
    let name = f.name_token()?.text().to_string();
    let sym_id = ctx.scopes.resolve(&name)?;
    let range = f.syntax().text_range();

    // Type definition: FUNC_DECL with a TYPE_FORM child, no clauses
    if f.syntax()
        .children()
        .any(|c| c.kind() == SyntaxKind::TYPE_FORM)
    {
        ctx.scopes.push_child();
        let type_params = lower_type_params(ctx, f);
        let body = f
            .syntax()
            .children()
            .find(|c| c.kind() == SyntaxKind::TYPE_FORM)
            .map(|n| lower_type(ctx, &n))
            .unwrap_or_else(|| ctx.error_type(range));
        ctx.scopes.pop();
        return Some(ctx.alloc_decl(HirDecl::TypeDef {
            name: sym_id,
            type_params,
            body,
        }));
    }

    // Function declaration
    ctx.scopes.push_child();
    let type_params = lower_type_params(ctx, f);
    // Type signature: first non-clause, non-param child in type position
    let sig = f
        .syntax()
        .children()
        .find(|c| is_type_sig_node(c))
        .map(|n| lower_type(ctx, &n));
    ctx.scopes.pop();

    let clauses: Vec<_> = f.clauses().collect();
    let body = lower_clauses(ctx, &clauses, range);
    Some(ctx.alloc_decl(HirDecl::Function {
        name: sym_id,
        type_params,
        sig,
        body,
    }))
}

fn lower_type_params(ctx: &mut LowerCtx, f: &FuncDecl) -> Vec<SymbolId> {
    let Some(tpl) = f.type_params() else {
        return Vec::new();
    };
    tpl.params()
        .map(|tok| {
            ctx.define_sym(
                tok.text().to_string(),
                SymbolKind::TypeParam,
                tok.text_range(),
            )
        })
        .collect()
}

fn is_type_sig_node(node: &SyntaxNode) -> bool {
    use SyntaxKind::*;
    matches!(
        node.kind(),
        FUNCTION_TYPE | TYPE_FORM | TYPE_RECORD | TYPE_UNION | VARIANT_TYPE | OPTIONAL_TYPE
    )
}

// ── Multi-clause lowering ─────────────────────────────────────────────────────
//
// `f :: A -> B -> C`
// `:: pat1a pat1b -> body1`
// `:: pat2a pat2b -> body2`
//
// Becomes (per plan §"Multi-clause lowering"):
//   Lambda(p0, Match(p0, [
//     (pat1a, Lambda(p1, Match(p1, [(pat1b, body1)]))),  // clause 1
//     (pat2a, Lambda(p1, Match(p1, [(pat2b, body2)]))),  // clause 2
//   ]))
//
// The outer Match tests all clauses' patterns at position 0.
// Each arm's body is the Lambda chain for the SAME clause's remaining args.
// This is correct for exhaustive, non-overlapping first-argument patterns.

fn lower_clauses(ctx: &mut LowerCtx, clauses: &[Clause], range: text_size::TextRange) -> HirExprId {
    if clauses.is_empty() {
        return ctx.error_expr(range);
    }

    let arity = clauses
        .iter()
        .map(|c| c.patterns().count())
        .max()
        .unwrap_or(0);

    if arity == 0 {
        // No-argument function: lower the first clause's body directly
        if let Some(body) = clauses[0].body() {
            ctx.scopes.push_child();
            let result = lower_block(ctx, body.syntax());
            ctx.scopes.pop();
            return result;
        }
        return ctx.error_expr(range);
    }

    lower_at_depth(ctx, clauses, 0, arity, range)
}

/// Build `Lambda(p, Match(p, [arm_per_clause]))` for argument position `depth`.
///
/// At `depth == 0`: one arm per clause, each arm's body is the single-clause
/// continuation at depth 1.
/// At `depth > 0`: called per-clause; one arm for that clause's remaining args.
fn lower_at_depth(
    ctx: &mut LowerCtx,
    clauses: &[Clause],
    depth: usize,
    arity: usize,
    range: text_size::TextRange,
) -> HirExprId {
    let param_name = format!("__p{depth}");
    ctx.scopes.push_child();
    let param_sym = ctx.define_sym(param_name, SymbolKind::Local, range);
    let param_pat = ctx.alloc_pat(HirPatKind::Bind(param_sym), range);
    let param_var = ctx.alloc_expr(HirExprKind::Var(param_sym), range);

    let mut arms = Vec::new();
    for clause in clauses {
        let pats: Vec<_> = clause.patterns().collect();
        let Some(pat_node) = pats.get(depth) else {
            continue;
        };

        ctx.scopes.push_child();
        let pat = lower_pat(ctx, pat_node.syntax());

        let (guard, body) = if depth == arity - 1 {
            // Last argument: lower the body and extract the guard
            let guard = clause
                .guard()
                .and_then(|g| g.condition())
                .map(|cond| lower_expr(ctx, cond.syntax()));
            let body = clause
                .body()
                .map(|b| lower_block(ctx, b.syntax()))
                .unwrap_or_else(|| ctx.error_expr(range));
            (guard, body)
        } else {
            // Not the last argument: recurse for this clause's remaining args
            let body = lower_single_clause_at_depth(ctx, clause, depth + 1, arity, range);
            (None, body)
        };

        ctx.scopes.pop();
        arms.push(HirArm { pat, guard, body });
    }

    let match_expr = ctx.alloc_expr(
        HirExprKind::Match {
            scrutinee: param_var,
            arms,
        },
        range,
    );
    ctx.scopes.pop();

    ctx.alloc_expr(
        HirExprKind::Lambda {
            params: vec![param_pat],
            body: match_expr,
        },
        range,
    )
}

/// Build the Lambda chain for a SINGLE clause's arguments starting at `depth`.
/// Used for depths > 0 where we've already committed to a specific clause.
fn lower_single_clause_at_depth(
    ctx: &mut LowerCtx,
    clause: &Clause,
    depth: usize,
    arity: usize,
    range: text_size::TextRange,
) -> HirExprId {
    let param_name = format!("__p{depth}");
    ctx.scopes.push_child();
    let param_sym = ctx.define_sym(param_name, SymbolKind::Local, range);
    let param_pat = ctx.alloc_pat(HirPatKind::Bind(param_sym), range);
    let param_var = ctx.alloc_expr(HirExprKind::Var(param_sym), range);

    let pats: Vec<_> = clause.patterns().collect();
    let pat_node = pats.get(depth);

    ctx.scopes.push_child();
    let inner_pat = pat_node
        .map(|n| lower_pat(ctx, n.syntax()))
        .unwrap_or_else(|| ctx.error_pat(range));

    let (guard, body) = if depth == arity - 1 {
        let guard = clause
            .guard()
            .and_then(|g| g.condition())
            .map(|cond| lower_expr(ctx, cond.syntax()));
        let body = clause
            .body()
            .map(|b| lower_block(ctx, b.syntax()))
            .unwrap_or_else(|| ctx.error_expr(range));
        (guard, body)
    } else {
        let body = lower_single_clause_at_depth(ctx, clause, depth + 1, arity, range);
        (None, body)
    };

    let arm = HirArm {
        pat: inner_pat,
        guard,
        body,
    };
    ctx.scopes.pop();

    let match_expr = ctx.alloc_expr(
        HirExprKind::Match {
            scrutinee: param_var,
            arms: vec![arm],
        },
        range,
    );
    ctx.scopes.pop();

    ctx.alloc_expr(
        HirExprKind::Lambda {
            params: vec![param_pat],
            body: match_expr,
        },
        range,
    )
}
