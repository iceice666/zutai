use std::collections::HashMap;

use la_arena::Arena;
use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{ThirFile, TypeId};

use crate::ir::{
    TlcDecl, TlcDeclId, TlcExpr, TlcExprId, TlcModule, TlcRecordField, TlcTupleField, TlcType,
    TlcTypeId, TlcTypeVar,
};

mod decl;
mod expr;
mod types;

pub fn lower_thir(file: &ThirFile) -> TlcModule {
    let mut lowerer = Lowerer::new(file);
    lowerer.lower_file()
}

struct Lowerer<'thir> {
    thir: &'thir ThirFile,
    decl_arena: Arena<TlcDecl>,
    expr_arena: Arena<TlcExpr>,
    type_arena: Arena<TlcType>,
    expr_types: HashMap<TlcExprId, TlcTypeId>,
    spans: HashMap<TlcExprId, Span>,
    type_cache: HashMap<u32, TlcTypeId>,
    infer_to_tyvar: HashMap<u32, TlcTypeVar>,
    named_to_tyvar: HashMap<u32, TlcTypeVar>,
    decl_thir_types: HashMap<BindingId, TypeId>,
    next_synth: u32,
}

impl<'thir> Lowerer<'thir> {
    fn new(thir: &'thir ThirFile) -> Self {
        Self {
            thir,
            decl_arena: Arena::new(),
            expr_arena: Arena::new(),
            type_arena: Arena::new(),
            expr_types: HashMap::new(),
            spans: HashMap::new(),
            type_cache: HashMap::new(),
            infer_to_tyvar: HashMap::new(),
            named_to_tyvar: HashMap::new(),
            decl_thir_types: HashMap::new(),
            next_synth: u32::MAX,
        }
    }

    fn lower_file(&mut self) -> TlcModule {
        self.collect_decl_types();
        let decls: Vec<TlcDeclId> = self
            .thir
            .decls
            .iter()
            .copied()
            .map(|id| self.lower_decl(id))
            .collect();
        TlcModule {
            decls,
            decl_arena: std::mem::take(&mut self.decl_arena),
            expr_arena: std::mem::take(&mut self.expr_arena),
            type_arena: std::mem::take(&mut self.type_arena),
            expr_types: std::mem::take(&mut self.expr_types),
            spans: std::mem::take(&mut self.spans),
        }
    }

    fn collect_decl_types(&mut self) {
        for &decl_id in &self.thir.decls {
            let decl = &self.thir.decl_arena[decl_id];
            let thir_ty = match &decl.kind {
                zutai_thir::ThirDeclKind::Value { ty, .. } => *ty,
                zutai_thir::ThirDeclKind::Function { sig, .. } => *sig,
                zutai_thir::ThirDeclKind::TypeAlias { .. } => continue,
            };
            self.decl_thir_types.insert(decl.binding, thir_ty);
        }
    }

    fn alloc_decl(&mut self, decl: TlcDecl) -> TlcDeclId {
        self.decl_arena.alloc(decl)
    }

    fn alloc_expr(&mut self, expr: TlcExpr, ty: TlcTypeId, span: Span) -> TlcExprId {
        let id = self.expr_arena.alloc(expr);
        self.expr_types.insert(id, ty);
        self.spans.insert(id, span);
        id
    }

    fn alloc_type(&mut self, ty: TlcType) -> TlcTypeId {
        self.type_arena.alloc(ty)
    }

    fn fresh_synth_binding(&mut self) -> BindingId {
        let id = self.next_synth;
        self.next_synth -= 1;
        BindingId(id)
    }
}
