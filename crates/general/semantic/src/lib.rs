//! Semantic analysis for Zutai general mode (`.zt`).
//!
//! Entry point: [`analyze`]. Feed it the root [`SyntaxNode`] returned by
//! `zutai_syntax::parse(src).syntax()`, get back an [`AnalysisResult`].

pub mod ast_ext;
pub mod context;
pub mod pass;
pub mod passes;
pub mod resolution;
pub mod scope;
pub mod ty;

pub use context::AnalysisContext;
pub use resolution::ResolutionMap;

use zutai_syntax::SyntaxNode;
use zutai_syntax::diag::Diagnostic;

/// The output of a full semantic analysis run.
pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,
    pub resolution: ResolutionMap,
}

/// Run all registered semantic passes over a parsed `.zt` tree.
///
/// Always succeeds (passes are infallible); diagnostics are collected
/// inside `AnalysisResult`. Call after `zutai_syntax::parse` and append
/// the two diagnostic vecs before rendering.
pub fn analyze(root: &SyntaxNode) -> AnalysisResult {
    let mut ctx = AnalysisContext::new();
    for pass in pass::default_passes() {
        pass.run(root, &mut ctx);
    }
    ctx.finish()
}
