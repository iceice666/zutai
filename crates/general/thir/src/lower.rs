use std::collections::{HashMap, HashSet};

use zutai_hir::{
    BindingId, BindingKind, HirDecl, HirDeclId, HirDeclKind, HirExpr, HirExprId, HirExprKind,
    HirFile, HirLocalBinding, HirPat, HirPatId, HirPatKind, HirRecordField, HirTypeExpr, HirTypeId,
    HirTypeKind, HirTypeRecordField, HirTypeTupleItem,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirClause, ThirDecl, ThirDeclId, ThirDeclKind, ThirExpr, ThirExprId, ThirExprKind, ThirFile,
    ThirLocalBinding, ThirPat, ThirPatId, ThirPatKind, ThirRecordField, Type, TypeId, TypeKind,
    TypeRecordField, TypeTupleItem,
};
use crate::pass::{ThirPassReport, run_default_passes};

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredThir {
    pub file: Option<ThirFile>,
    pub diagnostics: Vec<ThirDiagnostic>,
    pub pass_reports: Vec<ThirPassReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThirLowerOptions {
    pub run_passes: bool,
}

impl Default for ThirLowerOptions {
    fn default() -> Self {
        Self { run_passes: true }
    }
}

pub fn lower_hir(file: &zutai_hir::HirFile) -> LoweredThir {
    lower_hir_with_options(file, ThirLowerOptions::default())
}

pub fn lower_hir_with_options(file: &zutai_hir::HirFile, options: ThirLowerOptions) -> LoweredThir {
    let mut lowerer = Lowerer::new(file);
    let mut lowered = lowerer.lower_file();
    if options.run_passes {
        lowered.pass_reports = run_default_passes(&mut lowered.file, &mut lowered.diagnostics);
    }
    lowered
}

struct Lowerer<'hir> {
    hir: &'hir HirFile,
    decl_arena: Vec<ThirDecl>,
    expr_arena: Vec<ThirExpr>,
    pat_arena: Vec<ThirPat>,
    type_arena: Vec<Type>,
    aliases: HashMap<BindingId, TypeId>,
    value_types: HashMap<BindingId, TypeId>,
    diagnostics: Vec<ThirDiagnostic>,
    error_type: TypeId,
    type_type: TypeId,
}

impl<'hir> Lowerer<'hir> {
    fn new(hir: &'hir HirFile) -> Self {
        let mut lowerer = Self {
            hir,
            decl_arena: Vec::new(),
            expr_arena: Vec::new(),
            pat_arena: Vec::new(),
            type_arena: Vec::new(),
            aliases: HashMap::new(),
            value_types: HashMap::new(),
            diagnostics: Vec::new(),
            error_type: TypeId(0),
            type_type: TypeId(0),
        };
        lowerer.error_type = lowerer.alloc_type(Type {
            kind: TypeKind::Error,
            span: Span::default(),
        });
        lowerer.type_type = lowerer.alloc_type(Type {
            kind: TypeKind::Type,
            span: hir.span,
        });
        lowerer.seed_builtin_value_types();
        lowerer
    }

    fn lower_file(&mut self) -> LoweredThir {
        self.predeclare_decl_types();
        let decls: Vec<_> = self
            .hir
            .decls
            .iter()
            .copied()
            .map(|id| self.lower_decl(id))
            .collect();
        let final_expr = self.infer_expr(self.hir.final_expr);

        let file = ThirFile {
            decls,
            final_expr,
            decl_arena: std::mem::take(&mut self.decl_arena),
            expr_arena: std::mem::take(&mut self.expr_arena),
            pat_arena: std::mem::take(&mut self.pat_arena),
            type_arena: std::mem::take(&mut self.type_arena),
        };
        let diagnostics = std::mem::take(&mut self.diagnostics);

        LoweredThir {
            file: diagnostics.is_empty().then_some(file),
            diagnostics,
            pass_reports: Vec::new(),
        }
    }

