use std::collections::HashMap;

use la_arena::Arena;
use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{ThirFile, TypeId, TypeKind};

use crate::ir::{
    TlcDecl, TlcDeclId, TlcExpr, TlcExprId, TlcModule, TlcType, TlcTypeId, TlcTypeVar,
};

mod decl;
mod expr;
mod types;

pub fn lower_thir(file: &ThirFile) -> TlcModule {
    let mut lowerer = Lowerer::new(file);
    lowerer.lower_file()
}

/// Normalized target-type key for constraint witness lookup.
///
/// After THIR zonking every witness target is one of these; the key is used to
/// match a call-site concrete type against the registered witness dictionaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum WitnessTargetKey {
    Int,
    Float,
    Bool,
    Str,
    Atom,
    /// Named type alias or generic alias application, keyed by BindingId.0.
    Named(u32),
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
    /// constraint method BindingId → (constraint BindingId, method name).
    /// Used in the Apply arm to dispatch to `GetField` on the active dict param.
    constraint_methods: HashMap<BindingId, (BindingId, String)>,
    /// (constraint BindingId.0, WitnessTargetKey) → witness decl BindingId.
    /// Populated for every `Witness` THIR decl; queried at concrete call sites.
    witness_bindings: HashMap<(u32, WitnessTargetKey), BindingId>,
    /// function BindingId → vec of (type-param BindingId, constraint BindingIds),
    /// sorted ascending by type-param BindingId.0 to match THIR `collect_type_vars`.
    fn_explicit_params: HashMap<BindingId, Vec<(BindingId, Vec<BindingId>)>>,
    /// (constraint BindingId.0, type-param BindingId.0) → active dict Lam BindingId.
    /// Set when entering a bounded function body; cleared on exit.
    active_dict_params: HashMap<(u32, u32), BindingId>,
    /// dict Lam BindingId → its TLC type (Record placeholder).
    active_dict_types: HashMap<BindingId, TlcTypeId>,
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
            constraint_methods: HashMap::new(),
            witness_bindings: HashMap::new(),
            fn_explicit_params: HashMap::new(),
            active_dict_params: HashMap::new(),
            active_dict_types: HashMap::new(),
        }
    }

    fn lower_file(&mut self) -> TlcModule {
        self.collect_decl_types();
        // Skip Constraint decls — they are only registered in collect_decl_types.
        // Witness decls are lowered to TLC Value decls (dict record values).
        let decls: Vec<TlcDeclId> = self
            .thir
            .decls
            .iter()
            .copied()
            .filter(|&id| {
                !matches!(
                    self.thir.decl_arena[id].kind,
                    zutai_thir::ThirDeclKind::Constraint { .. }
                )
            })
            .map(|id| self.lower_decl(id))
            .collect();
        let final_expr = Some(self.lower_expr(self.thir.final_expr));
        TlcModule {
            decls,
            final_expr,
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
            match &decl.kind {
                zutai_thir::ThirDeclKind::Value { ty, .. } => {
                    self.decl_thir_types.insert(decl.binding, *ty);
                }
                zutai_thir::ThirDeclKind::Function {
                    sig,
                    params,
                    param_bounds,
                    ..
                } => {
                    self.decl_thir_types.insert(decl.binding, *sig);
                    // Register explicit type params if any param has constraints.
                    let has_bounds = param_bounds.iter().any(|b| !b.is_empty());
                    if has_bounds || !params.is_empty() {
                        // Build sorted (type_param, constraints) vec.
                        let mut entries: Vec<(BindingId, Vec<BindingId>)> = params
                            .iter()
                            .zip(param_bounds.iter())
                            .map(|(&p, bs)| (p, bs.clone()))
                            .collect();
                        entries.sort_by_key(|(p, _)| p.0);
                        self.fn_explicit_params.insert(decl.binding, entries);
                    }
                }
                zutai_thir::ThirDeclKind::Constraint { methods, .. } => {
                    // Register every method binding so the Apply arm can dispatch.
                    for method in methods {
                        if let Some(binding) = method.binding {
                            self.constraint_methods
                                .insert(binding, (decl.binding, method.name.clone()));
                        }
                    }
                }
                zutai_thir::ThirDeclKind::Witness {
                    constraint, target, ..
                } => {
                    // Register witness decl for lookup at concrete call sites.
                    if let Some(cst_binding) = constraint
                        && let Some(key) = self.thir_type_to_witness_key(*target)
                    {
                        self.witness_bindings
                            .insert((cst_binding.0, key), decl.binding);
                    }
                    // Witness values are not in poly_schemes; register their THIR type
                    // as a Record of field types so `get_dict_expr` can find the TLC type.
                    // The actual ty will be computed during lower_decl for Witness.
                }
                zutai_thir::ThirDeclKind::TypeAlias { .. } => {}
            }
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

    /// Map a THIR TypeId to a `WitnessTargetKey` for witness lookup.
    /// Returns `None` for types that cannot serve as witness targets (TypeVar, InferVar, etc.).
    pub(crate) fn thir_type_to_witness_key(&self, ty: TypeId) -> Option<WitnessTargetKey> {
        match &self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::Int => Some(WitnessTargetKey::Int),
            TypeKind::Float => Some(WitnessTargetKey::Float),
            TypeKind::Bool | TypeKind::True | TypeKind::False => Some(WitnessTargetKey::Bool),
            TypeKind::Text => Some(WitnessTargetKey::Str),
            TypeKind::Atom(_) => Some(WitnessTargetKey::Atom),
            TypeKind::Alias(b) => Some(WitnessTargetKey::Named(b.0)),
            TypeKind::AliasApply { binding, .. } => Some(WitnessTargetKey::Named(binding.0)),
            _ => None,
        }
    }

    /// Build a TLC expression for passing the witness dict at a call site.
    ///
    /// If `inst_type_id` is an abstract `TypeVar`, threads the active dict param.
    /// If it is a concrete type, looks up the registered witness decl.
    /// Returns a `Lit(Nothing)` placeholder on failure (undefined witness).
    pub(crate) fn get_dict_expr(
        &mut self,
        cst_binding: BindingId,
        inst_type_id: TypeId,
        span: Span,
    ) -> TlcExprId {
        use crate::ir::{Literal, PrimTy, Row};
        let thir_kind = self.thir.type_arena[inst_type_id.0 as usize].kind.clone();

        match thir_kind {
            TypeKind::TypeVar(tv_binding) => {
                // Abstract type — thread the active dict param.
                if let Some(&dp) = self.active_dict_params.get(&(cst_binding.0, tv_binding.0)) {
                    let dp_ty = self
                        .active_dict_types
                        .get(&dp)
                        .copied()
                        .unwrap_or_else(|| self.alloc_type(TlcType::Record(Row::REmpty)));
                    self.alloc_expr(TlcExpr::Var(dp), dp_ty, span)
                } else {
                    let ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                    self.alloc_expr(TlcExpr::Lit(Literal::Nothing), ty, span)
                }
            }
            _ => {
                // Concrete type — look up the registered witness.
                if let Some(key) = self.thir_type_to_witness_key(inst_type_id) {
                    if let Some(&wb) = self.witness_bindings.get(&(cst_binding.0, key)) {
                        let wb_ty = self
                            .decl_thir_types
                            .get(&wb)
                            .copied()
                            .map(|thir_ty| self.lower_type(thir_ty))
                            .unwrap_or_else(|| self.alloc_type(TlcType::Record(Row::REmpty)));
                        self.alloc_expr(TlcExpr::Var(wb), wb_ty, span)
                    } else {
                        let ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                        self.alloc_expr(TlcExpr::Lit(Literal::Nothing), ty, span)
                    }
                } else {
                    let ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                    self.alloc_expr(TlcExpr::Lit(Literal::Nothing), ty, span)
                }
            }
        }
    }
}
