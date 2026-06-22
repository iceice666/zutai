use rustc_hash::{FxHashMap, FxHashSet};

use indexmap::IndexMap;
use la_arena::Arena;
use zutai_hir::{Binding, BindingId, HirImportSource};
use zutai_syntax::Span;
use zutai_tlc::{
    BuiltinOp, Literal as TlcLit, Row, TlcDecl, TlcExprId, TlcModule, TlcPat, TlcPatItem, TlcType,
    TlcTypeId, TlcTypeVar,
};

use crate::{
    DataflowGraph, DfBuiltinOp, DfLit, DfNode, DfNodeKind, DfPositOp, DfRecordField, DfTupleField,
    DfTy, DfTyId, DfTyVar, ImportKind, NodeId,
};

mod expr;
mod pat;
mod types;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn lower_tyvar(v: TlcTypeVar) -> DfTyVar {
    match v {
        TlcTypeVar::Named(n) => DfTyVar::Named(n),
        TlcTypeVar::Inferred(n) => DfTyVar::Inferred(n),
    }
}

fn lower_lit(lit: &TlcLit) -> Option<DfLit> {
    match lit {
        TlcLit::Bool(b) => Some(DfLit::Bool(*b)),
        TlcLit::Int(n) => Some(DfLit::Int(*n)),
        TlcLit::Float(f) => Some(DfLit::Float(*f)),
        TlcLit::Posit(literal) => Some(DfLit::Posit(*literal)),
        TlcLit::Str(s) => Some(DfLit::Text(s.clone())),
        TlcLit::Atom(s) => Some(DfLit::Atom(s.clone())),
        TlcLit::Nothing => None,
    }
}

fn lower_builtin_op(op: BuiltinOp) -> DfBuiltinOp {
    match op {
        BuiltinOp::Add => DfBuiltinOp::Add,
        BuiltinOp::Sub => DfBuiltinOp::Sub,
        BuiltinOp::Mul => DfBuiltinOp::Mul,
        BuiltinOp::Div => DfBuiltinOp::Div,
        BuiltinOp::Eq => DfBuiltinOp::Eq,
        BuiltinOp::Ne => DfBuiltinOp::Ne,
        BuiltinOp::Lt => DfBuiltinOp::Lt,
        BuiltinOp::Le => DfBuiltinOp::Le,
        BuiltinOp::Gt => DfBuiltinOp::Gt,
        BuiltinOp::Ge => DfBuiltinOp::Ge,
        BuiltinOp::And => DfBuiltinOp::And,
        BuiltinOp::Or => DfBuiltinOp::Or,
        BuiltinOp::Coalesce => unreachable!("Coalesce handled separately"),
    }
}

/// Slot = rank of `field` among the record type's field names, sorted ascending.
fn record_slot(fields: &[DfRecordField], field: &str) -> usize {
    let mut names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    names.iter().position(|&name| name == field).unwrap_or(0)
}

fn lower_posit_op(op: BuiltinOp) -> Option<DfPositOp> {
    match op {
        BuiltinOp::Add => Some(DfPositOp::Add),
        BuiltinOp::Sub => Some(DfPositOp::Sub),
        BuiltinOp::Mul => Some(DfPositOp::Mul),
        BuiltinOp::Div => Some(DfPositOp::Div),
        BuiltinOp::Eq => Some(DfPositOp::Eq),
        BuiltinOp::Ne => Some(DfPositOp::Ne),
        BuiltinOp::Lt => Some(DfPositOp::Lt),
        BuiltinOp::Le => Some(DfPositOp::Le),
        BuiltinOp::Gt => Some(DfPositOp::Gt),
        BuiltinOp::Ge => Some(DfPositOp::Ge),
        BuiltinOp::And | BuiltinOp::Or | BuiltinOp::Coalesce => None,
    }
}

fn lower_import_source(source: &HirImportSource) -> (String, ImportKind) {
    match source {
        HirImportSource::String(path) => {
            let kind = if path.ends_with(".zti") {
                ImportKind::Zti
            } else {
                ImportKind::Zt
            };
            (path.clone(), kind)
        }
        HirImportSource::Path(parts) => (parts.join("."), ImportKind::Zt),
    }
}

// ── Lowerer ───────────────────────────────────────────────────────────────────