    fn seed_builtin_value_types(&mut self) {
        for (index, binding) in self.hir.bindings.iter().enumerate() {
            if binding.kind == BindingKind::BuiltinType {
                self.value_types
                    .insert(BindingId(index as u32), self.type_type);
            }
        }
    }

    fn predeclare_decl_types(&mut self) {
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

    fn lower_decl(&mut self, id: HirDeclId) -> ThirDeclId {
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

    fn check_expr(&mut self, id: HirExprId, expected: TypeId) -> ThirExprId {
        let expr = self.hir_expr(id);
        match &expr.kind {
            HirExprKind::Record(fields) => self.check_record_expr(id, fields, expected),
            HirExprKind::Block { bindings, result } => {
                self.lower_block_expr(id, bindings, *result, Some(expected))
            }
            _ => {
                let lowered = self.infer_expr(id);
                let found = self.expr(lowered).ty;
                if !self.type_matches(expected, found) {
                    self.type_mismatch(expected, found, expr.span);
                }
                lowered
            }
        }
    }

    fn infer_expr(&mut self, id: HirExprId) -> ThirExprId {
        let expr = self.hir_expr(id);
        match &expr.kind {
            HirExprKind::True => {
                let ty = self.bool_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::True,
                    span: expr.span,
                })
            }
            HirExprKind::False => {
                let ty = self.bool_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::False,
                    span: expr.span,
                })
            }
            HirExprKind::Integer(value) => {
                let ty = self.int_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Integer(*value),
                    span: expr.span,
                })
            }
            HirExprKind::Float(value) => {
                let ty = self.float_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Float(*value),
                    span: expr.span,
                })
            }
            HirExprKind::String(value) => {
                let ty = self.text_type(expr.span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::String(value.clone()),
                    span: expr.span,
                })
            }
            HirExprKind::Atom(name) => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::Atom(name.clone()),
                    span: expr.span,
                });
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Atom(name.clone()),
                    span: expr.span,
                })
            }
            HirExprKind::BindingRef(binding) => self.lower_binding_ref(id, *binding, expr.span),
            HirExprKind::Record(fields) => self.infer_record_expr(id, fields, expr.span),
            HirExprKind::TypeForm(ty) => {
                let value = self.lower_type(*ty);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty: self.type_type,
                    kind: ThirExprKind::TypeValue(value),
                    span: expr.span,
                })
            }
            HirExprKind::Access { receiver, field } => {
                self.lower_access_expr(id, *receiver, field, expr.span)
            }
            HirExprKind::Block { bindings, result } => {
                self.lower_block_expr(id, bindings, *result, None)
            }
            HirExprKind::Apply { func, arg } => self.lower_apply_expr(id, *func, *arg, expr.span),
            HirExprKind::UnresolvedIdent(name) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ValueTypeUnavailable { name: name.clone() },
                    span: expr.span,
                });
                self.error_expr(id, expr.span)
            }
            HirExprKind::Tuple(_) => self.unsupported_expr(id, "tuple expressions", expr.span),
            HirExprKind::List(_) => self.unsupported_expr(id, "list expressions", expr.span),
            HirExprKind::Lambda { .. } => {
                self.unsupported_expr(id, "lambda expressions", expr.span)
            }
            HirExprKind::If { .. } => self.unsupported_expr(id, "if expressions", expr.span),
            HirExprKind::Match { .. } => self.unsupported_expr(id, "match expressions", expr.span),
            HirExprKind::Import(_) => self.unsupported_expr(id, "imports", expr.span),
            HirExprKind::OptAccess { .. } => {
                self.unsupported_expr(id, "optional access expressions", expr.span)
            }
            HirExprKind::Binary { .. } => {
                self.unsupported_expr(id, "binary expressions", expr.span)
            }
        }
    }

    fn lower_function_clauses(
        &mut self,
        clauses: &[zutai_hir::HirClause],
        sig: TypeId,
    ) -> Vec<ThirClause> {
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

    fn lower_block_expr(
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

    fn lower_apply_expr(
        &mut self,
        id: HirExprId,
        func: HirExprId,
        arg: HirExprId,
        span: Span,
    ) -> ThirExprId {
        let func = self.infer_expr(func);
        let func_ty = self.expr(func).ty;
        let Some((from, to)) = self.function_input_output(func_ty, span) else {
            let found = self.type_name(func_ty);
            if !matches!(self.ty(func_ty).kind, TypeKind::Error) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExpectedFunction { found },
                    span,
                });
            }
            let arg = self.infer_expr(arg);
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.error_type,
                kind: ThirExprKind::Apply {
                    func,
                    arg,
                    instantiation: Vec::new(),
                },
                span,
            });
        };
        let arg = self.check_expr(arg, from);
        self.alloc_expr(ThirExpr {
            source: id,
            ty: to,
            kind: ThirExprKind::Apply {
                func,
                arg,
                instantiation: Vec::new(),
            },
            span,
        })
    }

    fn check_pattern(
        &mut self,
        id: HirPatId,
        expected: TypeId,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> ThirPatId {
        let pattern = self.hir_pat(id);
        let kind = match &pattern.kind {
            HirPatKind::Wildcard => ThirPatKind::Wildcard,
            HirPatKind::Bind(binding) => {
                self.value_types.insert(*binding, expected);
                scoped_bindings.push(*binding);
                ThirPatKind::Bind(*binding)
            }
            HirPatKind::True => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::True,
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::True
            }
            HirPatKind::False => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::False,
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::False
            }
            HirPatKind::Integer(value) => {
                let ty = self.int_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Integer(*value)
            }
            HirPatKind::Float(value) => {
                let ty = self.float_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Float(*value)
            }
            HirPatKind::String(value) => {
                let ty = self.text_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::String(value.clone())
            }
            HirPatKind::Atom(name) => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::Atom(name.clone()),
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Atom(name.clone())
            }
            HirPatKind::Tuple(_) => {
                self.unsupported("tuple patterns", pattern.span);
                ThirPatKind::Error
            }
            HirPatKind::Record(_) => {
                self.unsupported("record patterns", pattern.span);
                ThirPatKind::Error
            }
        };
        self.alloc_pat(ThirPat {
            source: id,
            ty: expected,
            kind,
            span: pattern.span,
        })
    }

    fn check_pattern_type(&mut self, expected: TypeId, found: TypeId, span: Span) {
        if !self.type_matches(expected, found) {
            self.type_mismatch(expected, found, span);
        }
    }

    fn clear_scoped_value_types(&mut self, scoped_bindings: &[BindingId]) {
        for binding in scoped_bindings {
            self.value_types.remove(binding);
        }
    }

    fn lower_binding_ref(&mut self, id: HirExprId, binding: BindingId, span: Span) -> ThirExprId {
        let binding_info = &self.hir.bindings[binding.0 as usize];
        if matches!(
            binding_info.kind,
            BindingKind::BuiltinType | BindingKind::TopType
        ) {
            let value = if binding_info.kind == BindingKind::TopType {
                self.alias_type(binding, span)
            } else {
                self.builtin_type_by_name(&binding_info.name, span)
                    .unwrap_or(self.error_type)
            };
            return self.alloc_expr(ThirExpr {
                source: id,
                ty: self.type_type,
                kind: ThirExprKind::TypeValue(value),
                span,
            });
        }

        match self.value_types.get(&binding).copied() {
            Some(ty) => self.alloc_expr(ThirExpr {
                source: id,
                ty,
                kind: ThirExprKind::BindingRef(binding),
                span,
            }),
            None => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ValueTypeUnavailable {
                        name: binding_info.name.clone(),
                    },
                    span,
                });
                self.error_expr(id, span)
            }
        }
    }

    fn infer_record_expr(
        &mut self,
        id: HirExprId,
        fields: &[HirRecordField],
        span: Span,
    ) -> ThirExprId {
        let mut thir_fields = Vec::with_capacity(fields.len());
        let mut type_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let value = self.infer_expr(field.value);
            let ty = self.expr(value).ty;
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
            type_fields.push(TypeRecordField {
                name: field.name.clone(),
                optional: false,
                ty,
                span: field.span,
            });
        }
        let ty = self.alloc_type(Type {
            kind: TypeKind::Record(type_fields),
            span,
        });
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    fn check_record_expr(
        &mut self,
        id: HirExprId,
        fields: &[HirRecordField],
        expected: TypeId,
    ) -> ThirExprId {
        let span = self.hir_expr(id).span;
        let Some(expected_fields) = self.record_fields(expected, span) else {
            let found = self.type_name(expected);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.infer_record_expr(id, fields, span);
        };

        let expected_by_name: HashMap<_, _> = expected_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        let actual_names: HashSet<_> = fields.iter().map(|field| field.name.as_str()).collect();

        for expected_field in &expected_fields {
            if !expected_field.optional && !actual_names.contains(expected_field.name.as_str()) {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::MissingRecordField {
                        name: expected_field.name.clone(),
                    },
                    span,
                });
            }
        }

        let mut thir_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(expected_field) = expected_by_name.get(field.name.as_str()) else {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::UnexpectedRecordField {
                        name: field.name.clone(),
                    },
                    span: field.span,
                });
                let value = self.infer_expr(field.value);
                thir_fields.push(ThirRecordField {
                    name: field.name.clone(),
                    value,
                    span: field.span,
                });
                continue;
            };
            let value = self.check_expr(field.value, expected_field.ty);
            thir_fields.push(ThirRecordField {
                name: field.name.clone(),
                value,
                span: field.span,
            });
        }

        self.alloc_expr(ThirExpr {
            source: id,
            ty: expected,
            kind: ThirExprKind::Record(thir_fields),
            span,
        })
    }

    fn lower_access_expr(
        &mut self,
        id: HirExprId,
        receiver: HirExprId,
        field: &str,
        span: Span,
    ) -> ThirExprId {
        let receiver = self.infer_expr(receiver);
        let receiver_ty = self.expr(receiver).ty;
        let Some(fields) = self.record_fields(receiver_ty, span) else {
            let found = self.type_name(receiver_ty);
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::ExpectedRecord { found },
                span,
            });
            return self.error_expr(id, span);
        };
        let Some(record_field) = fields.iter().find(|candidate| candidate.name == field) else {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnknownField {
                    name: field.to_string(),
                },
                span,
            });
            return self.error_expr(id, span);
        };
        let ty = if record_field.optional {
            self.optional_type(record_field.ty, record_field.span)
        } else {
            record_field.ty
        };
        self.alloc_expr(ThirExpr {
            source: id,
            ty,
            kind: ThirExprKind::Access {
                receiver,
                field: field.to_string(),
            },
            span,
        })
    }

    fn lower_type(&mut self, id: HirTypeId) -> TypeId {
        let ty = self.hir_type(id);
        match &ty.kind {
            HirTypeKind::BindingRef(binding) => self.alias_or_builtin_type(*binding, ty.span),
            HirTypeKind::Record(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| self.lower_type_record_field(field))
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Record(fields),
                    span: ty.span,
                })
            }
            HirTypeKind::Union(items) => {
                let items = items.iter().map(|item| self.lower_type(*item)).collect();
                self.alloc_type(Type {
                    kind: TypeKind::Union(items),
                    span: ty.span,
                })
            }
            HirTypeKind::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        HirTypeTupleItem::Named { name, ty, span } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.lower_type(*ty),
                            span: *span,
                        },
                        HirTypeTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.lower_type(*ty))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(items),
                    span: ty.span,
                })
            }
            HirTypeKind::Optional(inner) => {
                let inner = self.lower_type(*inner);
                self.optional_type(inner, ty.span)
            }
            HirTypeKind::Arrow { from, to } => {
                let from = self.lower_type(*from);
                let to = self.lower_type(*to);
                self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span: ty.span,
                })
            }
            HirTypeKind::Apply { func, arg } => self.lower_type_apply(*func, *arg, ty.span),
            HirTypeKind::Atom(name) => self.alloc_type(Type {
                kind: TypeKind::Atom(name.clone()),
                span: ty.span,
            }),
            HirTypeKind::True => self.alloc_type(Type {
                kind: TypeKind::True,
                span: ty.span,
            }),
            HirTypeKind::False => self.alloc_type(Type {
                kind: TypeKind::False,
                span: ty.span,
            }),
            HirTypeKind::UnresolvedIdent(_) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::InvalidTypeExpression {
                        reason: "unresolved type identifier",
                    },
                    span: ty.span,
                });
                self.error_type
            }
            HirTypeKind::Access { .. } => {
                self.invalid_type("type field access is not supported yet", ty.span)
            }
            HirTypeKind::ExprEscape(_) => {
                self.invalid_type("type expression escapes are not supported yet", ty.span)
            }
        }
    }

    fn lower_type_record_field(&mut self, field: &HirTypeRecordField) -> TypeRecordField {
        TypeRecordField {
            name: field.name.clone(),
            optional: field.optional,
            ty: self.lower_type(field.ty),
            span: field.span,
        }
    }

    fn lower_type_apply(&mut self, func: HirTypeId, arg: HirTypeId, span: Span) -> TypeId {
        let func_ty = self.hir_type(func);
        let arg = self.lower_type(arg);
        let HirTypeKind::BindingRef(binding) = func_ty.kind else {
            return self.invalid_type("only built-in type constructors are supported yet", span);
        };
        let name = &self.hir.bindings[binding.0 as usize].name;
        match name.as_str() {
            "List" => self.alloc_type(Type {
                kind: TypeKind::List(arg),
                span,
            }),
            "Optional" => self.optional_type(arg, span),
            _ => self.invalid_type("generic type application is not supported yet", span),
        }
    }

    fn alias_or_builtin_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        let binding_info = &self.hir.bindings[binding.0 as usize];
        match binding_info.kind {
            BindingKind::BuiltinType => self
                .builtin_type_by_name(&binding_info.name, span)
                .unwrap_or_else(|| self.invalid_type("unknown built-in type", span)),
            BindingKind::TopType => self.alias_type(binding, span),
            BindingKind::TypeParam => self.alloc_type(Type {
                kind: TypeKind::TypeVar(binding),
                span,
            }),
            _ => self.invalid_type("value binding used as a type", span),
        }
    }

    fn builtin_type_by_name(&mut self, name: &str, span: Span) -> Option<TypeId> {
        let kind = match name {
            "Type" => TypeKind::Type,
            "Text" => TypeKind::Text,
            "Bool" => TypeKind::Bool,
            "Int" => TypeKind::Int,
            "Float" => TypeKind::Float,
            _ => return None,
        };
        Some(self.alloc_type(Type { kind, span }))
    }

    fn alias_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Alias(binding),
            span,
        })
    }

    fn bool_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Bool,
            span,
        })
    }

    fn int_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Int,
            span,
        })
    }

    fn float_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Float,
            span,
        })
    }

    fn text_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Text,
            span,
        })
    }

    fn optional_type(&mut self, inner: TypeId, span: Span) -> TypeId {
        let normalized = self.resolve_alias(inner, &mut HashSet::new(), span);
        if matches!(self.ty(normalized).kind, TypeKind::Optional(_)) {
            return normalized;
        }
        self.alloc_type(Type {
            kind: TypeKind::Optional(inner),
            span,
        })
    }

    fn record_fields(&mut self, ty: TypeId, span: Span) -> Option<Vec<TypeRecordField>> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match &self.ty(resolved).kind {
            TypeKind::Record(fields) => Some(fields.clone()),
            _ => None,
        }
    }

    fn function_input_output(&mut self, ty: TypeId, span: Span) -> Option<(TypeId, TypeId)> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind {
            TypeKind::Function { from, to } => Some((from, to)),
            _ => None,
        }
    }

    fn function_parts(&mut self, ty: TypeId, span: Span) -> (Vec<TypeId>, TypeId) {
        let mut params = Vec::new();
        let mut current = ty;
        loop {
            let resolved = self.resolve_alias(current, &mut HashSet::new(), span);
            match self.ty(resolved).kind {
                TypeKind::Function { from, to } => {
                    params.push(from);
                    current = to;
                }
                _ => return (params, resolved),
            }
        }
    }

    fn type_matches(&mut self, expected: TypeId, found: TypeId) -> bool {
        let expected = self.resolve_alias(expected, &mut HashSet::new(), self.ty(expected).span);
        let found = self.resolve_alias(found, &mut HashSet::new(), self.ty(found).span);
        if expected == found {
            return true;
        }

        match (self.ty(expected).kind.clone(), self.ty(found).kind.clone()) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => true,
            (TypeKind::Bool, TypeKind::True | TypeKind::False) => true,
            (TypeKind::Union(items), _) => items
                .iter()
                .copied()
                .any(|item| self.type_matches(item, found)),
            (TypeKind::List(expected), TypeKind::List(found))
            | (TypeKind::Optional(expected), TypeKind::Optional(found)) => {
                self.type_matches(expected, found)
            }
            (TypeKind::Record(expected_fields), TypeKind::Record(found_fields)) => {
                self.record_types_match(&expected_fields, &found_fields)
            }
            (TypeKind::Tuple(expected_items), TypeKind::Tuple(found_items)) => {
                self.tuple_types_match(&expected_items, &found_items)
            }
            (
                TypeKind::Function {
                    from: expected_from,
                    to: expected_to,
                },
                TypeKind::Function {
                    from: found_from,
                    to: found_to,
                },
            ) => {
                self.type_matches(expected_from, found_from)
                    && self.type_matches(expected_to, found_to)
            }
            (left, right) => left == right,
        }
    }

    fn record_types_match(
        &mut self,
        expected_fields: &[TypeRecordField],
        found_fields: &[TypeRecordField],
    ) -> bool {
        let found_by_name: HashMap<_, _> = found_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        for expected in expected_fields {
            let Some(found) = found_by_name.get(expected.name.as_str()) else {
                if expected.optional {
                    continue;
                }
                return false;
            };
            if !self.type_matches(expected.ty, found.ty) {
                return false;
            }
        }
        found_fields
            .iter()
            .all(|found| expected_fields.iter().any(|field| field.name == found.name))
    }

    fn tuple_types_match(
        &mut self,
        expected_items: &[TypeTupleItem],
        found_items: &[TypeTupleItem],
    ) -> bool {
        if expected_items.len() != found_items.len() {
            return false;
        }
        expected_items
            .iter()
            .zip(found_items)
            .all(|(expected, found)| match (expected, found) {
                (TypeTupleItem::Positional(expected), TypeTupleItem::Positional(found)) => {
                    self.type_matches(*expected, *found)
                }
                (
                    TypeTupleItem::Named {
                        name: expected_name,
                        ty: expected,
                        ..
                    },
                    TypeTupleItem::Named {
                        name: found_name,
                        ty: found,
                        ..
                    },
                ) if expected_name == found_name => self.type_matches(*expected, *found),
                _ => false,
            })
    }

    fn resolve_alias(&mut self, ty: TypeId, seen: &mut HashSet<BindingId>, span: Span) -> TypeId {
        let TypeKind::Alias(binding) = self.ty(ty).kind else {
            return ty;
        };
        if !seen.insert(binding) {
            let name = self.hir.bindings[binding.0 as usize].name.clone();
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::AliasCycle { name },
                span,
            });
            return self.error_type;
        }
        match self.aliases.get(&binding).copied() {
            Some(alias) => self.resolve_alias(alias, seen, span),
            None => ty,
        }
    }

    fn type_name(&mut self, ty: TypeId) -> String {
        let ty = self.resolve_alias(ty, &mut HashSet::new(), self.ty(ty).span);
        match self.ty(ty).kind.clone() {
            TypeKind::Type => "Type".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Text => "Text".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Atom(name) => format!("#{name}"),
            TypeKind::True => "true".to_string(),
            TypeKind::False => "false".to_string(),
            TypeKind::List(inner) => format!("List {}", self.type_name(inner)),
            TypeKind::Optional(inner) => format!("{}?", self.type_name(inner)),
            TypeKind::Record(_) => "record".to_string(),
            TypeKind::Union(_) => "union".to_string(),
            TypeKind::Tuple(_) => "tuple".to_string(),
            TypeKind::Function { .. } => "function".to_string(),
            TypeKind::TypeVar(binding) | TypeKind::Alias(binding) => {
                self.hir.bindings[binding.0 as usize].name.clone()
            }
            TypeKind::Error => "<error>".to_string(),
        }
    }

    fn type_mismatch(&mut self, expected: TypeId, found: TypeId, span: Span) {
        let expected = self.type_name(expected);
        let found = self.type_name(found);
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::TypeMismatch { expected, found },
            span,
        });
    }

    fn unsupported_expr(&mut self, id: HirExprId, feature: &'static str, span: Span) -> ThirExprId {
        self.unsupported(feature, span);
        self.error_expr(id, span)
    }

    fn unsupported(&mut self, feature: &'static str, span: Span) {
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::UnsupportedFeature { feature },
            span,
        });
    }

    fn invalid_type(&mut self, reason: &'static str, span: Span) -> TypeId {
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::InvalidTypeExpression { reason },
            span,
        });
        self.error_type
    }

    fn error_expr(&mut self, source: HirExprId, span: Span) -> ThirExprId {
        self.alloc_expr(ThirExpr {
            source,
            ty: self.error_type,
            kind: ThirExprKind::Error,
            span,
        })
    }

    fn hir_decl(&self, id: HirDeclId) -> &'hir HirDecl {
        &self.hir.decl_arena[id.0 as usize]
    }

    fn hir_expr(&self, id: HirExprId) -> &'hir HirExpr {
        &self.hir.expr_arena[id.0 as usize]
    }

    fn hir_type(&self, id: HirTypeId) -> &'hir HirTypeExpr {
        &self.hir.type_arena[id.0 as usize]
    }

    fn hir_pat(&self, id: HirPatId) -> &'hir HirPat {
        &self.hir.pat_arena[id.0 as usize]
    }

    fn expr(&self, id: ThirExprId) -> &ThirExpr {
        &self.expr_arena[id.0 as usize]
    }

    fn ty(&self, id: TypeId) -> &Type {
        &self.type_arena[id.0 as usize]
    }

    fn alloc_decl(&mut self, decl: ThirDecl) -> ThirDeclId {
        let id = ThirDeclId(self.decl_arena.len() as u32);
        self.decl_arena.push(decl);
        id
    }

    fn alloc_expr(&mut self, expr: ThirExpr) -> ThirExprId {
        let id = ThirExprId(self.expr_arena.len() as u32);
        self.expr_arena.push(expr);
        id
    }

    fn alloc_pat(&mut self, pat: ThirPat) -> ThirPatId {
        let id = ThirPatId(self.pat_arena.len() as u32);
        self.pat_arena.push(pat);
        id
    }

    fn alloc_type(&mut self, ty: Type) -> TypeId {
        let id = TypeId(self.type_arena.len() as u32);
        self.type_arena.push(ty);
        id
    }
}
