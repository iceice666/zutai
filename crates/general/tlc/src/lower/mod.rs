use std::collections::{HashMap, HashSet};

use la_arena::Arena;
use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{RowTail, ThirFile, TypeId, TypeKind, TypeTupleItem};

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
    Structural(String),
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
    /// Next fresh row-variable id for anonymous open rows (`...`). Allocated from
    /// the top of the id space and mapped to `TlcTypeVar::Inferred`, so it never
    /// collides with a THIR `InferVar` id (small, counted from zero).
    next_row_var: u32,
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
            next_row_var: u32::MAX,
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
                    if let Some(cst_binding) = constraint {
                        if let Some(key) = self.thir_type_to_witness_key(*target) {
                            self.witness_bindings
                                .insert((cst_binding.0, key), decl.binding);
                        }
                        if let Some(key) = self.thir_type_to_resolved_witness_key(*target) {
                            self.witness_bindings
                                .insert((cst_binding.0, key), decl.binding);
                        }
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

    /// Mint a fresh row variable for an anonymous open row tail (`...`).
    fn fresh_row_var(&mut self) -> TlcTypeVar {
        let id = self.next_row_var;
        self.next_row_var -= 1;
        TlcTypeVar::Inferred(id)
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
            TypeKind::Record(_, _)
            | TypeKind::Tuple(_)
            | TypeKind::Union(_, _)
            | TypeKind::List(_)
            | TypeKind::Optional(_)
            | TypeKind::Function { .. } => self.thir_type_to_resolved_witness_key(ty),
            _ => None,
        }
    }

    fn thir_type_to_resolved_witness_key(&self, ty: TypeId) -> Option<WitnessTargetKey> {
        let key = self.structural_witness_key(ty, &mut HashSet::new())?;
        match key.as_str() {
            "Int" => Some(WitnessTargetKey::Int),
            "Float" => Some(WitnessTargetKey::Float),
            "Bool" => Some(WitnessTargetKey::Bool),
            "Text" => Some(WitnessTargetKey::Str),
            "Atom" => Some(WitnessTargetKey::Atom),
            _ => Some(WitnessTargetKey::Structural(key)),
        }
    }

    fn structural_witness_key(&self, ty: TypeId, seen: &mut HashSet<BindingId>) -> Option<String> {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Int => Some("Int".to_string()),
            TypeKind::Float => Some("Float".to_string()),
            TypeKind::Bool | TypeKind::True | TypeKind::False => Some("Bool".to_string()),
            TypeKind::Text => Some("Text".to_string()),
            TypeKind::Atom(name) => Some(format!("#{name}")),
            TypeKind::List(inner) => {
                Some(format!("[{}]", self.structural_witness_key(inner, seen)?))
            }
            TypeKind::Optional(inner) => {
                Some(format!("{}?", self.structural_witness_key(inner, seen)?))
            }
            TypeKind::Record(fields, tail) => {
                let mut parts: Vec<String> = fields
                    .into_iter()
                    .map(|field| {
                        let key = self.structural_witness_key(field.ty, seen)?;
                        let marker = if field.optional { "?:" } else { ":" };
                        Some(format!("{}{}{}", field.name, marker, key))
                    })
                    .collect::<Option<_>>()?;
                parts.sort();
                Some(format!("{{{}{}}}", parts.join(","), row_tail_key(tail)))
            }
            TypeKind::Union(variants, tail) => {
                let parts: Vec<String> = variants
                    .into_iter()
                    .map(|variant| match variant.payload {
                        Some(payload) => Some(format!(
                            "{}({})",
                            variant.name,
                            self.structural_witness_key(payload, seen)?
                        )),
                        None => Some(variant.name),
                    })
                    .collect::<Option<_>>()?;
                Some(format!("<{}{}>", parts.join("|"), row_tail_key(tail)))
            }
            TypeKind::Tuple(items) => {
                let parts: Vec<String> = items
                    .into_iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => Some(format!(
                            "{}:{}",
                            name,
                            self.structural_witness_key(ty, seen)?
                        )),
                        TypeTupleItem::Positional(ty) => self.structural_witness_key(ty, seen),
                    })
                    .collect::<Option<_>>()?;
                Some(format!("({})", parts.join(",")))
            }
            TypeKind::Function { from, to } => Some(format!(
                "({}->{})",
                self.structural_witness_key(from, seen)?,
                self.structural_witness_key(to, seen)?
            )),
            TypeKind::Alias(binding) => {
                if !seen.insert(binding) {
                    return None;
                }
                let body = self.type_alias_body(binding)?;
                self.structural_witness_key(body, seen)
            }
            TypeKind::AliasApply { binding, args } => {
                if !seen.insert(binding) {
                    return None;
                }
                let (params, body) = self.type_alias_params_body(binding)?;
                self.structural_witness_key_subst(
                    body,
                    &params.into_iter().zip(args).collect(),
                    seen,
                )
            }
            TypeKind::TypeVar(binding) => Some(format!("@{}", binding.0)),
            TypeKind::InferVar(v) => Some(format!("?{v}")),
            TypeKind::Type | TypeKind::Error => None,
        }
    }

    fn structural_witness_key_subst(
        &self,
        ty: TypeId,
        subst: &HashMap<BindingId, TypeId>,
        seen: &mut HashSet<BindingId>,
    ) -> Option<String> {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(binding) => subst
                .get(&binding)
                .copied()
                .map(|replacement| self.structural_witness_key(replacement, seen))
                .unwrap_or_else(|| Some(format!("@{}", binding.0))),
            _ => self.structural_witness_key(ty, seen),
        }
    }

    fn type_alias_body(&self, binding: BindingId) -> Option<TypeId> {
        self.type_alias_params_body(binding).map(|(_, body)| body)
    }

    fn type_alias_params_body(&self, binding: BindingId) -> Option<(Vec<BindingId>, TypeId)> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == binding
                && let zutai_thir::ThirDeclKind::TypeAlias { params, ty } = &decl.kind
            {
                return Some((params.clone(), *ty));
            }
            None
        })
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

fn row_tail_key(tail: RowTail) -> String {
    match tail {
        RowTail::Closed => String::new(),
        RowTail::Open => "...".to_string(),
        RowTail::Param(b) => format!("...#{}", b.0),
        RowTail::Infer(v) => format!("...?{v}"),
    }
}
