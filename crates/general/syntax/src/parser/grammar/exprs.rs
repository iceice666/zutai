use crate::SyntaxKind;

use super::{
    super::{CompletedMarker, Parser},
    Ctx,
    primary::primary,
};

// ── Binding-power constants ───────────────────────────────────────────────────
// Pairs (l_bp, r_bp) per operator level, from tightest to loosest.
// Postfix operators only have a left_bp (no recursive right operand).
//
// Convention (Matklad-style Pratt):
//   left-assoc:  l_bp=N, r_bp=N+1  (equal-level op won't re-enter)
//   right-assoc: l_bp=N+1, r_bp=N  (equal-level op re-enters on right)
//   non-assoc:   l_bp=N, r_bp=N+1  + manual check after first fold

const BP_POSTFIX: u8 = 19; // . ?. and (Type-only) ?
const BP_APP_L: u8 = 17; // juxtaposition application
const BP_APP_R: u8 = 18;
const BP_MUL_L: u8 = 15; // * /
const BP_MUL_R: u8 = 16;
const BP_ADD_L: u8 = 13; // + -
const BP_ADD_R: u8 = 14;
const BP_CMP_L: u8 = 11; // == != < <= > >= (non-assoc)
const BP_CMP_R: u8 = 12;
const BP_AND_L: u8 = 9; // &&
const BP_AND_R: u8 = 10;
const BP_OR_L: u8 = 7; // ||
const BP_OR_R: u8 = 8;
const BP_COAL_L: u8 = 6; // ?? (right-assoc: l > r)
const BP_COAL_R: u8 = 5;
const BP_PIPE_L: u8 = 3; // |> and <| share the same left_bp
const BP_PIPE_FWD_R: u8 = 4; // |> left-assoc: r > l
const BP_PIPE_BWD_R: u8 = 2; // <| right-assoc: r < l
const BP_ARROW_L: u8 = 2; // -> function type (right-assoc, Type ctx only)
const BP_ARROW_R: u8 = 1;

// ── Public entry points ───────────────────────────────────────────────────────

/// Parse an expression at precedence 0 (lowest). Used from file() and other callers.
pub(crate) fn expr(p: &mut Parser) -> Option<CompletedMarker> {
    expr_bp(p, 0, Ctx::Expr)
}

/// Parse a type expression at precedence 0.
pub(crate) fn type_expr(p: &mut Parser) -> Option<CompletedMarker> {
    expr_bp(p, 0, Ctx::Type)
}