struct Lowerer<'m> {
    module: &'m TlcModule,
    hir_bindings: &'m [Binding],
    nodes: Arena<DfNode>,
    types: Arena<DfTy>,
    globals: IndexMap<String, NodeId>,
    spans: Vec<Option<Span>>,
    type_cache: FxHashMap<TlcTypeId, DfTyId>,
    /// Local binding table: maps BindingId → the DC NodeId for that binding.
    /// This is the sharing mechanism: each local is lowered once; all references
    /// become edges to the same NodeId (tree-to-graph transformation).
    local_env: FxHashMap<BindingId, NodeId>,
    /// Global bindings: BindingId → string name (for GlobalRef emission).
    global_names: FxHashMap<BindingId, String>,
    /// Global bindings: BindingId → TLC type (for GlobalRef node types).
    global_types: FxHashMap<BindingId, TlcTypeId>,
    /// Type alias binding → TLC body, used only to recover record field names for slots.
    type_aliases: FxHashMap<BindingId, TlcTypeId>,
    /// Pre-allocated error type ID.
    error_ty: DfTyId,
    /// Named alias bindings → their canonical DC type `DfTyId`.  All
    /// `TyVar(Named(binding))` occurrences return the same slot, regardless of
    /// the source `TlcTypeId` (multiple arena nodes may represent the same alias
    /// reference).  During body lowering the slot holds a placeholder `DfTy::Error`
    /// back-reference; after lowering the slot is overwritten with the real body
    /// content, making the DfTy arena equirecursively cyclic.  Never cleared — it
    /// is a permanent alias-binding → DfTyId cache that also serves as the
    /// in-progress guard.
    alias_binding_type: FxHashMap<BindingId, DfTyId>,
    /// Saturated `TyFun` applications instantiated into concrete DC shapes.
    /// Recursive generic aliases (e.g. `Tree Int`) back-reference the same
    /// application key from their own fields, so this cache is also the
    /// equirecursive placeholder guard.
    type_app_cache: FxHashMap<(DfTyId, Vec<DfTyId>), DfTyId>,
    type_app_depth: u32,
}

impl<'m> Lowerer<'m> {
    fn new(module: &'m TlcModule, hir_bindings: &'m [Binding]) -> Self {
        let mut types = Arena::new();
        let error_ty = types.alloc(DfTy::Error);
        Self {
            module,
            hir_bindings,
            nodes: Arena::new(),
            types,
            globals: IndexMap::new(),
            spans: Vec::new(),
            alias_binding_type: FxHashMap::default(),
            type_app_cache: FxHashMap::default(),
            type_app_depth: 0,
            type_cache: FxHashMap::default(),
            local_env: FxHashMap::default(),
            global_names: FxHashMap::default(),
            global_types: FxHashMap::default(),
            type_aliases: FxHashMap::default(),
            error_ty,
        }
    }

    fn alloc_node(&mut self, kind: DfNodeKind, ty: DfTyId, span: Option<Span>) -> NodeId {
        let id = self.nodes.alloc(DfNode { ty, kind });
        self.spans.push(span);
        debug_assert_eq!(
            self.spans.len(),
            self.nodes.len(),
            "spans table out of sync with nodes arena"
        );
        id
    }

    fn record_slot_for_df_ty(&self, ty: DfTyId, field: &str) -> Option<usize> {
        match &self.types[ty] {
            DfTy::Record(fields) => Some(record_slot(fields, field)),
            _ => None,
        }
    }

    fn record_field_ty_for_df_ty(&self, ty: DfTyId, field: &str) -> Option<DfTyId> {
        match &self.types[ty] {
            DfTy::Record(fields) => fields
                .iter()
                .find(|record_field| record_field.name == field)
                .map(|record_field| record_field.ty),
            _ => None,
        }
    }

    fn tuple_field_ty_for_df_ty(
        &self,
        ty: DfTyId,
        index: usize,
        name: Option<&str>,
    ) -> Option<DfTyId> {
        let DfTy::Tuple(fields) = &self.types[ty] else {
            return None;
        };
        if let Some(name) = name
            && let Some(ty) = fields.iter().find_map(|field| match field {
                DfTupleField::Named {
                    name: field_name,
                    ty,
                } if field_name == name => Some(*ty),
                _ => None,
            })
        {
            return Some(ty);
        }
        fields.get(index).map(|field| match field {
            DfTupleField::Named { ty, .. } | DfTupleField::Positional(ty) => *ty,
        })
    }

    fn variant_tag_index_for_df_ty(&self, ty: DfTyId, tag: &str) -> usize {
        match &self.types[ty] {
            DfTy::Union(members) => members
                .iter()
                .position(|member| member.tag == tag)
                .unwrap_or(0),
            DfTy::Optional(_) => usize::from(tag == "some"),
            DfTy::Maybe(_) => usize::from(tag == "present"),
            _ => 0,
        }
    }

    fn variant_payload_ty_for_df_ty(&self, ty: DfTyId, tag: &str) -> DfTyId {
        match &self.types[ty] {
            DfTy::Union(members) => members
                .iter()
                .find(|member| member.tag == tag)
                .map(|member| member.ty)
                .unwrap_or(ty),
            DfTy::Optional(inner) | DfTy::Maybe(inner) => *inner,
            _ => ty,
        }
    }

