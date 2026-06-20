use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn alloc_decl(&mut self, decl: ThirDecl) -> ThirDeclId {
        self.decl_arena.alloc(decl)
    }

    pub(in crate::lower) fn alloc_expr(&mut self, expr: ThirExpr) -> ThirExprId {
        self.expr_arena.alloc(expr)
    }

    pub(in crate::lower) fn alloc_pat(&mut self, pat: ThirPat) -> ThirPatId {
        self.pat_arena.alloc(pat)
    }

    pub(in crate::lower) fn alloc_type(&mut self, ty: Type) -> TypeId {
        let id = TypeId(self.type_arena.len() as u32);
        self.type_arena.push(ty);
        id
    }

    pub(in crate::lower) fn hir_decl(&self, id: HirDeclId) -> &'hir HirDecl {
        &self.hir.decl_arena[id]
    }

    pub(in crate::lower) fn hir_expr(&self, id: HirExprId) -> &'hir HirExpr {
        &self.hir.expr_arena[id]
    }

    pub(in crate::lower) fn hir_type(&self, id: HirTypeId) -> &'hir HirTypeExpr {
        &self.hir.type_arena[id]
    }

    pub(in crate::lower) fn hir_pat(&self, id: HirPatId) -> &'hir HirPat {
        &self.hir.pat_arena[id]
    }

    pub(in crate::lower) fn expr(&self, id: ThirExprId) -> &ThirExpr {
        &self.expr_arena[id]
    }

    pub(in crate::lower) fn ty(&self, id: TypeId) -> &Type {
        &self.type_arena[id.0 as usize]
    }

    pub(in crate::lower) fn unsupported_expr(
        &mut self,
        id: HirExprId,
        feature: &'static str,
        span: Span,
    ) -> ThirExprId {
        self.unsupported(feature, span);
        self.error_expr(id, span)
    }

    pub(in crate::lower) fn unsupported(&mut self, feature: &'static str, span: Span) {
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::UnsupportedFeature { feature },
            span,
        });
    }

    pub(in crate::lower) fn invalid_type(&mut self, reason: &'static str, span: Span) -> TypeId {
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::InvalidTypeExpression { reason },
            span,
        });
        self.error_type
    }

    pub(in crate::lower) fn error_expr(&mut self, source: HirExprId, span: Span) -> ThirExprId {
        self.alloc_expr(ThirExpr {
            source,
            ty: self.error_type,
            kind: ThirExprKind::Error,
            span,
        })
    }
}
