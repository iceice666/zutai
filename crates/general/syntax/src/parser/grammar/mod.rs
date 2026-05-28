mod decls;
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
pub(super) fn file(p: &mut Parser) {
    let m = p.start();
    while !p.at_eof() {
        // Top-level declaration: `IDENT` followed by `:=`, `:`, or `::`.
        if p.is_decl_start() {
            decls::top_decl(p);
        } else if exprs::expr(p).is_some() {
            // Parsed an expression (the trailing file-output expression, or an
            // error-recovery expression). Keep looping — if there are tokens
            // remaining they are errors and will be consumed by the next iteration.
        } else {
            // Nothing matched: consume one token into an ERROR_NODE so the loop
            // always makes forward progress.
            let err_m = p.start();
            p.error(format!("unexpected token at top level: {:?}", p.current()));
            p.bump_any();
            err_m.complete(p, SyntaxKind::ERROR_NODE);
        }
    }
    m.complete(p, SyntaxKind::FILE);
}
