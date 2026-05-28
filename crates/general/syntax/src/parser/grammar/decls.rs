use crate::{SyntaxKind, token_set::CLAUSE_RECOVERY};

use super::{
    super::Parser,
    Ctx,
    exprs::{expr, type_expr},
    patterns::pattern,
    primary::block_expr,
    types::type_form,
};

/// Parse a top-level declaration.
///
/// Pre-condition: `p.at(IDENT)` and `p.nth(1) ∈ {COLON_EQ, COLON, COLON_COLON}`.
pub(super) fn top_decl(p: &mut Parser) {
    match p.nth(1) {
        SyntaxKind::COLON_EQ => inferred_binding(p),
        SyntaxKind::COLON => annotated_binding(p),
        SyntaxKind::COLON_COLON => func_decl(p),
        _ => unreachable!("top_decl called without `:=`/`:`/`::`"),
    }
}

// ── IDENT := Expr ─────────────────────────────────────────────────────────────

fn inferred_binding(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::IDENT);
    p.bump(SyntaxKind::COLON_EQ);
    if expr(p).is_none() {
        p.error("expected expression in value binding");
    }
    m.complete(p, SyntaxKind::INFERRED_BINDING);
}

// ── IDENT : TypeExpr = Expr ───────────────────────────────────────────────────

fn annotated_binding(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::IDENT);
    p.bump(SyntaxKind::COLON);
    if type_expr(p).is_none() {
        p.error("expected type expression after ':'");
    }
    if !p.eat(SyntaxKind::EQ) {
        p.error("expected '=' after type annotation");
    }
    if expr(p).is_none() {
        p.error("expected expression after '='");
    }
    m.complete(p, SyntaxKind::ANNOTATED_BINDING);
}

// ── IDENT :: TypeParamList? (TypeExpr ::)? Clause+  or  IDENT :: type {...} ──

fn func_decl(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::IDENT);
    p.bump(SyntaxKind::COLON_COLON);

    // Type definition: `IDENT :: type { ... }` or `IDENT :: type [ ... ]`
    if p.at(SyntaxKind::KW_TYPE) {
        type_form(p);
        m.complete(p, SyntaxKind::FUNC_DECL);
        return;
    }

    // Optional type parameter list: `[A]`, `[A, B]` (comma-only, no `;` at depth 1).
    if p.at(SyntaxKind::L_BRACK) && looks_like_type_param_list(p) {
        type_param_list(p);
    }

    // Does a type signature precede the first clause?
    // Scan forward: if `::` appears before `{` at bracket-depth 0, there's a sig.
    if has_type_sig(p) {
        if type_expr(p).is_none() {
            p.error("expected type expression in function signature");
        }
        if !p.eat(SyntaxKind::COLON_COLON) {
            p.error("expected '::' after function signature");
        }
    }

    // One or more clauses.
    if p.at(SyntaxKind::EOF) {
        p.error("expected at least one clause in function declaration");
        m.complete(p, SyntaxKind::FUNC_DECL);
        return;
    }
    clause(p);
    while p.eat(SyntaxKind::COLON_COLON) {
        clause(p);
    }

    m.complete(p, SyntaxKind::FUNC_DECL);
}

/// Parse `[TypeVar (, TypeVar)*]` as a TYPE_PARAM_LIST node.
fn type_param_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::L_BRACK);
    if !p.eat(SyntaxKind::IDENT) {
        p.error("expected type variable in type parameter list");
    }
    while p.eat(SyntaxKind::COMMA) {
        if !p.eat(SyntaxKind::IDENT) {
            p.error("expected type variable after ','");
            break;
        }
    }
    p.expect(SyntaxKind::R_BRACK);
    m.complete(p, SyntaxKind::TYPE_PARAM_LIST);
}

// ── Clause: Pattern ("->" Pattern)* Guard? "{" Block "}" ─────────────────────

fn clause(p: &mut Parser) {
    let m = p.start();

    // First pattern (required). Use err_recover if no pattern found.
    if pattern(p).is_none() {
        p.err_recover(
            format!("expected pattern in clause, got {:?}", p.current()),
            CLAUSE_RECOVERY,
        );
    }

    // Additional argument patterns separated by `->`.
    while p.eat(SyntaxKind::ARROW) {
        if pattern(p).is_none() {
            p.err_recover("expected pattern after '->' in clause", CLAUSE_RECOVERY);
            break;
        }
    }

    // Optional guard: `if Expr`
    if p.at(SyntaxKind::KW_IF) {
        let gm = p.start();
        p.bump(SyntaxKind::KW_IF);
        if expr(p).is_none() {
            p.error("expected expression in clause guard");
        }
        gm.complete(p, SyntaxKind::GUARD);
    }

    // Block body: `{ … }`
    if p.at(SyntaxKind::L_BRACE) {
        block_expr(p, Ctx::Expr);
    } else {
        p.error(format!(
            "expected '{{' to start clause body, got {:?}",
            p.current()
        ));
        // Minimal recovery: skip to next `::` or EOF so the decl loop can continue.
        while !p.at_eof() && !p.at(SyntaxKind::COLON_COLON) && !p.is_decl_start() {
            p.bump_any();
        }
    }

    m.complete(p, SyntaxKind::CLAUSE);
}

// ── Lookahead helpers ─────────────────────────────────────────────────────────

/// From the current position (`[`), determine whether this bracket is a type
/// parameter list (`[A]`, `[A, B]`) rather than a union type.
///
/// A TYPE_PARAM_LIST has only comma-separated items with no `;` at depth 1.
/// A union type (`[A; B;]`) has `;` at depth 1.
fn looks_like_type_param_list(p: &Parser) -> bool {
    debug_assert!(p.at(SyntaxKind::L_BRACK));
    let mut depth = 1usize;
    let mut off = 1usize; // inside the `[`
    loop {
        match p.nth(off) {
            SyntaxKind::EOF => return false,
            SyntaxKind::L_BRACK | SyntaxKind::L_PAREN => depth += 1,
            SyntaxKind::L_BRACE => depth += 1,
            SyntaxKind::R_PAREN => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            SyntaxKind::R_BRACE => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            SyntaxKind::R_BRACK => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
                if depth == 0 {
                    return true; // closed without a depth-1 `;`
                }
            }
            SyntaxKind::SEMI if depth == 1 => return false,
            _ => {}
        }
        off += 1;
        if off > 64 {
            return false; // bounded: type param lists are short
        }
    }
}

/// From the current position, scan forward (respecting bracket depth) looking
/// for `::` or `{` at depth 0. Returns `true` if `::` is found first (there is
/// a type signature), `false` if `{` is found first (clauses-only form).
fn has_type_sig(p: &Parser) -> bool {
    let mut depth = 0usize;
    let mut off = 0usize;
    loop {
        match p.nth(off) {
            SyntaxKind::EOF => return false,
            SyntaxKind::L_BRACK | SyntaxKind::L_PAREN => depth += 1,
            SyntaxKind::L_BRACE => {
                if depth == 0 {
                    return false; // clause body starts here, no type sig
                }
                depth += 1;
            }
            SyntaxKind::R_BRACK | SyntaxKind::R_PAREN | SyntaxKind::R_BRACE => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            SyntaxKind::COLON_COLON if depth == 0 => return true,
            _ => {}
        }
        off += 1;
        if off > 512 {
            return false; // safety bound
        }
    }
}
