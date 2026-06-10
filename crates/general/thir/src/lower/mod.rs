use std::collections::HashMap;

use zutai_hir::{
    BindingId, BindingKind, HirDecl, HirDeclId, HirExpr, HirExprId, HirFile, HirPat, HirPatId,
    HirTypeExpr, HirTypeId,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirDecl, ThirDeclId, ThirExpr, ThirExprId, ThirExprKind, ThirFile, ThirPat, ThirPatId, Type,
    TypeId, TypeKind, TypeTupleItem,
};
use crate::pass::{ThirPassReport, run_default_passes};

mod decl;
mod expr;
mod pat;
mod types;

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
    next_infer_var: u32,
    infer_subst: HashMap<u32, TypeId>,
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
            next_infer_var: 0,
            infer_subst: HashMap::new(),
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

        // Zonk: replace solved InferVar slots in the type arena with their
        // concrete types so downstream consumers see fully-resolved types.
        self.zonk_type_arena();

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

    // ── Inference / unification ──────────────────────────────────────────────

    pub(super) fn fresh_infer_var(&mut self, span: Span) -> TypeId {
        let id = self.next_infer_var;
        self.next_infer_var += 1;
        self.alloc_type(Type {
            kind: TypeKind::InferVar(id),
            span,
        })
    }

    /// Chase InferVar substitution chains to find the canonical representative.
    pub(super) fn resolve(&self, ty: TypeId) -> TypeId {
        let mut current = ty;
        loop {
            match self.type_arena[current.0 as usize].kind {
                TypeKind::InferVar(v) => {
                    if let Some(&next) = self.infer_subst.get(&v) {
                        current = next;
                    } else {
                        return current;
                    }
                }
                _ => return current,
            }
        }
    }

    /// Occurs check: true if `var_id` appears free in `ty`.
    pub(super) fn occurs(&self, var_id: u32, ty: TypeId) -> bool {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::InferVar(v) => v == var_id,
            TypeKind::Function { from, to } => self.occurs(var_id, from) || self.occurs(var_id, to),
            TypeKind::List(inner) | TypeKind::Optional(inner) => self.occurs(var_id, inner),
            TypeKind::Union(items) => items.iter().any(|&item| self.occurs(var_id, item)),
            TypeKind::Tuple(items) => items.iter().any(|item| {
                let inner = match item {
                    TypeTupleItem::Named { ty, .. } => *ty,
                    TypeTupleItem::Positional(ty) => *ty,
                };
                self.occurs(var_id, inner)
            }),
            TypeKind::Record(fields) => fields.iter().any(|f| self.occurs(var_id, f.ty)),
            _ => false,
        }
    }

    /// Structural unification of two types.  Solves InferVars in `infer_subst`.
    /// Reports a `TypeMismatch` diagnostic for rigid conflicts.
    pub(super) fn unify(&mut self, t1: TypeId, t2: TypeId, span: Span) {
        let t1 = self.resolve(t1);
        let t2 = self.resolve(t2);
        if t1 == t2 {
            return;
        }

        let k1 = self.type_arena[t1.0 as usize].kind.clone();
        let k2 = self.type_arena[t2.0 as usize].kind.clone();

        match (k1, k2) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => {}

            (TypeKind::InferVar(v), _) => {
                if !self.occurs(v, t2) {
                    self.infer_subst.insert(v, t2);
                }
            }

            (_, TypeKind::InferVar(v)) => {
                if !self.occurs(v, t1) {
                    self.infer_subst.insert(v, t1);
                }
            }

            (TypeKind::Function { from: f1, to: r1 }, TypeKind::Function { from: f2, to: r2 }) => {
                self.unify(f1, f2, span);
                self.unify(r1, r2, span);
            }

            (TypeKind::List(e1), TypeKind::List(e2)) => self.unify(e1, e2, span),

            (TypeKind::Optional(e1), TypeKind::Optional(e2)) => self.unify(e1, e2, span),

            (left, right) => {
                if left != right {
                    self.type_mismatch(t1, t2, span);
                }
            }
        }
    }

    /// Zonk: for every solved InferVar slot in the type arena, overwrite it
    /// with the kind of its resolved type so callers see concrete types without
    /// having to chase substitution chains.
    fn zonk_type_arena(&mut self) {
        for i in 0..self.type_arena.len() {
            if matches!(self.type_arena[i].kind, TypeKind::InferVar(_)) {
                let resolved = self.resolve(TypeId(i as u32));
                if resolved.0 as usize != i {
                    let resolved_kind = self.type_arena[resolved.0 as usize].kind.clone();
                    self.type_arena[i].kind = resolved_kind;
                }
            }
        }
    }
}
