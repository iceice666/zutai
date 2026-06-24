use rustc_hash::{FxHashMap, FxHashSet};

use la_arena::Arena;
use zutai_syntax::Span;
use zutai_syntax::ast;

use crate::diagnostic::{HirDiagnostic, HirDiagnosticKind};
use crate::ir::{
    Binding, BindingId, BindingKind, HirClause, HirConstraintMethod, HirDecl, HirDeclId,
    HirDeclKind, HirDeriveRecipe, HirEffectOp, HirEffectRow, HirExpr, HirExprId, HirExprKind,
    HirFile, HirHandleClause, HirHandleOp, HirImportSource, HirLevel, HirLocalBinding, HirPat,
    HirPatId, HirPatKind, HirRecordField, HirRecordPatField, HirRowTail, HirRowTailKind,
    HirSelectField, HirTupleItem, HirTuplePatItem, HirTypeExpr, HirTypeId, HirTypeKind,
    HirTypeParam, HirTypeRecordField, HirTypeTupleItem, HirUnionVariant, HirWitnessField,
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
    names: FxHashMap<String, BindingId>,
}

/// Tracks the lexically-nearest `handle` clause body during lowering so that
/// `resume` can be validated: it is legal only inside an *operation* clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandlerClauseKind {
    Value,
    Operation,
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
    constraint_method_bindings: FxHashMap<BindingId, Vec<Option<BindingId>>>,
    /// The lexically-nearest enclosing `handle` clause body, if any. `resume`
    /// is only valid when this is `Some(HandlerClauseKind::Operation)`.
    handler_clause: Option<HandlerClauseKind>,
    /// Level-parameter bindings (`<$l>`) referenced by a `$…` level use, used to
    /// report declared-but-unused level variables.
    used_level_params: FxHashSet<BindingId>,
}

mod decl;
mod expr;
mod types;

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
            constraint_method_bindings: FxHashMap::default(),
            handler_clause: None,
            used_level_params: FxHashSet::default(),
        };
        for name in [
            "Type",
            "Text",
            "Bool",
            "Int",
            "Float",
            "i8",
            "i16",
            "i32",
            "i64",
            "u8",
            "u16",
            "u32",
            "u64",
            "f32",
            "f64",
            "List",
            "Stream",
            "Optional",
            "Maybe",
            "Patch",
            "DeepPatch",
        ] {
            lowerer.define_current(name.to_string(), BindingKind::BuiltinType, file_span);
        }
        for name in crate::ir::HOST_CAPABILITY_TYPE_NAMES
            .iter()
            .chain(crate::ir::HOST_SUPPORT_TYPE_NAMES)
        {
            lowerer.define_current((*name).to_string(), BindingKind::BuiltinType, file_span);
        }
        for nbits in [32u8, 64] {
            lowerer.define_current(format!("Posit{nbits}"), BindingKind::BuiltinType, file_span);
            for es in 0..nbits {
                lowerer.define_current(
                    format!("Posit{nbits}e{es}"),
                    BindingKind::BuiltinType,
                    file_span,
                );
            }
        }
        for name in crate::ir::BUILTIN_VALUE_NAMES {
            lowerer.define_current((*name).to_string(), BindingKind::BuiltinValue, file_span);
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

    pub(super) fn define_current(
        &mut self,
        name: String,
        kind: BindingKind,
        span: Span,
    ) -> BindingId {
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

    pub(super) fn resolve(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).copied())
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
        debug_assert!(!self.scopes.is_empty());
    }

    pub(super) fn alloc_decl(&mut self, decl: HirDecl) -> HirDeclId {
        self.decl_arena.alloc(decl)
    }

    pub(super) fn alloc_expr(&mut self, expr: HirExpr) -> HirExprId {
        self.expr_arena.alloc(expr)
    }

    pub(super) fn alloc_pat(&mut self, pat: HirPat) -> HirPatId {
        self.pat_arena.alloc(pat)
    }

    pub(super) fn alloc_type(&mut self, ty: HirTypeExpr) -> HirTypeId {
        self.type_arena.alloc(ty)
    }
}
