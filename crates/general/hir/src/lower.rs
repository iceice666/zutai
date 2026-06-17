use std::collections::HashMap;

use la_arena::Arena;
use zutai_syntax::Span;
use zutai_syntax::ast;

use crate::diagnostic::{HirDiagnostic, HirDiagnosticKind};
use crate::ir::{
    Binding, BindingId, BindingKind, HirClause, HirConstraintMethod, HirDecl, HirDeclId,
    HirDeclKind, HirExpr, HirExprId, HirExprKind, HirFile, HirImportSource, HirLocalBinding,
    HirPat, HirPatId, HirPatKind, HirRecordField, HirRecordPatField, HirTupleItem, HirTuplePatItem,
    HirTypeExpr, HirTypeId, HirTypeKind, HirTypeParam, HirTypeRecordField, HirTypeTupleItem,
    HirUnionVariant, HirWitnessField,
};
use crate::pass::{HirPassReport, run_default_passes};

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredHir {
    pub file: HirFile,
    pub diagnostics: Vec<HirDiagnostic>,
    pub pass_reports: Vec<HirPassReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirLowerOptions {
    pub run_passes: bool,
}

impl Default for HirLowerOptions {
    fn default() -> Self {
        Self { run_passes: true }
    }
}

pub fn lower_file(file: &ast::File) -> LoweredHir {
    lower_file_with_options(file, HirLowerOptions::default())
}

pub fn lower_file_with_options(file: &ast::File, options: HirLowerOptions) -> LoweredHir {
    let mut lowerer = Lowerer::new(file.span);
    let mut lowered = lowerer.lower_file(file);
    if options.run_passes {
        lowered.pass_reports = run_default_passes(&mut lowered.file, &mut lowered.diagnostics);
    }
    lowered
}

#[derive(Default)]
struct Scope {
    names: HashMap<String, BindingId>,
}

struct Lowerer {
    bindings: Vec<Binding>,
    decl_arena: Arena<HirDecl>,
    expr_arena: Arena<HirExpr>,
    pat_arena: Arena<HirPat>,
    type_arena: Arena<HirTypeExpr>,
    scopes: Vec<Scope>,
    diagnostics: Vec<HirDiagnostic>,
    /// Maps each constraint's `BindingId` to the index-aligned vector of
    /// per-method bindings allocated in Pass 1.  `None` entries are operator methods.
    constraint_method_bindings: HashMap<BindingId, Vec<Option<BindingId>>>,
}

impl Lowerer {
    fn new(file_span: Span) -> Self {
        let mut lowerer = Self {
            bindings: Vec::new(),
            decl_arena: Arena::new(),
            expr_arena: Arena::new(),
            pat_arena: Arena::new(),
            type_arena: Arena::new(),
            scopes: vec![Scope::default()],
            diagnostics: Vec::new(),
            constraint_method_bindings: HashMap::new(),
        };
        for name in ["Type", "Text", "Bool", "Int", "Float", "List"] {
            lowerer.define_current(name.to_string(), BindingKind::BuiltinType, file_span);
        }
        lowerer
    }

    fn lower_file(&mut self, file: &ast::File) -> LoweredHir {
        let mut top_bindings = Vec::with_capacity(file.decls.len());
        for decl in &file.decls {
            top_bindings.push(self.define_top_decl(decl));
        }

        let decls = file
            .decls
            .iter()
            .zip(top_bindings)
            .map(|(decl, binding)| self.lower_decl(decl, binding))
            .collect();
        let final_expr = self.lower_expr(&file.final_expr);

        LoweredHir {
            file: HirFile {
                decls,
                final_expr,
                span: file.span,
                bindings: std::mem::take(&mut self.bindings),
                decl_arena: std::mem::take(&mut self.decl_arena),
                expr_arena: std::mem::take(&mut self.expr_arena),
                pat_arena: std::mem::take(&mut self.pat_arena),
                type_arena: std::mem::take(&mut self.type_arena),
            },
            diagnostics: std::mem::take(&mut self.diagnostics),
            pass_reports: Vec::new(),
        }
    }

    fn define_top_decl(&mut self, decl: &ast::Decl) -> BindingId {
        match decl {
            ast::Decl::Inferred { .. } | ast::Decl::Typed { .. } => {
                self.define_current(decl.name().to_string(), BindingKind::TopValue, decl.span())
            }
            ast::Decl::TypeAlias { .. } => {
                self.define_current(decl.name().to_string(), BindingKind::TopType, decl.span())
            }
            ast::Decl::Function { .. } | ast::Decl::NoSigFn { .. } => self.define_current(
                decl.name().to_string(),
                BindingKind::TopFunction,
                decl.span(),
            ),
            ast::Decl::Constraint { name, methods, .. } => {
                let constraint_binding =
                    self.define_current(name.clone(), BindingKind::TopConstraint, decl.span());
                // D1/D3: Allocate a BindingId for each *named* method now (Pass 1) so
                // method names are resolvable by any body lowered in Pass 2, regardless of
                // source order.  Operator methods get `None` (deferred to a later increment).
                let method_bindings: Vec<Option<BindingId>> = methods
                    .iter()
                    .map(|m| match &m.name {
                        ast::MethodName::Ident(method_name) => {
                            let id = self.define_current(
                                method_name.clone(),
                                BindingKind::ConstraintMethod,
                                m.span,
                            );
                            Some(id)
                        }
                        // D6 (4b): allocate an unscoped binding for operator methods so
                        // ThirConstraintMethod.binding is Some for operators too.
                        // Unscoped (not define_current) because operators are never
                        // referenced as bare idents and two constraints could share the
                        // same symbol name without a DuplicateBinding conflict.
                        ast::MethodName::Operator(op) => {
                            let id = self.alloc_binding_unscoped(
                                op.clone(),
                                BindingKind::ConstraintMethod,
                                m.span,
                            );
                            Some(id)
                        }
                    })
                    .collect();
                self.constraint_method_bindings
                    .insert(constraint_binding, method_bindings);
                constraint_binding
            }
            ast::Decl::Witness { constraint, .. } => {
                // D3: unscoped so duplicate witnesses don't raise DuplicateBinding
                self.alloc_binding_unscoped(
                    constraint.clone(),
                    BindingKind::TopWitness,
                    decl.span(),
                )
            }
        }
    }

    fn alloc_binding_unscoped(&mut self, name: String, kind: BindingKind, span: Span) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding { name, kind, span });
        id
    }

    fn lower_decl(&mut self, decl: &ast::Decl, binding: BindingId) -> HirDeclId {
        let (kind, span) = match decl {
            ast::Decl::Inferred { value, span, .. } => (
                HirDeclKind::Value {
                    annotation: None,
                    value: self.lower_expr(value),
                },
                *span,
            ),
            ast::Decl::Typed {
                ty, value, span, ..
            } => (
                HirDeclKind::Value {
                    annotation: Some(self.lower_type(ty)),
                    value: self.lower_expr(value),
                },
                *span,
            ),
            ast::Decl::TypeAlias {
                params, ty, span, ..
            } => {
                self.push_scope();
                let hir_params = self.lower_type_params(params);
                let ty = self.lower_type(ty);
                self.pop_scope();
                (
                    HirDeclKind::TypeAlias {
                        params: hir_params.into_iter().map(|p| p.binding).collect(),
                        ty,
                    },
                    *span,
                )
            }
            ast::Decl::Function {
                params,
                sig,
                clauses,
                span,
                ..
            } => {
                self.push_scope();
                let params = self.lower_type_params(params);
                let sig = self.lower_type(sig);
                let clauses = clauses
                    .iter()
                    .map(|clause| self.lower_clause(clause))
                    .collect();
                self.pop_scope();
                (
                    HirDeclKind::Function {
                        params,
                        sig: Some(sig),
                        clauses,
                    },
                    *span,
                )
            }
            ast::Decl::NoSigFn {
                patterns,
                body,
                span,
                ..
            } => {
                self.push_scope();
                let patterns = patterns.iter().map(|pat| self.lower_pattern(pat)).collect();
                let body = self.lower_expr(body);
                self.pop_scope();
                (
                    HirDeclKind::Function {
                        params: Vec::new(),
                        sig: None,
                        clauses: vec![HirClause {
                            patterns,
                            guard: None,
                            body,
                            span: *span,
                        }],
                    },
                    *span,
                )
            }
            ast::Decl::Constraint {
                params,
                target,
                methods,
                derivable,
                span,
                ..
            } => {
                self.push_scope();
                let hir_params = self.lower_hir_type_params(params);
                let hir_target = self.lower_type(target);
                let mut seen_methods: HashMap<String, Span> = HashMap::new();
                let mut hir_methods = Vec::new();
                for (idx, method) in methods.iter().enumerate() {
                    let key = method.name.as_str().to_string();
                    if let Some(&first_span) = seen_methods.get(&key) {
                        self.diagnostics.push(HirDiagnostic {
                            kind: HirDiagnosticKind::DuplicateConstraintMethod {
                                name: key.clone(),
                                first_span,
                            },
                            span: method.span,
                        });
                    } else {
                        seen_methods.insert(key.clone(), method.span);
                    }
                    self.push_scope();
                    let method_params = self.lower_hir_type_params(&method.params);
                    let sig = self.lower_type(&method.sig);
                    let default = method
                        .default
                        .iter()
                        .map(|c| self.lower_clause(c))
                        .collect();
                    self.pop_scope();
                    let (is_operator, name_str) = match &method.name {
                        ast::MethodName::Ident(s) => (false, s.clone()),
                        ast::MethodName::Operator(s) => (true, s.clone()),
                    };
                    // D3: retrieve the pre-allocated binding from the threaded map.
                    let method_binding = self
                        .constraint_method_bindings
                        .get(&binding)
                        .and_then(|v| v.get(idx))
                        .copied()
                        .flatten();
                    hir_methods.push(HirConstraintMethod {
                        name: name_str,
                        is_operator,
                        optional: method.optional,
                        params: method_params,
                        sig,
                        default,
                        span: method.span,
                        binding: method_binding,
                    });
                }
                self.pop_scope();
                (
                    HirDeclKind::Constraint {
                        params: hir_params,
                        target: hir_target,
                        methods: hir_methods,
                        derivable: *derivable,
                    },
                    *span,
                )
            }
            ast::Decl::Witness {
                constraint,
                target,
                params,
                body,
                span,
            } => {
                let constraint_binding = match self.resolve(constraint) {
                    Some(bid) => Some(bid),
                    None => {
                        self.diagnostics.push(HirDiagnostic {
                            kind: HirDiagnosticKind::UnknownConstraint {
                                name: constraint.clone(),
                            },
                            span: *span,
                        });
                        None
                    }
                };
                self.push_scope();
                let hir_params = self.lower_hir_type_params(params);
                let hir_target = self.lower_type(target);
                let (hir_fields, derive) = match body {
                    ast::WitnessBody::Derive => (Vec::new(), true),
                    ast::WitnessBody::Fields(fields) => {
                        let mut seen: HashMap<String, Span> = HashMap::new();
                        let mut hir_fields = Vec::new();
                        for field in fields {
                            let key = field.name.as_str().to_string();
                            if let Some(&first_span) = seen.get(&key) {
                                self.diagnostics.push(HirDiagnostic {
                                    kind: HirDiagnosticKind::DuplicateWitnessField {
                                        name: key.clone(),
                                        first_span,
                                    },
                                    span: field.span,
                                });
                            } else {
                                seen.insert(key.clone(), field.span);
                            }
                            let (is_operator, name_str) = match &field.name {
                                ast::MethodName::Ident(s) => (false, s.clone()),
                                ast::MethodName::Operator(s) => (true, s.clone()),
                            };
                            let value = self.lower_expr(&field.value);
                            hir_fields.push(HirWitnessField {
                                name: name_str,
                                is_operator,
                                value,
                                span: field.span,
                            });
                        }
                        (hir_fields, false)
                    }
                };
                self.pop_scope();
                (
                    HirDeclKind::Witness {
                        constraint: constraint_binding,
                        target: hir_target,
                        params: hir_params,
                        fields: hir_fields,
                        derive,
                    },
                    *span,
                )
            }
        };
        self.alloc_decl(HirDecl {
            binding,
            kind,
            span,
        })
    }

    fn lower_type_params(&mut self, params: &[ast::TypeParam]) -> Vec<HirTypeParam> {
        // First pass: allocate all BindingIds so that forward-references within
        // the param list are handled correctly (mirrors lower_hir_type_params).
        let bindings: Vec<BindingId> = params
            .iter()
            .map(|param| {
                self.define_current(param.name.clone(), BindingKind::TypeParam, param.span)
            })
            .collect();
        // Second pass: resolve bounds, storing them (was D1 resolve-but-don't-store).
        bindings
            .into_iter()
            .zip(params)
            .map(|(binding, param)| {
                let bounds: Vec<BindingId> = param
                    .bounds
                    .iter()
                    .filter_map(|bound| match self.resolve(&bound.name) {
                        Some(bid) => Some(bid),
                        None => {
                            self.diagnostics.push(HirDiagnostic {
                                kind: HirDiagnosticKind::UnknownIdentifier {
                                    name: bound.name.clone(),
                                },
                                span: bound.span,
                            });
                            None
                        }
                    })
                    .collect();
                let kind = param.kind.as_ref().map(|k| self.lower_type(k));
                HirTypeParam {
                    binding,
                    bounds,
                    kind,
                    span: param.span,
                }
            })
            .collect()
    }

    /// Lower type params for constraint/witness decls: creates `HirTypeParam` with
    /// resolved bounds and lowered kind annotations.
    fn lower_hir_type_params(&mut self, params: &[ast::TypeParam]) -> Vec<HirTypeParam> {
        params
            .iter()
            .map(|param| {
                let binding =
                    self.define_current(param.name.clone(), BindingKind::TypeParam, param.span);
                let bounds: Vec<BindingId> = param
                    .bounds
                    .iter()
                    .filter_map(|bound| match self.resolve(&bound.name) {
                        Some(bid) => Some(bid),
                        None => {
                            self.diagnostics.push(HirDiagnostic {
                                kind: HirDiagnosticKind::UnknownIdentifier {
                                    name: bound.name.clone(),
                                },
                                span: bound.span,
                            });
                            None
                        }
                    })
                    .collect();
                let kind = param.kind.as_ref().map(|k| self.lower_type(k));
                HirTypeParam {
                    binding,
                    bounds,
                    kind,
                    span: param.span,
                }
            })
            .collect()
    }

    fn lower_clause(&mut self, clause: &ast::FuncClause) -> HirClause {
        self.push_scope();
        let patterns = clause
            .patterns
            .iter()
            .map(|pat| self.lower_pattern(pat))
            .collect();
        let guard = clause.guard.as_ref().map(|guard| self.lower_expr(guard));
        let body = self.lower_expr(&clause.body);
        self.pop_scope();
        HirClause {
            patterns,
            guard,
            body,
            span: clause.span,
        }
    }

    fn lower_expr(&mut self, expr: &ast::Expr) -> HirExprId {
        let span = expr.span();
        let kind = match expr {
            ast::Expr::True(_) => HirExprKind::True,
            ast::Expr::False(_) => HirExprKind::False,
            ast::Expr::Integer { value, .. } => HirExprKind::Integer(*value),
            ast::Expr::Float { value, .. } => HirExprKind::Float(*value),
            ast::Expr::String { value, .. } => HirExprKind::String(value.clone()),
            ast::Expr::Atom { name, .. } => HirExprKind::Atom(name.clone()),
            ast::Expr::TaggedValue { tag, payload, .. } => HirExprKind::TaggedValue {
                tag: tag.clone(),
                payload: self.lower_expr(payload),
            },
            ast::Expr::Ident { name, span } => self.lower_ident(name, *span),
            ast::Expr::Record { fields, .. } => HirExprKind::Record(
                fields
                    .iter()
                    .map(|field| HirRecordField {
                        name: field.name.clone(),
                        value: self.lower_expr(&field.value),
                        span: field.span,
                    })
                    .collect(),
            ),
            ast::Expr::Tuple { items, .. } => HirExprKind::Tuple(
                items
                    .iter()
                    .map(|item| match item {
                        ast::TupleItem::Named { name, value, span } => HirTupleItem::Named {
                            name: name.clone(),
                            value: self.lower_expr(value),
                            span: *span,
                        },
                        ast::TupleItem::Positional(value) => {
                            HirTupleItem::Positional(self.lower_expr(value))
                        }
                    })
                    .collect(),
            ),
            ast::Expr::List { items, .. } => {
                HirExprKind::List(items.iter().map(|item| self.lower_expr(item)).collect())
            }
            ast::Expr::Block {
                bindings, result, ..
            } => {
                self.push_scope();
                let bindings = bindings
                    .iter()
                    .map(|binding| {
                        let value = self.lower_expr(&binding.value);
                        let binding_id = self.define_current(
                            binding.name.clone(),
                            BindingKind::Local,
                            binding.span,
                        );
                        HirLocalBinding {
                            binding: binding_id,
                            value,
                            span: binding.span,
                        }
                    })
                    .collect();
                let result = self.lower_expr(result);
                self.pop_scope();
                HirExprKind::Block { bindings, result }
            }
            ast::Expr::Lambda { params, body, .. } => {
                self.push_scope();
                let params = params
                    .iter()
                    .map(|param| self.lower_pattern(param))
                    .collect();
                let body = self.lower_expr(body);
                self.pop_scope();
                HirExprKind::Lambda { params, body }
            }
            ast::Expr::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => HirExprKind::If {
                cond: self.lower_expr(cond),
                then_branch: self.lower_expr(then_branch),
                else_branch: self.lower_expr(else_branch),
            },
            ast::Expr::Match {
                scrutinee, arms, ..
            } => HirExprKind::Match {
                scrutinee: self.lower_expr(scrutinee),
                arms: arms.iter().map(|arm| self.lower_clause(arm)).collect(),
            },
            ast::Expr::Import { source, .. } => HirExprKind::Import(clone_import_source(source)),
            ast::Expr::TypeForm { ty, .. } => HirExprKind::TypeForm(self.lower_type(ty)),
            ast::Expr::Apply { func, arg, .. } => HirExprKind::Apply {
                func: self.lower_expr(func),
                arg: self.lower_expr(arg),
            },
            ast::Expr::Access {
                receiver, field, ..
            } => HirExprKind::Access {
                receiver: self.lower_expr(receiver),
                field: field.clone(),
            },
            ast::Expr::OptAccess {
                receiver, field, ..
            } => HirExprKind::OptAccess {
                receiver: self.lower_expr(receiver),
                field: field.clone(),
            },
            ast::Expr::Binary { op, lhs, rhs, .. } => HirExprKind::Binary {
                op: *op,
                lhs: self.lower_expr(lhs),
                rhs: self.lower_expr(rhs),
            },
            ast::Expr::Pipeline { dir, lhs, rhs, .. } => {
                let lhs = self.lower_expr(lhs);
                let rhs = self.lower_expr(rhs);
                match dir {
                    ast::PipelineDir::Forward => HirExprKind::Apply {
                        func: rhs,
                        arg: lhs,
                    },
                    ast::PipelineDir::Backward => HirExprKind::Apply {
                        func: lhs,
                        arg: rhs,
                    },
                }
            }
        };
        self.alloc_expr(HirExpr { kind, span })
    }

    fn lower_ident(&mut self, name: &str, span: Span) -> HirExprKind {
        match self.resolve(name) {
            Some(binding) => HirExprKind::BindingRef(binding),
            None => {
                self.diagnostics.push(HirDiagnostic {
                    kind: HirDiagnosticKind::UnknownIdentifier {
                        name: name.to_string(),
                    },
                    span,
                });
                HirExprKind::UnresolvedIdent(name.to_string())
            }
        }
    }

    fn lower_pattern(&mut self, pattern: &ast::Pattern) -> HirPatId {
        let span = pattern.span();
        let kind = match pattern {
            ast::Pattern::Wildcard(_) => HirPatKind::Wildcard,
            ast::Pattern::Ident { name, span } => {
                let binding = self.define_current(name.clone(), BindingKind::Param, *span);
                HirPatKind::Bind(binding)
            }
            ast::Pattern::True(_) => HirPatKind::True,
            ast::Pattern::False(_) => HirPatKind::False,
            ast::Pattern::Integer { value, .. } => HirPatKind::Integer(*value),
            ast::Pattern::Float { value, .. } => HirPatKind::Float(*value),
            ast::Pattern::String { value, .. } => HirPatKind::String(value.clone()),
            ast::Pattern::Atom { name, .. } => HirPatKind::Atom(name.clone()),
            ast::Pattern::TaggedValue { tag, payload, .. } => HirPatKind::TaggedValue {
                tag: tag.clone(),
                payload: payload
                    .iter()
                    .map(|field| HirRecordPatField {
                        name: field.name.clone(),
                        pattern: self.lower_pattern(&field.pattern),
                        span: field.span,
                    })
                    .collect(),
            },
            ast::Pattern::Tuple { items, .. } => HirPatKind::Tuple(
                items
                    .iter()
                    .map(|item| match item {
                        ast::TuplePatternItem::Named {
                            name,
                            pattern,
                            span,
                        } => HirTuplePatItem::Named {
                            name: name.clone(),
                            pattern: self.lower_pattern(pattern),
                            span: *span,
                        },
                        ast::TuplePatternItem::Positional(pattern) => {
                            HirTuplePatItem::Positional(self.lower_pattern(pattern))
                        }
                    })
                    .collect(),
            ),
            ast::Pattern::Record { fields, .. } => HirPatKind::Record(
                fields
                    .iter()
                    .map(|field| HirRecordPatField {
                        name: field.name.clone(),
                        pattern: self.lower_pattern(&field.pattern),
                        span: field.span,
                    })
                    .collect(),
            ),
        };
        self.alloc_pat(HirPat { kind, span })
    }

    fn lower_type(&mut self, ty: &ast::TypeExpr) -> HirTypeId {
        let span = ty.span();
        let kind = match ty {
            ast::TypeExpr::Ident { name, span } => match self.resolve(name) {
                Some(binding) => HirTypeKind::BindingRef(binding),
                None => {
                    self.diagnostics.push(HirDiagnostic {
                        kind: HirDiagnosticKind::UnknownIdentifier {
                            name: name.to_string(),
                        },
                        span: *span,
                    });
                    HirTypeKind::UnresolvedIdent(name.clone())
                }
            },
            ast::TypeExpr::Record { fields, .. } => HirTypeKind::Record(
                fields
                    .iter()
                    .map(|field| HirTypeRecordField {
                        name: field.name.clone(),
                        optional: field.optional,
                        ty: self.lower_type(&field.ty),
                        span: field.span,
                    })
                    .collect(),
            ),
            ast::TypeExpr::Union { variants, .. } => HirTypeKind::Union(
                variants
                    .iter()
                    .map(|v| HirUnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.as_ref().map(|fields| {
                            fields
                                .iter()
                                .map(|field| HirTypeRecordField {
                                    name: field.name.clone(),
                                    optional: field.optional,
                                    ty: self.lower_type(&field.ty),
                                    span: field.span,
                                })
                                .collect()
                        }),
                        span: v.span,
                    })
                    .collect(),
            ),
            ast::TypeExpr::Tuple { items, .. } => HirTypeKind::Tuple(
                items
                    .iter()
                    .map(|item| match item {
                        ast::TypeTupleItem::Named { name, ty, span } => HirTypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.lower_type(ty),
                            span: *span,
                        },
                        ast::TypeTupleItem::Positional(ty) => {
                            HirTypeTupleItem::Positional(self.lower_type(ty))
                        }
                    })
                    .collect(),
            ),
            ast::TypeExpr::Optional { inner, .. } => HirTypeKind::Optional(self.lower_type(inner)),
            ast::TypeExpr::Arrow { from, to, .. } => HirTypeKind::Arrow {
                from: self.lower_type(from),
                to: self.lower_type(to),
            },
            ast::TypeExpr::Apply { func, arg, .. } => HirTypeKind::Apply {
                func: self.lower_type(func),
                arg: self.lower_type(arg),
            },
            ast::TypeExpr::Access {
                receiver, field, ..
            } => HirTypeKind::Access {
                receiver: self.lower_type(receiver),
                field: field.clone(),
            },
            ast::TypeExpr::Atom { name, .. } => HirTypeKind::Atom(name.clone()),
            ast::TypeExpr::True(_) => HirTypeKind::True,
            ast::TypeExpr::False(_) => HirTypeKind::False,
            ast::TypeExpr::ExprEscape(expr) => HirTypeKind::ExprEscape(self.lower_expr(expr)),
        };
        self.alloc_type(HirTypeExpr { kind, span })
    }

    fn define_current(&mut self, name: String, kind: BindingKind, span: Span) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        let scope = self.scopes.last_mut().expect("scope stack is never empty");
        if let Some(first) = scope.names.get(&name).copied() {
            self.diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateBinding {
                    name: name.clone(),
                    first_span: self.bindings[first.0 as usize].span,
                },
                span,
            });
        } else {
            scope.names.insert(name.clone(), id);
        }
        self.bindings.push(Binding { name, kind, span });
        id
    }

    fn resolve(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).copied())
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        debug_assert!(!self.scopes.is_empty());
    }

    fn alloc_decl(&mut self, decl: HirDecl) -> HirDeclId {
        self.decl_arena.alloc(decl)
    }

    fn alloc_expr(&mut self, expr: HirExpr) -> HirExprId {
        self.expr_arena.alloc(expr)
    }

    fn alloc_pat(&mut self, pat: HirPat) -> HirPatId {
        self.pat_arena.alloc(pat)
    }

    fn alloc_type(&mut self, ty: HirTypeExpr) -> HirTypeId {
        self.type_arena.alloc(ty)
    }
}

fn clone_import_source(source: &ast::ImportSource) -> HirImportSource {
    match source {
        ast::ImportSource::String(value) => HirImportSource::String(value.clone()),
        ast::ImportSource::Path(parts) => HirImportSource::Path(parts.clone()),
    }
}
