use zutai_hir::{HirClause, HirDeclId, HirDeclKind};

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{ThirClause, ThirDecl, ThirDeclId, ThirDeclKind, TypeId};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn predeclare_decl_types(&mut self) {
        for decl_id in &self.hir.decls {
            let decl = self.hir_decl(*decl_id);
            match &decl.kind {
                HirDeclKind::TypeAlias { params, ty } => {
                    if !params.is_empty() {
                        self.unsupported("generic type aliases", decl.span);
                        continue;
                    }
                    let ty = self.lower_type(*ty);
                    self.aliases.insert(decl.binding, ty);
                    self.value_types.insert(decl.binding, self.type_type);
                }
                HirDeclKind::Value {
                    annotation: Some(annotation),
                    ..
                } => {
                    let ty = self.lower_type(*annotation);
                    self.value_types.insert(decl.binding, ty);
                }
                HirDeclKind::Function {
                    params,
                    sig: Some(sig),
                    ..
                } => {
                    if !params.is_empty() {
                        self.unsupported("generic function declarations", decl.span);
                        continue;
                    }
                    let sig = self.lower_type(*sig);
                    self.value_types.insert(decl.binding, sig);
                }
                HirDeclKind::Function { sig: None, .. } => {
                    self.unsupported("no-signature function declarations", decl.span);
                }
                HirDeclKind::Value {
                    annotation: None, ..
                } => {}
            }
        }
    }

    pub(super) fn lower_decl(&mut self, id: HirDeclId) -> ThirDeclId {
        let decl = self.hir_decl(id);
        let kind = match &decl.kind {
            HirDeclKind::TypeAlias { params, ty } => {
                let ty = self
                    .aliases
                    .get(&decl.binding)
                    .copied()
                    .unwrap_or_else(|| self.lower_type(*ty));
                ThirDeclKind::TypeAlias {
                    params: params.clone(),
                    ty,
                }
            }
            HirDeclKind::Value {
                annotation: Some(annotation),
                value,
            } => {
                let ty = self.lower_type(*annotation);
                let value = self.check_expr(*value, ty);
                self.value_types.insert(decl.binding, ty);
                ThirDeclKind::Value { ty, value }
            }
            HirDeclKind::Value {
                annotation: None,
                value,
            } => {
                let value = self.infer_expr(*value);
                let ty = self.expr(value).ty;
                self.value_types.insert(decl.binding, ty);
                ThirDeclKind::Value { ty, value }
            }
            HirDeclKind::Function {
                params,
                sig,
                clauses,
            } => {
                let sig = sig
                    .and_then(|_| self.value_types.get(&decl.binding).copied())
                    .unwrap_or(self.error_type);
                let clauses = if params.is_empty() && sig != self.error_type {
                    self.lower_function_clauses(clauses, sig)
                } else {
                    Vec::new()
                };
                ThirDeclKind::Function {
                    params: params.clone(),
                    sig,
                    clauses,
                }
            }
        };
        self.alloc_decl(ThirDecl {
            source: id,
            binding: decl.binding,
            kind,
            span: decl.span,
        })
    }

    fn lower_function_clauses(&mut self, clauses: &[HirClause], sig: TypeId) -> Vec<ThirClause> {
        let (param_types, return_type) = self.function_parts(sig, self.ty(sig).span);
        clauses
            .iter()
            .map(|clause| {
                if clause.patterns.len() != param_types.len() {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::FunctionClauseArityMismatch {
                            expected: param_types.len(),
                            found: clause.patterns.len(),
                        },
                        span: clause.span,
                    });
                }

                let mut scoped_bindings = Vec::new();
                let patterns = clause
                    .patterns
                    .iter()
                    .enumerate()
                    .map(|(index, pattern)| {
                        let expected = param_types.get(index).copied().unwrap_or(self.error_type);
                        self.check_pattern(*pattern, expected, &mut scoped_bindings)
                    })
                    .collect();
                let guard = clause.guard.map(|guard| {
                    let bool_ty = self.bool_type(clause.span);
                    self.check_expr(guard, bool_ty)
                });
                let body = self.check_expr(clause.body, return_type);
                self.clear_scoped_value_types(&scoped_bindings);

                ThirClause {
                    patterns,
                    guard,
                    body,
                    span: clause.span,
                }
            })
            .collect()
    }
}
