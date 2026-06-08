use std::collections::HashMap;

use zutai_hir::{
    BindingId, BindingKind, HirDecl, HirDeclId, HirExpr, HirExprId, HirFile, HirPat, HirPatId,
    HirTypeExpr, HirTypeId,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    ThirDecl, ThirDeclId, ThirExpr, ThirExprId, ThirExprKind, ThirFile, ThirPat, ThirPatId, Type,
    TypeId, TypeKind,
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
}
