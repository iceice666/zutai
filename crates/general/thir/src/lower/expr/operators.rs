use super::*;

impl<'hir> Lowerer<'hir> {
    pub(super) fn lower_binary_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        match op {
            ast::BinOp::And | ast::BinOp::Or => self.lower_bool_binary_expr(id, op, lhs, rhs, span),
            ast::BinOp::Eq | ast::BinOp::Ne => self.lower_equality_expr(id, op, lhs, rhs, span),
            ast::BinOp::Lt | ast::BinOp::Le | ast::BinOp::Gt | ast::BinOp::Ge => {
                self.lower_ordering_expr(id, op, lhs, rhs, span)
            }
            ast::BinOp::Add | ast::BinOp::Sub | ast::BinOp::Mul | ast::BinOp::Div => {
                self.lower_arithmetic_expr(id, op, lhs, rhs, span)
            }
            ast::BinOp::Coalesce => self.lower_coalesce_expr(id, lhs, rhs, span),
        }
    }

    pub(super) fn lower_bool_binary_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let ty = self.bool_type(span);
        let lhs = self.check_expr(lhs, ty);
        let rhs = self.check_expr(rhs, ty);
        self.alloc_binary_expr(id, op, lhs, rhs, ty, span)
    }

    pub(super) fn lower_equality_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let rhs = self.check_expr(rhs, lhs_ty);
        let ty = self.bool_type(span);
        self.alloc_binary_expr(id, op, lhs, rhs, ty, span)
    }

    pub(super) fn lower_ordering_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let rhs = self.check_expr(rhs, lhs_ty);
        let lhs_resolved = self.resolve(lhs_ty);
        if !self.is_ordered_scalar(lhs_ty) {
            if matches!(
                self.type_arena[lhs_resolved.0 as usize].kind,
                TypeKind::InferVar(_)
            ) {
                let int_ty = self.int_type(span);
                self.unify(lhs_ty, int_ty, span);
            } else if !self.hir_has_ordering_constraint(op) {
                let rhs_ty = self.expr(rhs).ty;
                self.invalid_binary_operands(op, lhs_ty, rhs_ty, span);
            }
        }
        let ty = self.bool_type(span);
        self.alloc_binary_expr(id, op, lhs, rhs, ty, span)
    }

    pub(super) fn lower_arithmetic_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let rhs = self.check_expr(rhs, lhs_ty);
        let lhs_resolved = self.resolve(lhs_ty);
        if !self.is_numeric_scalar(lhs_ty) {
            if matches!(
                self.type_arena[lhs_resolved.0 as usize].kind,
                TypeKind::InferVar(_)
            ) {
                // Default unresolved numeric context to Int.
                let int_ty = self.int_type(span);
                self.unify(lhs_ty, int_ty, span);
            } else {
                let rhs_ty = self.expr(rhs).ty;
                self.invalid_binary_operands(op, lhs_ty, rhs_ty, span);
            }
        }
        // After possible unification, use the resolved type for the result.
        let result_ty = self.resolve(lhs_ty);
        self.alloc_binary_expr(id, op, lhs, rhs, result_ty, span)
    }

    pub(super) fn lower_coalesce_expr(
        &mut self,
        id: HirExprId,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let lhs = self.infer_expr(lhs);
        let lhs_ty = self.expr(lhs).ty;
        let Some((_wrapper_kind, inner)) = self.optional_or_maybe_inner_type(lhs_ty, span) else {
            let found = self.type_name(lhs_ty);
            if !matches!(self.ty(lhs_ty).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedOptionalOrMaybe { found },
                    span,
                });
            }
            let rhs = self.infer_expr(rhs);
            return self.alloc_binary_expr(
                id,
                ast::BinOp::Coalesce,
                lhs,
                rhs,
                self.error_type,
                span,
            );
        };
        let rhs = self.check_expr(rhs, inner);
        self.alloc_binary_expr(id, ast::BinOp::Coalesce, lhs, rhs, inner, span)
    }

    pub(super) fn alloc_binary_expr(
        &mut self,
        id: HirExprId,
        op: ast::BinOp,
        lhs: ThirExprId,
        rhs: ThirExprId,
        ty: TypeId,
        span: Span,
    ) -> ThirExprId {
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Binary { op, lhs, rhs },
            span,
        })
    }

    pub(super) fn is_numeric_scalar(&mut self, ty: TypeId) -> bool {
        let resolved = self.resolve_alias_for_expr(ty);
        matches!(
            self.ty(resolved).kind,
            TypeKind::Int | TypeKind::Float | TypeKind::Posit(_)
        )
    }

    pub(super) fn is_ordered_scalar(&mut self, ty: TypeId) -> bool {
        self.is_numeric_scalar(ty) || {
            let resolved = self.resolve_alias_for_expr(ty);
            matches!(self.ty(resolved).kind, TypeKind::Text)
        }
    }

    /// Returns `true` if any HIR constraint declares an operator method whose
    /// name matches `bin_op_name(op)`. Used to allow non-scalar ordering
    /// expressions to type-check when a user-defined witness may cover them.
    pub(super) fn hir_has_ordering_constraint(&self, op: ast::BinOp) -> bool {
        let op_name = bin_op_name(op);
        for &decl_id in &self.hir.decls {
            let decl = self.hir_decl(decl_id);
            if let HirDeclKind::Constraint { methods, .. } = &decl.kind {
                for m in methods {
                    if m.is_operator && m.name == op_name {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub(super) fn invalid_binary_operands(
        &mut self,
        op: ast::BinOp,
        lhs: TypeId,
        rhs: TypeId,
        span: Span,
    ) {
        let lhs = self.type_name(lhs);
        let rhs = self.type_name(rhs);
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::InvalidBinaryOperands {
                op: bin_op_name(op),
                lhs,
                rhs,
            },
            span,
        });
    }
}

fn bin_op_name(op: ast::BinOp) -> &'static str {
    match op {
        ast::BinOp::Mul => "*",
        ast::BinOp::Div => "/",
        ast::BinOp::Add => "+",
        ast::BinOp::Sub => "-",
        ast::BinOp::Eq => "==",
        ast::BinOp::Ne => "!=",
        ast::BinOp::Lt => "<",
        ast::BinOp::Le => "<=",
        ast::BinOp::Gt => ">",
        ast::BinOp::Ge => ">=",
        ast::BinOp::And => "&&",
        ast::BinOp::Or => "||",
        ast::BinOp::Coalesce => "??",
    }
}
