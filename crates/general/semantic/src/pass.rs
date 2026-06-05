use zutai_hir::HirFile;

use crate::context::AnalysisContext;
use crate::passes::TypeCheck;

// ── Pass trait ────────────────────────────────────────────────────────────────

/// A single, independent semantic analysis pass.
///
/// Passes run in order over the lowered HIR for one file. Name resolution and
/// CST-to-HIR normalization have already happened before this point.
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, hir: &mut HirFile, ctx: &mut AnalysisContext);
}

// ── Pass registry ─────────────────────────────────────────────────────────────

/// Returns the ordered list of passes to run.
///
/// Order matters: later passes may depend on types or facts produced by earlier
/// passes.
pub fn default_passes() -> Vec<Box<dyn Pass>> {
    vec![Box::new(TypeCheck)]
}
