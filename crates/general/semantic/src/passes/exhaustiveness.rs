use std::collections::BTreeSet;

use zutai_hir::HirFile;
use zutai_hir::decl::HirDecl;
use zutai_hir::expr::{HirExprId, HirExprKind, HirTupleExprElem};
use zutai_hir::pat::{HirPatId, HirPatKind, HirTuplePatElem};
use zutai_hir::ty::LitVal;
use zutai_syntax::diag::ErrorCode;

use crate::context::AnalysisContext;
use crate::passes::Pass;
use crate::ty::{Ty, TyId, UNKNOWN_TY};

pub struct ExhaustivenessCheck;

impl Pass for ExhaustivenessCheck {
    fn name(&self) -> &'static str {
        "exhaustiveness"
    }

    fn run(&self, hir: &mut HirFile, ctx: &mut AnalysisContext) {
        let mut checker = ExhaustivenessChecker { hir, ctx };
        checker.check_file();
    }
}

struct ExhaustivenessChecker<'a> {
    hir: &'a HirFile,
    ctx: &'a mut AnalysisContext,
}

impl<'a> ExhaustivenessChecker<'a> {
    fn check_file(&mut self) {
        self.hir.decls.iter().for_each(|id| {
            let decl = self.hir.decls_arena.get(*id);

            if let HirDecl::Value { body, .. } | HirDecl::Function { body, .. } = decl {
                self.check_expr(*body);
            }
        });
    }

    fn check_expr(&mut self, expr_id: HirExprId) {
        let expr = self.hir.exprs.get(expr_id);

        match &expr.kind {
            HirExprKind::Record { fields } => fields.iter().for_each(|(_, e)| self.check_expr(*e)),
            HirExprKind::Tuple { items } => items.iter().for_each(|item| match item {
                HirTupleExprElem::Positional(e) | HirTupleExprElem::Named(_, e) => {
                    self.check_expr(*e)
                }
            }),
            HirExprKind::List { items } => items.iter().for_each(|item| self.check_expr(*item)),
            HirExprKind::Lambda { body, .. } => self.check_expr(*body),
            HirExprKind::Apply { fun, arg } => {
                self.check_expr(*fun);
                self.check_expr(*arg);
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(*lhs);
                self.check_expr(*rhs);
            }
            HirExprKind::Let { value, body, .. } => {
                self.check_expr(*value);
                self.check_expr(*body);
            }
            HirExprKind::If { cond, then_, else_ } => {
                self.check_expr(*cond);
                self.check_expr(*then_);
                self.check_expr(*else_);
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.check_match(expr_id, *scrutinee);
                self.check_expr(*scrutinee);

                arms.iter().for_each(|arm| {
                    if let Some(guard) = arm.guard {
                        self.check_expr(guard);
                    }
                    self.check_expr(arm.body);
                });
            }
            HirExprKind::Field { value, .. } => {
                self.check_expr(*value);
            }
            HirExprKind::Annot { expr, .. } => {
                self.check_expr(*expr);
            }

            HirExprKind::Import { .. }
            | HirExprKind::Lit(..)
            | HirExprKind::Var(..)
            | HirExprKind::Error => {}
        }
    }

    fn expr_ty(&self, expr_id: HirExprId) -> TyId {
        if let Some(ty) = self.ctx.expr_types.get(&expr_id) {
            return *ty;
        }

        let expr = self.hir.exprs.get(expr_id);

        match &expr.kind {
            HirExprKind::Var(sym) if !sym.is_error() => self
                .hir
                .symbols
                .get(*sym)
                .ty
                .map(|ty| TyId(ty.0))
                .unwrap_or(UNKNOWN_TY),

            _ => UNKNOWN_TY,
        }
    }

    // Extract finite union cases
    fn finite_cases(&self, ty: TyId) -> Option<BTreeSet<String>> {
        let Ty::Union(variants) = self.ctx.types.get(ty) else {
            return None;
        };

        let mut cases = BTreeSet::new();

        for variant in variants {
            match self.ctx.types.get(*variant) {
                Ty::Atom(atom) => {
                    cases.insert(atom.clone());
                }
                Ty::Tuple(_) => {
                    if let Some((tag, _payload)) = self
                        .ctx
                        .types
                        .get(*variant)
                        .as_tagged_tuple(&self.ctx.types)
                    {
                        cases.insert(tag.to_string());
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }

        if cases.is_empty() { None } else { Some(cases) }
    }

    fn pattern_coverage(&self, pat_id: HirPatId) -> Coverage {
        let pat = self.hir.pats.get(pat_id);

        match &pat.kind {
            HirPatKind::Wildcard | HirPatKind::Bind(_) => Coverage::All,

            HirPatKind::Literal(LitVal::Atom(atom)) => {
                Coverage::Cases(BTreeSet::from([atom.clone()]))
            }

            HirPatKind::Paren(inner) => self.pattern_coverage(*inner),

            HirPatKind::Tuple { items } => {
                let Some(HirTuplePatElem::Positional(first)) = items.first() else {
                    return Coverage::None;
                };
                self.pattern_coverage(*first)
            }

            HirPatKind::Literal(_) | HirPatKind::Record { .. } | HirPatKind::Error => {
                Coverage::None
            }
        }
    }

    fn check_match(&mut self, match_id: HirExprId, scrutinee: HirExprId) {
        let scrutinee_ty = self.expr_ty(scrutinee);
        let Some(expected) = self.finite_cases(scrutinee_ty) else {
            return;
        };

        let match_expr = self.hir.exprs.get(match_id);
        let HirExprKind::Match { arms, .. } = &match_expr.kind else {
            return;
        };

        let mut covered = BTreeSet::new();

        for arm in arms {
            if arm.guard.is_some() {
                continue;
            }

            match self.pattern_coverage(arm.pat) {
                Coverage::All => return,
                Coverage::Cases(cases) => {
                    covered.extend(cases);
                }
                Coverage::None => {}
            }
        }

        let missing: Vec<_> = expected.difference(&covered).cloned().collect();

        if !missing.is_empty() {
            self.ctx.error(
                match_expr.range,
                ErrorCode::NonExhaustiveMatch,
                format!(
                    "non-exhaustive match: missing {}",
                    missing
                        .iter()
                        .map(|case| format!("#{case}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    }
}

enum Coverage {
    All,
    Cases(BTreeSet<String>),
    None,
}
