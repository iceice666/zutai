use super::*;

impl<'hir> Lowerer<'hir> {
    pub(super) fn lower_block_expr(
        &mut self,
        id: HirExprId,
        bindings: &[HirLocalBinding],
        result: HirExprId,
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let mut scoped_bindings = Vec::with_capacity(bindings.len());
        let bindings = bindings
            .iter()
            .map(|binding| {
                let value = self.infer_expr(binding.value);
                let ty = self.expr(value).ty;
                self.value_types.insert(binding.binding, ty);
                scoped_bindings.push(binding.binding);
                ThirLocalBinding {
                    binding: binding.binding,
                    ty,
                    value,
                    span: binding.span,
                }
            })
            .collect();
        let result = match expected {
            Some(expected) => self.check_expr(result, expected),
            None => self.infer_expr(result),
        };
        self.clear_scoped_value_types(&scoped_bindings);
        let ty = self.expr(result).ty;

        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Block { bindings, result },
            span,
        })
    }

    pub(super) fn lower_sequence_expr(
        &mut self,
        id: HirExprId,
        items: &[HirExprId],
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some((&last, prefix)) = items.split_last() else {
            let ty = self.unit_type(span);
            return self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::Sequence(Vec::new()),
                span,
            });
        };

        let mut lowered = Vec::with_capacity(items.len());
        for &item in prefix {
            lowered.push(self.infer_expr(item));
        }
        let last = match expected {
            Some(expected) => self.check_expr(last, expected),
            None => self.infer_expr(last),
        };
        lowered.push(last);
        let ty = self.expr(last).ty;
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Sequence(lowered),
            span,
        })
    }

    pub(super) fn lower_if_expr(
        &mut self,
        id: HirExprId,
        cond: HirExprId,
        then_branch: HirExprId,
        else_branch: HirExprId,
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let bool_ty = self.bool_type(span);
        let cond = self.check_expr(cond, bool_ty);
        let (then_branch, else_branch, ty) = match expected {
            Some(expected) => {
                let then_branch = self.check_expr(then_branch, expected);
                let else_branch = self.check_expr(else_branch, expected);
                (then_branch, else_branch, expected)
            }
            None => {
                let then_branch = self.infer_expr(then_branch);
                let then_ty = self.expr(then_branch).ty;
                if self.is_never_type(then_ty) {
                    let else_branch = self.infer_expr(else_branch);
                    let ty = self.expr(else_branch).ty;
                    (then_branch, else_branch, ty)
                } else {
                    let else_branch = self.check_expr(else_branch, then_ty);
                    (then_branch, else_branch, then_ty)
                }
            }
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            },
            span,
        })
    }

    pub(super) fn lower_match_expr(
        &mut self,
        id: HirExprId,
        scrutinee: HirExprId,
        arms: &[zutai_hir::HirClause],
        expected: Option<TypeId>,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let scrutinee_thir = self.infer_expr(scrutinee);
        let scrutinee_ty = self.expr(scrutinee_thir).ty;

        if arms.is_empty() {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnsupportedFeature {
                    feature: "empty match expressions",
                },
                span,
            });
            return self.error_expr(id, span);
        }

        let mut body_ty = expected;
        let mut fallback_never_ty = None;
        let mut lowered_arms = Vec::with_capacity(arms.len());

        for arm in arms {
            if arm.patterns.len() != 1 {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::MatchArmPatternCountMismatch {
                        found: arm.patterns.len(),
                    },
                    span: arm.span,
                });
            }

            let mut scoped_bindings = Vec::new();
            let patterns: Vec<_> = arm
                .patterns
                .iter()
                .map(|&pat_id| self.check_pattern(pat_id, scrutinee_ty, &mut scoped_bindings))
                .collect();

            let guard = arm.guard.map(|guard_id| {
                let bool_ty = self.bool_type(arm.span);
                self.check_expr(guard_id, bool_ty)
            });

            let body = match body_ty {
                Some(ty) => self.check_expr(arm.body, ty),
                None => {
                    let b = self.infer_expr(arm.body);
                    let inferred = self.expr(b).ty;
                    if self.is_never_type(inferred) {
                        fallback_never_ty.get_or_insert(inferred);
                    } else {
                        body_ty = Some(inferred);
                    }
                    b
                }
            };

            self.clear_scoped_value_types(&scoped_bindings);

            lowered_arms.push(crate::ir::ThirClause {
                patterns,
                guard,
                body,
                span: arm.span,
            });
        }

        // Only check coverage when every arm has the expected single pattern;
        // an arm-count mismatch already produced a diagnostic and would yield a
        // malformed matrix.
        if lowered_arms.iter().all(|arm| arm.patterns.len() == 1) {
            self.check_match_exhaustiveness(&lowered_arms, &[scrutinee_ty], span);
        }

        let ty = body_ty.or(fallback_never_ty).unwrap_or(self.error_type);
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Match {
                scrutinee: scrutinee_thir,
                arms: lowered_arms,
            },
            span,
        })
    }
}
