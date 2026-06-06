use std::collections::HashMap;

use text_size::TextRange;

use zutai_hir::expr::HirExprId;
use zutai_syntax::diag::{Diagnostic, ErrorCode};

use crate::ResolutionMap;
use crate::scope::ScopeStack;
use crate::ty::{TyId, TyInterner};

// ── AnalysisContext ───────────────────────────────────────────────────────────

/// Shared mutable state threaded through every analysis pass.
pub struct AnalysisContext {
    pub scopes: ScopeStack,
    pub resolution: ResolutionMap,
    pub types: TyInterner,
    pub expr_types: HashMap<HirExprId, TyId>,
    pub diagnostics: Vec<Diagnostic>,
}

impl AnalysisContext {
    pub fn new() -> Self {
        Self {
            scopes: ScopeStack::new(),
            resolution: ResolutionMap::new(),
            types: TyInterner::new(),
            expr_types: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Push an error diagnostic.
    pub fn error(&mut self, range: TextRange, code: ErrorCode, msg: impl Into<String>) {
        self.diagnostics.push(Diagnostic::error(range, code, msg));
    }

    /// Push a warning diagnostic.
    pub fn warning(&mut self, range: TextRange, code: ErrorCode, msg: impl Into<String>) {
        self.diagnostics.push(Diagnostic::warning(range, code, msg));
    }

    /// Push a diagnostic with an additional secondary label.
    pub fn error_with_label(
        &mut self,
        range: TextRange,
        code: ErrorCode,
        msg: impl Into<String>,
        label_range: TextRange,
        label_msg: impl Into<String>,
    ) {
        self.diagnostics
            .push(Diagnostic::error(range, code, msg).with_label(label_range, label_msg));
    }

    /// Consume the context and produce the diagnostics and resolution map.
    /// The caller (`analyze`) is responsible for combining these with the HIR.
    pub fn finish(self) -> (Vec<zutai_syntax::diag::Diagnostic>, ResolutionMap) {
        (self.diagnostics, self.resolution)
    }
}

impl Default for AnalysisContext {
    fn default() -> Self {
        Self::new()
    }
}
