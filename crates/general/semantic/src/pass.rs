use zutai_syntax::SyntaxNode;

use crate::context::AnalysisContext;
use crate::passes::{NameResolution, TypeCheck};

// ── Pass trait ────────────────────────────────────────────────────────────────

/// A single, independent semantic analysis pass.
///
/// Passes run in order over the same `SyntaxNode` root. Each pass may read
/// from `ctx` (e.g. resolution results produced by a prior pass) and push
/// diagnostics via `ctx.error` / `ctx.warning`.
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, root: &SyntaxNode, ctx: &mut AnalysisContext);
}

// ── Pass registry ─────────────────────────────────────────────────────────────

/// Returns the ordered list of passes to run.
///
/// Order matters: later passes may depend on results (e.g. resolution map)
/// produced by earlier ones.
pub fn default_passes() -> Vec<Box<dyn Pass>> {
    vec![Box::new(NameResolution), Box::new(TypeCheck)]
}