    fn record_slot_for_tlc_type(
        &mut self,
        ty: TlcTypeId,
        field: &str,
        seen_aliases: &mut FxHashSet<BindingId>,
    ) -> Option<usize> {
        match self.module.type_arena[ty].clone() {
            TlcType::Record(_) => {
                let df_ty = self.lower_type(ty);
                self.record_slot_for_df_ty(df_ty, field)
            }
            TlcType::TyVar(TlcTypeVar::Named(binding), _) => {
                let binding = BindingId(binding);
                let body = *self.type_aliases.get(&binding)?;
                if seen_aliases.insert(binding) {
                    self.record_slot_for_tlc_type(body, field, seen_aliases)
                } else {
                    None
                }
            }
            TlcType::TyLamK(_, _, body) | TlcType::TyApp(body, _) => {
                self.record_slot_for_tlc_type(body, field, seen_aliases)
            }
            _ => None,
        }
    }

    fn record_slot_for_expr_type(&mut self, expr: TlcExprId, field: &str) -> Option<usize> {
        let ty = self.module.expr_types.get(&expr).copied()?;
        self.record_slot_for_tlc_type(ty, field, &mut FxHashSet::default())
    }

    // ── First pass: collect global names and types ────────────────────────────

    fn collect_globals(&mut self) {
        for &decl_id in &self.module.decls {
            match &self.module.decl_arena[decl_id] {
                TlcDecl::Value { binding, ty, .. } => {
                    if let Some(b) = self.hir_bindings.get(binding.0 as usize) {
                        self.global_names.insert(*binding, b.name.clone());
                        self.global_types.insert(*binding, *ty);
                    }
                }
                TlcDecl::TypeAlias { binding, body, .. } => {
                    self.type_aliases.insert(*binding, *body);
                }
            }
        }
    }

    // ── Main lowering pass ────────────────────────────────────────────────────

    fn lower_file(&mut self) -> DataflowGraph {
        self.collect_globals();

        // Lower each value decl into globals.
        // Collect (binding, name, body) to avoid borrow conflicts.
        let decls: Vec<(BindingId, String, TlcExprId)> = self
            .module
            .decls
            .iter()
            .filter_map(|&decl_id| match &self.module.decl_arena[decl_id] {
                TlcDecl::Value { binding, body, .. } => {
                    let name = self.global_names.get(binding)?.clone();
                    Some((*binding, name, *body))
                }
                TlcDecl::TypeAlias { .. } => None,
            })
            .collect();

        for (_binding, name, body_id) in decls {
            let node_id = self.lower_expr(body_id);
            self.globals.insert(name, node_id);
        }

        let root = match self.module.final_expr {
            Some(final_id) => self.lower_expr(final_id),
            None => self.alloc_node(DfNodeKind::Error, self.error_ty, None),
        };

        let graph = DataflowGraph {
            nodes: std::mem::take(&mut self.nodes),
            types: std::mem::take(&mut self.types),
            globals: std::mem::take(&mut self.globals),
            root,
            spans: std::mem::take(&mut self.spans),
        };

        // Cheap structural integrity (ref bounds, type-shape compat, bind ownership,
        // stray globals, span/root) runs in every build: a graph with dangling refs or
        // shape mismatches would silently miscompile in ANF→SSA→codegen.
        if let Err(errs) = crate::validate::validate_structural(&graph) {
            panic!("internal compiler error: invalid DataflowGraph: {errs:?}");
        }
        // The O(node × scope) capture/scope walk (invariants 3 and 4) stays debug-only.
        #[cfg(debug_assertions)]
        if let Err(errs) = crate::validate::validate(&graph) {
            panic!("internal compiler error: DataflowGraph validation failed: {errs:?}");
        }

        graph
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn row_to_fields(row: &Row) -> Vec<(String, bool, TlcTypeId)> {
    let mut result = Vec::new();
    let mut r = row;
    while let Row::RExtend {
        label,
        ty,
        optional,
        tail,
    } = r
    {
        result.push((label.clone(), *optional, *ty));
        r = tail;
    }
    result
}

fn remove_pat_bindings(pat: &TlcPat, env: &mut FxHashMap<BindingId, NodeId>) {
    match pat {
        TlcPat::Bind(b) => {
            env.remove(b);
        }
        TlcPat::Tuple(items) => {
            for item in items {
                match item {
                    TlcPatItem::Named { pat, .. } => remove_pat_bindings(pat, env),
                    TlcPatItem::Positional(p) => remove_pat_bindings(p, env),
                }
            }
        }
        TlcPat::Record(fields) => {
            for (_, p) in fields {
                remove_pat_bindings(p, env);
            }
        }
        TlcPat::Variant(_, inner) => remove_pat_bindings(inner, env),
        _ => {}
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

pub(crate) fn lower_tlc(module: &TlcModule, hir_bindings: &[Binding]) -> DataflowGraph {
    let mut lowerer = Lowerer::new(module, hir_bindings);
    lowerer.lower_file()
}
