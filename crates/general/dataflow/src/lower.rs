use std::collections::HashMap;

use indexmap::IndexMap;
use la_arena::Arena;
use zutai_hir::{Binding, BindingId, HirImportSource};
use zutai_syntax::Span;
use zutai_tlc::{
    BuiltinOp, Literal as TlcLit, PrimTy, Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcModule,
    TlcPat, TlcPatItem, TlcTupleField, TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};

use crate::{
    DataflowGraph, DfArm, DfBuiltinOp, DfLit, DfNode, DfNodeKind, DfPattern, DfRecordField,
    DfTupleField, DfTupleNodeItem, DfTuplePatItem, DfTy, DfTyId, DfTyVar, ImportKind, NodeId,
};

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
    type_cache: HashMap<TlcTypeId, DfTyId>,
    /// Local binding table: maps BindingId → the DC NodeId for that binding.
    /// This is the sharing mechanism: each local is lowered once; all references
    /// become edges to the same NodeId (tree-to-graph transformation).
    local_env: HashMap<BindingId, NodeId>,
    /// Global bindings: BindingId → string name (for GlobalRef emission).
    global_names: HashMap<BindingId, String>,
    /// Global bindings: BindingId → TLC type (for GlobalRef node types).
    global_types: HashMap<BindingId, TlcTypeId>,
    /// Pre-allocated error type ID.
    error_ty: DfTyId,
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
            type_cache: HashMap::new(),
            local_env: HashMap::new(),
            global_names: HashMap::new(),
            global_types: HashMap::new(),
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
                TlcDecl::TypeAlias { .. } => {}
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

        #[cfg(debug_assertions)]
        if let Err(errs) = crate::validate::validate(&graph) {
            panic!("DataflowGraph validation failed: {errs:?}");
        }

        graph
    }

    // ── Expression lowering ───────────────────────────────────────────────────

    fn lower_expr(&mut self, id: TlcExprId) -> NodeId {
        let expr = self.module.expr_arena[id].clone();
        let span = self.module.spans.get(&id).copied();
        let df_ty = match self.module.expr_types.get(&id).copied() {
            Some(t) => self.lower_type(t),
            None => self.error_ty,
        };

        match expr {
            TlcExpr::Var(binding) => {
                // Local env first — this is where sharing happens.
                if let Some(&node_id) = self.local_env.get(&binding) {
                    return node_id;
                }
                // Global reference.
                if let Some(name) = self.global_names.get(&binding).cloned() {
                    return self.alloc_node(DfNodeKind::GlobalRef(name), df_ty, span);
                }
                self.alloc_node(DfNodeKind::Error, self.error_ty, span)
            }

            TlcExpr::Lit(lit) => match lower_lit(&lit) {
                Some(df_lit) => self.alloc_node(DfNodeKind::Lit(df_lit), df_ty, span),
                None => self.alloc_node(DfNodeKind::Error, self.error_ty, span),
            },

            TlcExpr::Lam(param_binding, param_ty, body) => {
                let param_df_ty = self.lower_type(param_ty);
                let bind_node = self.alloc_node(DfNodeKind::Bind, param_df_ty, None);
                self.local_env.insert(param_binding, bind_node);
                let body_node = self.lower_expr(body);
                self.local_env.remove(&param_binding);
                self.alloc_node(
                    DfNodeKind::Lambda {
                        param: bind_node,
                        body: body_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::App(func, arg) => {
                let func_node = self.lower_expr(func);
                let arg_node = self.lower_expr(arg);
                self.alloc_node(
                    DfNodeKind::Apply {
                        func: func_node,
                        arg: arg_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::TyLam(tyvar, _kind, body) => {
                let body_node = self.lower_expr(body);
                self.alloc_node(
                    DfNodeKind::TyLam {
                        ty_params: vec![lower_tyvar(tyvar)],
                        body: body_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::TyApp(expr, ty_arg) => {
                let poly_node = self.lower_expr(expr);
                let df_ty_arg = self.lower_type(ty_arg);
                self.alloc_node(
                    DfNodeKind::TyApp {
                        poly: poly_node,
                        ty_args: vec![df_ty_arg],
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Let {
                binding,
                ty: _,
                value,
                body,
            } => {
                // Tree-to-graph: lower value once, register its NodeId.
                // All references to `binding` in `body` will reuse this NodeId directly.
                let value_node = self.lower_expr(value);
                self.local_env.insert(binding, value_node);
                let body_node = self.lower_expr(body);
                self.local_env.remove(&binding);
                // The `let` itself disappears; graph edges carry the sharing.
                body_node
            }

            TlcExpr::Letrec { bindings, body } => {
                // TlcExpr::Letrec is defined in the IR but never generated by the v0 TLC
                // lowerer. Lower defensively without mutual visibility: each binding's value
                // is lowered before the next binding's name is in scope (not true letrec
                // semantics). This is acceptable because this code path is unreachable in
                // well-formed v0 programs; the ANF phase handles recursion via GlobalRef SCC
                // analysis on globals, not via local letrec.
                for (binding, _, value_id) in &bindings {
                    let value_node = self.lower_expr(*value_id);
                    self.local_env.insert(*binding, value_node);
                }
                let body_node = self.lower_expr(body);
                for (binding, _, _) in &bindings {
                    self.local_env.remove(binding);
                }
                body_node
            }

            TlcExpr::Case(scrutinee, alts) => {
                // Resolve the scrutinee's type so arm-bound variables get the right Bind type.
                let scrutinee_df_ty = self
                    .module
                    .expr_types
                    .get(&scrutinee)
                    .copied()
                    .map(|t| self.lower_type(t))
                    .unwrap_or(self.error_ty);
                let scrutinee_node = self.lower_expr(scrutinee);
                let arms: Vec<DfArm> = alts
                    .iter()
                    .map(|alt| self.lower_alt(alt, scrutinee_df_ty))
                    .collect();
                self.alloc_node(
                    DfNodeKind::Match {
                        scrutinee: scrutinee_node,
                        arms,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Record(fields) => {
                let df_fields: Vec<(String, NodeId)> = fields
                    .iter()
                    .map(|(name, expr_id)| (name.clone(), self.lower_expr(*expr_id)))
                    .collect();
                self.alloc_node(DfNodeKind::Record(df_fields), df_ty, span)
            }

            TlcExpr::RecordUpdate { receiver, fields } => {
                let base = self.lower_expr(receiver);
                let updates: Vec<(String, NodeId)> = fields
                    .iter()
                    .map(|(name, expr_id)| (name.clone(), self.lower_expr(*expr_id)))
                    .collect();
                self.alloc_node(DfNodeKind::RecordUpdate { base, updates }, df_ty, span)
            }

            TlcExpr::GetField(expr, field) => {
                let base_node = self.lower_expr(expr);
                self.alloc_node(
                    DfNodeKind::Select {
                        base: base_node,
                        field,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Tuple(items) => {
                let df_items: Vec<DfTupleNodeItem> = items
                    .iter()
                    .map(|item| match item {
                        TlcTupleItem::Named { name, value } => DfTupleNodeItem::Named {
                            name: name.clone(),
                            value: self.lower_expr(*value),
                        },
                        TlcTupleItem::Positional(v) => {
                            DfTupleNodeItem::Positional(self.lower_expr(*v))
                        }
                    })
                    .collect();
                self.alloc_node(DfNodeKind::Tuple(df_items), df_ty, span)
            }

            TlcExpr::List(items) => {
                let df_items: Vec<NodeId> = items.iter().map(|&e| self.lower_expr(e)).collect();
                self.alloc_node(DfNodeKind::List(df_items), df_ty, span)
            }

            TlcExpr::Builtin(op, lhs, rhs) => {
                let lhs_node = self.lower_expr(lhs);
                let rhs_node = self.lower_expr(rhs);
                if op == BuiltinOp::Coalesce {
                    self.alloc_node(
                        DfNodeKind::Coalesce {
                            value: lhs_node,
                            fallback: rhs_node,
                        },
                        df_ty,
                        span,
                    )
                } else {
                    let df_op = lower_builtin_op(op);
                    self.alloc_node(DfNodeKind::Builtin(df_op, lhs_node, rhs_node), df_ty, span)
                }
            }

            TlcExpr::Variant(tag, payload) => {
                let payload_node = self.lower_expr(payload);
                self.alloc_node(DfNodeKind::Variant(tag, payload_node), df_ty, span)
            }

            TlcExpr::Import(source) => {
                let (path, kind) = lower_import_source(&source);
                self.alloc_node(DfNodeKind::Import { path, kind }, df_ty, span)
            }

            TlcExpr::Sequence(items) => match items.last().copied() {
                Some(last) => self.lower_expr(last),
                None => self.alloc_node(DfNodeKind::Error, self.error_ty, span),
            },

            TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. } => {
                self.alloc_node(DfNodeKind::Error, self.error_ty, span)
            }
        }
    }

    // ── Match arm lowering ────────────────────────────────────────────────────

    fn lower_alt(&mut self, alt: &TlcAlt, scrutinee_ty: DfTyId) -> DfArm {
        // Pattern lowering inserts Bind nodes into local_env.
        // Passing the scrutinee type gives each Bind node an accurate DfTyId.
        let pattern = self.lower_pat(&alt.pat, scrutinee_ty);
        let guard = alt.guard.map(|g| self.lower_expr(g));
        let body = self.lower_expr(alt.body);
        // Remove pattern bindings from scope (arm body is done).
        remove_pat_bindings(&alt.pat, &mut self.local_env);
        DfArm {
            pattern,
            guard,
            body,
        }
    }

    fn lower_pat(&mut self, pat: &TlcPat, context_ty: DfTyId) -> DfPattern {
        match pat {
            TlcPat::Wildcard => DfPattern::Wildcard,
            TlcPat::Bind(binding) => {
                let bind_node = self.alloc_node(DfNodeKind::Bind, context_ty, None);
                self.local_env.insert(*binding, bind_node);
                DfPattern::Bind(bind_node)
            }
            TlcPat::Lit(lit) => match lower_lit(lit) {
                Some(df_lit) => DfPattern::Lit(df_lit),
                None => DfPattern::Wildcard,
            },
            TlcPat::Atom(s) => DfPattern::Atom(s.clone()),
            TlcPat::Tuple(items) => {
                let df_items = items
                    .iter()
                    .map(|item| match item {
                        TlcPatItem::Named { name, pat } => DfTuplePatItem::Named {
                            name: name.clone(),
                            pattern: self.lower_pat(pat, context_ty),
                        },
                        TlcPatItem::Positional(p) => {
                            DfTuplePatItem::Positional(self.lower_pat(p, context_ty))
                        }
                    })
                    .collect();
                DfPattern::Tuple(df_items)
            }
            TlcPat::Record(fields) => {
                let df_fields = fields
                    .iter()
                    .map(|(name, p)| (name.clone(), self.lower_pat(p, context_ty)))
                    .collect();
                DfPattern::Record(df_fields)
            }
            TlcPat::Variant(tag, inner) => {
                DfPattern::Variant(tag.clone(), Box::new(self.lower_pat(inner, context_ty)))
            }
        }
    }

    // ── Type lowering ─────────────────────────────────────────────────────────

    fn lower_type(&mut self, id: TlcTypeId) -> DfTyId {
        if let Some(&cached) = self.type_cache.get(&id) {
            return cached;
        }
        // Clone to release the borrow on self.module before calling lower_type recursively.
        let ty = self.module.type_arena[id].clone();
        let result = self.lower_type_owned(ty);
        self.type_cache.insert(id, result);
        result
    }

    fn lower_type_owned(&mut self, ty: TlcType) -> DfTyId {
        match ty {
            TlcType::Prim(PrimTy::Int) => self.types.alloc(DfTy::Int),
            TlcType::Prim(PrimTy::Float) => self.types.alloc(DfTy::Float),
            TlcType::Prim(PrimTy::FixedNum(fw)) => {
                let ty = if fw.is_float() {
                    DfTy::Float
                } else {
                    DfTy::Int
                };
                self.types.alloc(ty)
            }
            TlcType::Prim(PrimTy::Bool) => self.types.alloc(DfTy::Bool),
            TlcType::Prim(PrimTy::Str) => self.types.alloc(DfTy::Text),
            TlcType::Prim(PrimTy::Atom) => self.types.alloc(DfTy::Atom),
            TlcType::Prim(PrimTy::Nothing) => self.types.alloc(DfTy::Error),

            TlcType::Singleton(TlcLit::Bool(true)) => self.types.alloc(DfTy::True),
            TlcType::Singleton(TlcLit::Bool(false)) => self.types.alloc(DfTy::False),
            // Atom singletons (used for union-arm discrimination) lower to the generic
            // Atom primitive — DC's type system has no singleton-Atom variant.
            TlcType::Singleton(TlcLit::Atom(_)) => self.types.alloc(DfTy::Atom),
            // Other singletons (Int, Float, Text, Nothing) have no DC type representation.
            TlcType::Singleton(_) => self.types.alloc(DfTy::Error),

            TlcType::Fun(a, b, _eff) => {
                let da = self.lower_type(a);
                let db = self.lower_type(b);
                self.types.alloc(DfTy::Fun(da, db))
            }

            TlcType::ForAll(v, _, body) => {
                let dv = lower_tyvar(v);
                let dbody = self.lower_type(body);
                self.types.alloc(DfTy::TyFun(vec![dv], dbody))
            }

            TlcType::TyVar(v, _) => self.types.alloc(DfTy::TyVar(lower_tyvar(v))),

            TlcType::TyApp(f, arg) => {
                let df = self.lower_type(f);
                let darg = self.lower_type(arg);
                self.types.alloc(DfTy::TyApp(df, vec![darg]))
            }

            TlcType::TyLamK(v, _, body) => {
                let dv = lower_tyvar(v);
                let dbody = self.lower_type(body);
                self.types.alloc(DfTy::TyFun(vec![dv], dbody))
            }

            TlcType::Record(row) => {
                // Collect field data (copy TlcTypeIds out) before calling lower_type.
                let field_data: Vec<(String, bool, TlcTypeId)> = row_to_fields(&row);
                let df_fields: Vec<DfRecordField> = field_data
                    .into_iter()
                    .map(|(name, optional, ty_id)| DfRecordField {
                        name,
                        optional,
                        ty: self.lower_type(ty_id),
                    })
                    .collect();
                self.types.alloc(DfTy::Record(df_fields))
            }

            TlcType::VariantT(row) => {
                let ty_ids: Vec<TlcTypeId> = row.fields().map(|(_, t)| t).collect();
                let df_variants: Vec<DfTyId> =
                    ty_ids.into_iter().map(|t| self.lower_type(t)).collect();
                self.types.alloc(DfTy::Union(df_variants))
            }

            TlcType::Tuple(fields) => {
                let df_fields: Vec<DfTupleField> = fields
                    .into_iter()
                    .map(|f| match f {
                        TlcTupleField::Named { name, ty } => DfTupleField::Named {
                            name,
                            ty: self.lower_type(ty),
                        },
                        TlcTupleField::Positional(ty) => {
                            DfTupleField::Positional(self.lower_type(ty))
                        }
                    })
                    .collect();
                self.types.alloc(DfTy::Tuple(df_fields))
            }

            TlcType::List(t) => {
                let dt = self.lower_type(t);
                self.types.alloc(DfTy::List(dt))
            }

            TlcType::Optional(t) => {
                let dt = self.lower_type(t);
                self.types.alloc(DfTy::Optional(dt))
            }

            TlcType::Maybe(t) => {
                let dt = self.lower_type(t);
                self.types.alloc(DfTy::Maybe(dt))
            }
        }
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

fn remove_pat_bindings(pat: &TlcPat, env: &mut HashMap<BindingId, NodeId>) {
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
