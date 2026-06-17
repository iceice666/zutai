use zutai_hir::{HirClause, HirDeclId, HirDeclKind, HirExprKind};

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirClause, ThirConstraintMethod, ThirDecl, ThirDeclId, ThirDeclKind, ThirWitnessField, Type,
    TypeId, TypeKind,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn predeclare_decl_types(&mut self) {
        for decl_id in &self.hir.decls {
            let decl = self.hir_decl(*decl_id);
            match &decl.kind {
                HirDeclKind::TypeAlias { params, ty } => {
                    let ty = self.lower_type(*ty);
                    self.aliases.insert(decl.binding, ty);
                    if !params.is_empty() {
                        // Generic alias: record the params so use sites can build
                        // AliasApply nodes and resolve_alias can expand them.
                        self.alias_params.insert(decl.binding, params.clone());
                    }
                    self.value_types.insert(decl.binding, self.type_type);
                }
                HirDeclKind::Value {
                    annotation: Some(annotation),
                    ..
                } => {
                    let ty = self.lower_type(*annotation);
                    self.value_types.insert(decl.binding, ty);
                }
                HirDeclKind::Function { sig: Some(sig), .. } => {
                    // Works for both monomorphic (params=[]) and generic (params non-empty):
                    // type params are BindingKind::TypeParam and lower to TypeKind::TypeVar.
                    let sig = self.lower_type(*sig);
                    self.value_types.insert(decl.binding, sig);
                }
                HirDeclKind::Function {
                    sig: None, clauses, ..
                } => {
                    // No-signature inference: assign fresh InferVars for each
                    // parameter position and an InferVar for the return type.
                    // Unification during clause lowering will solve them.
                    let arity = clauses.first().map(|c| c.patterns.len()).unwrap_or(0);
                    let span = decl.span;
                    let param_vars: Vec<TypeId> =
                        (0..arity).map(|_| self.fresh_infer_var(span)).collect();
                    let ret_var = self.fresh_infer_var(span);
                    let sig = param_vars.iter().rev().fold(ret_var, |to, &from| {
                        self.alloc_type(Type {
                            kind: TypeKind::Function { from, to },
                            span,
                        })
                    });
                    self.value_types.insert(decl.binding, sig);
                }
                HirDeclKind::Value {
                    annotation: None, ..
                } => {}
                // D4: register each named method's signature so that method-name
                // BindingRefs are resolvable via the normal `value_types` path.
                HirDeclKind::Constraint { methods, .. } => {
                    for m in methods {
                        if let Some(b) = m.binding {
                            let ty = self.lower_type(m.sig);
                            self.value_types.insert(b, ty);
                        }
                    }
                }
                // Witness decls contribute no value bindings.
                HirDeclKind::Witness { .. } => continue,
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
                // Track import-binding associations for annotation-position access.
                if let HirExprKind::Import(source) = &self.hir_expr(*value).kind {
                    self.binding_import_key.insert(decl.binding, source.clone());
                }
                let ty = self.lower_type(*annotation);
                let value = self.check_expr(*value, ty);
                self.value_types.insert(decl.binding, ty);
                ThirDeclKind::Value { ty, value }
            }
            HirDeclKind::Value {
                annotation: None,
                value,
            } => {
                // Track import-binding associations for annotation-position access.
                if let HirExprKind::Import(source) = &self.hir_expr(*value).kind {
                    self.binding_import_key.insert(decl.binding, source.clone());
                }
                let value = self.infer_expr(*value);
                let ty = self.expr(value).ty;
                self.value_types.insert(decl.binding, ty);
                self.generalize_if_polymorphic(decl.binding, ty);
                ThirDeclKind::Value { ty, value }
            }
            HirDeclKind::Function {
                params, clauses, ..
            } => {
                // Use whatever sig was pre-declared: explicit annotation lowered
                // to its type, or InferVar chain for no-signature functions.
                let sig = self
                    .value_types
                    .get(&decl.binding)
                    .copied()
                    .unwrap_or(self.error_type);
                let clauses = if sig != self.error_type {
                    let clauses = self.lower_function_clauses(clauses, sig);
                    self.generalize_if_polymorphic(decl.binding, sig);
                    clauses
                } else {
                    Vec::new()
                };
                ThirDeclKind::Function {
                    params: params.clone(),
                    sig,
                    clauses,
                }
            }
            // D2′: Constraint/Witness decls are now lowered to THIR (no longer filtered).
            // Method sigs use `lower_type`; witness field values use `infer_expr`.
            // Method-level params and default bodies are dropped (deferred to D6).
            // Increment 3 (check_witnesses) is implemented and runs after cw-lowering.
            HirDeclKind::Constraint {
                params,
                target,
                methods,
                derivable,
            } => {
                let target = self.lower_type(*target);
                let params: Vec<_> = params.iter().map(|p| p.binding).collect();
                let methods: Vec<ThirConstraintMethod> = methods
                    .iter()
                    .map(|m| ThirConstraintMethod {
                        name: m.name.clone(),
                        is_operator: m.is_operator,
                        optional: m.optional,
                        sig: self.lower_type(m.sig),
                        span: m.span,
                        binding: m.binding,
                    })
                    .collect();
                ThirDeclKind::Constraint {
                    params,
                    target,
                    methods,
                    derivable: *derivable,
                }
            }
            HirDeclKind::Witness {
                constraint,
                target,
                params,
                fields,
                derive,
            } => {
                let target = self.lower_type(*target);
                let params: Vec<_> = params.iter().map(|p| p.binding).collect();
                let fields: Vec<ThirWitnessField> = fields
                    .iter()
                    .map(|f| ThirWitnessField {
                        name: f.name.clone(),
                        is_operator: f.is_operator,
                        value: self.infer_expr(f.value),
                        span: f.span,
                    })
                    .collect();
                ThirDeclKind::Witness {
                    constraint: *constraint,
                    target,
                    params,
                    fields,
                    derive: *derive,
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
        let sig_span = self.ty(sig).span;
        let (param_types, return_type) = self.function_parts(sig, sig_span);
        let lowered: Vec<ThirClause> = clauses
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
            .collect();

        // Only check coverage when every clause matches the function arity; a
        // clause-arity mismatch already produced a diagnostic.
        if lowered
            .iter()
            .all(|clause| clause.patterns.len() == param_types.len())
        {
            self.check_match_exhaustiveness(&lowered, &param_types, sig_span);
        }

        lowered
    }
}
