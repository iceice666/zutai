pub(super) mod exprs;
mod patterns;
mod primary;
mod types;

use super::Parser;
use crate::SyntaxKind;

/// Parsing context — distinguishes expression from type position so the shared
/// Pratt driver can flip behaviour for `{`/`[` disambiguation and type-only
/// operators (`?` postfix, `->`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ctx {
    Expr,
    Type,
}

/// Entry point: parse a complete `.zt` file (`TopDecl* Expr`).
/// M3: treats the file as a sequence of expressions, recovering over unknown
/// tokens. M7 will add proper top-level declaration dispatch.
pub(super) fn file(p: &mut Parser) {
    let m = p.start();
    while !p.at_eof() {
        if exprs::expr(p).is_none() {
            // Unknown token in non-expression position — wrap in ERROR_NODE and
            // continue so the round-trip invariant holds.
            let err_m = p.start();
            p.error(format!("unexpected token: {:?}", p.current()));
            p.bump_any();
            err_m.complete(p, SyntaxKind::ERROR_NODE);
        }
    }
    m.complete(p, SyntaxKind::FILE);
}