/// Core Pratt expression/type parser. Parses with a minimum binding power of
/// `min_bp` in context `ctx`. Returns the completed marker for the parsed node,
/// or `None` when the current token cannot start an expression.
pub(crate) fn expr_bp(p: &mut Parser, min_bp: u8, ctx: Ctx) -> Option<CompletedMarker> {
    let mut lhs = primary(p, ctx)?;

    // Per-loop state for semantic checks.
    let mut saw_comparison = false;
    let mut pipe_dir: Option<bool> = None; // Some(true)=|>, Some(false)=<|

    loop {
        let cur = p.current();

        // ── Postfix operators (prec 1 / highest) ─────────────────────────────
        if cur == SyntaxKind::DOT {
            if BP_POSTFIX < min_bp {
                break;
            }
            let m = lhs.precede(p);
            p.bump(SyntaxKind::DOT);
            if super::primary::field_name(p).is_none() {
                p.error("expected field name after '.'");
            }
            lhs = m.complete(p, SyntaxKind::ACCESS_EXPR);
            continue;
        }

        if cur == SyntaxKind::OPTIONAL_DOT {
            if BP_POSTFIX < min_bp {
                break;
            }
            let m = lhs.precede(p);
            p.bump(SyntaxKind::OPTIONAL_DOT);
            if super::primary::field_name(p).is_none() {
                p.error("expected field name after '?.'");
            }
            lhs = m.complete(p, SyntaxKind::OPTIONAL_ACCESS_EXPR);
            continue;
        }

        // Postfix `?` is only active in Type context (optional type `T?`).
        if cur == SyntaxKind::QUESTION && ctx == Ctx::Type {
            if BP_POSTFIX < min_bp {
                break;
            }
            let m = lhs.precede(p);
            p.bump(SyntaxKind::QUESTION);
            lhs = m.complete(p, SyntaxKind::OPTIONAL_TYPE);
            continue;
        }

        // `??` in type position means double-optional (`T??` = `(T?)?`).
        // The token is a single QUESTION_QUESTION due to maximal munch; it cannot
        // be split, so one OPTIONAL_TYPE node wraps the lhs and holds the `??` token.
        if cur == SyntaxKind::QUESTION_QUESTION && ctx == Ctx::Type {
            if BP_POSTFIX < min_bp {
                break;
            }
            let m = lhs.precede(p);
            p.bump(SyntaxKind::QUESTION_QUESTION);
            lhs = m.complete(p, SyntaxKind::OPTIONAL_TYPE);
            continue;
        }

        // ── Application by juxtaposition (prec 2) ────────────────────────────
        if BP_APP_L >= min_bp && is_app_start(p) {
            let m = lhs.precede(p);
            // Parse the argument at APP_R_BP so equal-level application folds
            // left-associatively: `f a b` → `(f a) b`.
            if expr_bp(p, BP_APP_R, ctx).is_none() {
                p.error("expected expression as function argument");
            }
            lhs = m.complete(p, SyntaxKind::CALL_EXPR);
            continue;
        }

        // ── Infix operators ───────────────────────────────────────────────────
        let (l_bp, r_bp, node_kind) = match infix_bp(cur, ctx) {
            Some(x) => x,
            None => break,
        };

        if l_bp < min_bp {
            break;
        }

        // Non-assoc comparison: diagnose chaining after the first fold.
        let is_cmp = is_comparison(cur);
        if is_cmp && saw_comparison {
            p.error(
                "comparison operators are non-associative; use parentheses to make grouping explicit",
            );
        }

        // Pipeline-direction mix ban.
        if cur == SyntaxKind::PIPE_ARROW || cur == SyntaxKind::ARROW_PIPE {
            let dir = cur == SyntaxKind::PIPE_ARROW;
            if let Some(prev) = pipe_dir
                && prev != dir
            {
                p.error("mixing |> and <| in one pipeline chain is ambiguous; use parentheses");
            }
            pipe_dir = Some(dir);
        }

        let m = lhs.precede(p);
        p.bump_any(); // consume the operator token

        if expr_bp(p, r_bp, ctx).is_none() {
            p.error("expected expression");
        }
        lhs = m.complete(p, node_kind);

        if is_cmp {
            saw_comparison = true;
        }
    }

    Some(lhs)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `(left_bp, right_bp, node_kind)` for infix operators, or `None` when
/// the current token is not an infix operator (in the given context).
fn infix_bp(kind: SyntaxKind, ctx: Ctx) -> Option<(u8, u8, SyntaxKind)> {
    use SyntaxKind::*;
    Some(match kind {
        STAR | SLASH => (BP_MUL_L, BP_MUL_R, BINARY_EXPR),
        PLUS | MINUS => (BP_ADD_L, BP_ADD_R, BINARY_EXPR),
        EQ_EQ | BANG_EQ | LT | LT_EQ | GT | GT_EQ => (BP_CMP_L, BP_CMP_R, BINARY_EXPR),
        AMP_AMP => (BP_AND_L, BP_AND_R, BINARY_EXPR),
        PIPE_PIPE => (BP_OR_L, BP_OR_R, BINARY_EXPR),
        QUESTION_QUESTION => (BP_COAL_L, BP_COAL_R, BINARY_EXPR),
        PIPE_ARROW => (BP_PIPE_L, BP_PIPE_FWD_R, PIPELINE_EXPR),
        ARROW_PIPE => (BP_PIPE_L, BP_PIPE_BWD_R, PIPELINE_EXPR),
        // `->` is the function-type arrow; only active in Type context.
        ARROW if ctx == Ctx::Type => (BP_ARROW_L, BP_ARROW_R, FUNCTION_TYPE),
        _ => return None,
    })
}

fn is_comparison(kind: SyntaxKind) -> bool {
    use SyntaxKind::*;
    matches!(kind, EQ_EQ | BANG_EQ | LT | LT_EQ | GT | GT_EQ)
}

/// Whether the current token can start a primary that acts as a function argument
/// (juxtaposition application).
///
/// `{` is intentionally excluded: it appears as a match/clause body and including
/// it would cause `match scrutinee { ... }` to swallow the case block as an arg.
/// Records as arguments must be wrapped in parentheses.
///
/// Top-level declarations (`IDENT :=`, `IDENT :`, `IDENT ::`) are also excluded:
/// they terminate the current expression so each declaration's RHS stays bounded.
fn is_app_start(p: &Parser) -> bool {
    use SyntaxKind::*;
    // An IDENT followed by `:=`, `:`, or `::` starts a new top-level declaration,
    // not an argument. Without this guard, `make_adder 5\nadd_neg :=` would
    // incorrectly parse as `make_adder 5 add_neg`.
    if p.is_decl_start() {
        return false;
    }
    match p.current() {
        IDENT | UNDERSCORE | INT | FLOAT | STRING | ATOM | KW_TRUE | KW_FALSE | KW_NONE
        | L_PAREN | L_BRACK | BACKSLASH | KW_IF | KW_MATCH | KW_IMPORT => true,
        // Negative literal: MINUS immediately adjacent to a number.
        MINUS => p.raw_adjacent() && matches!(p.nth(1), INT | FLOAT),
        _ => false,
    }
}
