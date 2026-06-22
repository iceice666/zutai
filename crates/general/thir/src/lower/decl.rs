use zutai_hir::{BindingKind, HirClause, HirDeclId, HirDeclKind, HirExprKind};

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::import::ImportedType;

use crate::ir::{
    ThirClause, ThirConstraintMethod, ThirDecl, ThirDeclId, ThirDeclKind, ThirDeriveRecipe,
    ThirWitnessField, Type, TypeId, TypeKind,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn predeclare_import_decls(&mut self) {
        for decl_id in &self.hir.decls {
            let decl = self.hir_decl(*decl_id);
            if self.hir.bindings[decl.binding.0 as usize].kind != BindingKind::TopImport {
                continue;
            }
            let HirDeclKind::Value { value, .. } = &decl.kind else {
                continue;
            };
            let HirExprKind::Import(source) = &self.hir_expr(*value).kind else {
                continue;
            };
            let Some(desc) = self.imports.get(source).cloned() else {
                continue;
            };

            let ty = self.intern_imported_type_with_source(&desc, Some(source), decl.span);
            self.value_types.insert(decl.binding, ty);
            self.binding_import_key.insert(decl.binding, source.clone());

            if let ImportedType::Type(inner) = desc {
                let denotation = self.intern_imported_type_with_source(&inner, None, decl.span);
                self.aliases.insert(decl.binding, denotation);
            }
        }
    }

    pub(super) fn predeclare_decl_types(&mut self) {
        // Pass 1: register every parametric alias's arity before lowering any
        // body. Generic self- and mutually-recursive aliases (e.g.
        // `Tree :: <A> type { #node : { left : Tree A; ... }; ... }`) reference
        // their own/each other's constructor inside the body, so `lower_type_apply`
        // must already see the arity to build an `AliasApply` node instead of
        // rejecting `Tree A` as "not a parametric constructor". Definition order
        // is irrelevant because every alias is known before any body is lowered.
        for decl_id in &self.hir.decls {
            let decl = self.hir_decl(*decl_id);
            if let HirDeclKind::TypeAlias { params, .. } = &decl.kind
                && !params.is_empty()
            {
                self.alias_params.insert(decl.binding, params.clone());
            }
        }
        for decl_id in &self.hir.decls {
            let decl = self.hir_decl(*decl_id);
            match &decl.kind {
                HirDeclKind::TypeAlias { ty, .. } => {
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
                // Partial application is only legal in witness targets; an
                // under-applied constructor buried in an alias body (e.g. a field
                // typed `Pair Int`) must still be diagnosed.
                self.require_ground_type(ty, decl.span);
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
                self.require_ground_type(ty, decl.span);
                if self.is_non_function_effect_type(ty) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::UnsupportedFeature {
                            feature: "effectful top-level value bindings",
                        },
                        span: decl.span,
                    });
                }
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
                let binding_kind = self.hir.bindings[decl.binding.0 as usize].kind;
                let predeclared_import_ty = if binding_kind == BindingKind::TopImport {
                    self.value_types.get(&decl.binding).copied()
                } else {
                    None
                };
                let value = if let Some(ty) = predeclared_import_ty {
                    self.check_expr(*value, ty)
                } else {
                    self.infer_expr(*value)
                };
                let ty = predeclared_import_ty.unwrap_or_else(|| self.expr(value).ty);
                self.value_types.insert(decl.binding, ty);
                if binding_kind != BindingKind::TopImport {
                    self.generalize_if_polymorphic(decl.binding, ty);
                }
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
                    self.require_ground_type(sig, decl.span);
                    let clauses = self.lower_function_clauses(clauses, sig);
                    self.generalize_if_polymorphic(decl.binding, sig);
                    clauses
                } else {
                    Vec::new()
                };
                let param_bounds: Vec<Vec<zutai_hir::BindingId>> =
                    params.iter().map(|p| p.bounds.clone()).collect();
                let params: Vec<zutai_hir::BindingId> = params.iter().map(|p| p.binding).collect();
                ThirDeclKind::Function {
                    params,
                    param_bounds,
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
                recipe,
            } => {
                let target = self.lower_type(*target);
                let params: Vec<_> = params.iter().map(|p| p.binding).collect();
                let methods: Vec<ThirConstraintMethod> = methods
                    .iter()
                    .map(|m| {
                        let sig = self.lower_type(m.sig);
                        self.require_ground_type(sig, m.span);
                        // D6/4a: lower default clause body if present.
                        // Use lower_function_clauses against the method sig so the
                        // clauses are type-checked. Skip when empty (no default).
                        let default = if m.default.is_empty() {
                            None
                        } else {
                            Some(self.lower_function_clauses(&m.default, sig))
                        };
                        ThirConstraintMethod {
                            name: m.name.clone(),
                            is_operator: m.is_operator,
                            optional: m.optional,
                            sig,
                            params: m.params.iter().map(|p| p.binding).collect(),
                            param_bounds: m.params.iter().map(|p| p.bounds.clone()).collect(),
                            span: m.span,
                            binding: m.binding,
                            default,
                        }
                    })
                    .collect();
                let recipe = recipe.as_ref().map(|recipe| {
                    let body = self.infer_expr(recipe.body);
                    ThirDeriveRecipe {
                        params: recipe.params.iter().map(|param| param.binding).collect(),
                        body,
                        span: recipe.span,
                    }
                });
                ThirDeclKind::Constraint {
                    params,
                    target,
                    methods,
                    derivable: *derivable,
                    recipe,
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
                let param_bounds: Vec<Vec<zutai_hir::BindingId>> =
                    params.iter().map(|p| p.bounds.clone()).collect();
                let params: Vec<_> = params.iter().map(|p| p.binding).collect();
                // Check each field against its constraint method's signature
                // (instantiated at the witness target) instead of inferring it
                // blind. This lets a witness field reference a bounded helper whose
                // own type param must alias the witness param — e.g.
                // `Eq @(Pair A) :: <A: Eq> { eq = pairEq; }` — and gives inline
                // lambda fields their parameter types so field access resolves.
                let method_sigs = constraint
                    .map(|c| self.witness_method_sigs(c, target))
                    .unwrap_or_default();
                let mut thir_fields: Vec<ThirWitnessField> = Vec::with_capacity(fields.len());
                for f in fields {
                    let value = match method_sigs.get(&f.name) {
                        Some(&expected) => self.check_expr(f.value, expected),
                        None => self.infer_expr(f.value),
                    };
                    thir_fields.push(ThirWitnessField {
                        name: f.name.clone(),
                        is_operator: f.is_operator,
                        value,
                        span: f.span,
                    });
                }
                let fields = thir_fields;
                ThirDeclKind::Witness {
                    constraint: *constraint,
                    target,
                    params,
                    param_bounds,
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
    /// Build the expected witness field signatures for `constraint` at `target`:
    /// each constraint method's signature with the single constraint type param
    /// substituted by the witness target. Requires the constraint decl to be
    /// already lowered (constraints are lowered before witnesses). Returns an
    /// empty map for multi-param constraints (handled elsewhere).
    pub(super) fn witness_method_sigs(
        &mut self,
        constraint: zutai_hir::BindingId,
        target: TypeId,
    ) -> std::collections::HashMap<String, TypeId> {
        let info = self.decl_arena.iter().find_map(|(_, decl)| {
            if decl.binding == constraint
                && let ThirDeclKind::Constraint {
                    params, methods, ..
                } = &decl.kind
            {
                let sigs: Vec<(String, TypeId, Vec<zutai_hir::BindingId>)> = methods
                    .iter()
                    .map(|m| (m.name.clone(), m.sig, m.params.clone()))
                    .collect();
                return Some((params.clone(), sigs));
            }
            None
        });
        let Some((params, sigs)) = info else {
            return std::collections::HashMap::new();
        };
        if params.len() != 1 {
            return std::collections::HashMap::new();
        }
        sigs.into_iter()
            .map(|(name, sig, method_params)| {
                let mut subst: std::collections::HashMap<zutai_hir::BindingId, TypeId> =
                    [(params[0], target)].into_iter().collect();
                // Method-level type params become fresh InferVars per field so a
                // witness implementation need not be parametric in them (e.g.
                // `Functor @List { map = \f xs. xs; }` checks even though it is not
                // `(A->B)->List A->List B`). Gate-scoped leniency: eval/compile
                // still refuse HKT execution, so no unsound value is produced.
                let span = self.ty(sig).span;
                for &mp in &method_params {
                    let fresh = self.fresh_infer_var(span);
                    subst.insert(mp, fresh);
                }
                (name, self.instantiate_type_vars(sig, &subst))
            })
            .collect()
    }

    fn lower_function_clauses(&mut self, clauses: &[HirClause], sig: TypeId) -> Vec<ThirClause> {
        let sig_span = self.ty(sig).span;
        let (param_types, return_type) = self.function_parts(sig, sig_span);
        let (body_type, saved_effect_ambient) = self.enter_effectful_result(return_type);
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
                let body = self.check_expr(clause.body, body_type);
                self.clear_scoped_value_types(&scoped_bindings);

                ThirClause {
                    patterns,
                    guard,
                    body,
                    span: clause.span,
                }
            })
            .collect();

        self.exit_effectful_result(saved_effect_ambient);

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
