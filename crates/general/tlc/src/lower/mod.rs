use std::collections::{HashMap, HashSet};

use la_arena::Arena;
use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{RowTail, ThirDeclKind, ThirFile, TypeId, TypeKind, TypeTupleItem};

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

/// A parametric (conditional) witness such as `Eq @(List A) :: <A: Eq>`.
///
/// Its TLC value is a function `forall params. (component dicts) -> Record`,
/// so resolving it at a concrete call site means structurally matching `target`
/// against the call-site type, then applying the witness to the recursively
/// resolved component dictionaries.
#[derive(Clone)]
pub(crate) struct ConditionalWitness {
    /// Witness decl binding (the TLC value to apply).
    binding: BindingId,
    /// Constraint BindingId.0 this witness satisfies.
    constraint: u32,
    /// Witness target type, containing the witness params as `TypeVar` holes.
    target: TypeId,
    /// Witness type params, in declaration order; each gets a `TyApp`.
    params: Vec<BindingId>,
    /// Per-param constraint bounds, parallel to `params`; each bound gets an
    /// `App` of the recursively resolved component dict.
    param_bounds: Vec<Vec<BindingId>>,
}
/// Per-constraint-method dispatch info, keyed by the method's `BindingId`.
/// Lets the Apply arm split a call site's `instantiation` vector into the
/// constraint-param entry (selects the dict) and the method-level params
/// (each becomes a `TyApp` on the fetched method).
#[derive(Clone)]
pub(crate) struct ConstraintMethodInfo {
    /// The constraint's own `BindingId` (for dict lookup).
    constraint: BindingId,
    /// Method name (the dict field to `GetField`).
    name: String,
    /// The constraint's type parameter (`@F`); its instantiation selects the dict.
    constraint_param: BindingId,
    /// The method's own type parameters (`<A, B>`); each becomes a `TyApp`.
    method_params: Vec<BindingId>,
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
    constraint_methods: HashMap<BindingId, ConstraintMethodInfo>,
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
    /// Parametric witnesses, matched structurally at concrete call sites.
    conditional_witnesses: Vec<ConditionalWitness>,
    /// Recursion guard for conditional-witness resolution: `(constraint.0,
    /// concrete TypeId.0)` pairs currently being resolved. Re-entry signals a
    /// non-terminating witness search; resolution bails to avoid a stack overflow.
    resolving_dicts: HashSet<(u32, u32)>,
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
            conditional_witnesses: Vec::new(),
            resolving_dicts: HashSet::new(),
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
                zutai_thir::ThirDeclKind::Constraint {
                    params, methods, ..
                } => {
                    // Register every method binding so the Apply arm can dispatch.
                    // The constraint's first param is the `@F` target param.
                    let Some(&constraint_param) = params.first() else {
                        continue;
                    };
                    for method in methods {
                        if let Some(binding) = method.binding {
                            self.constraint_methods.insert(
                                binding,
                                ConstraintMethodInfo {
                                    constraint: decl.binding,
                                    name: method.name.clone(),
                                    constraint_param,
                                    method_params: method.params.clone(),
                                },
                            );
                        }
                    }
                }
                zutai_thir::ThirDeclKind::Witness {
                    constraint,
                    target,
                    params,
                    param_bounds,
                    ..
                } => {
                    if let Some(cst_binding) = constraint {
                        if params.is_empty() {
                            // Concrete witness: register under its structural key(s)
                            // for direct lookup at matching call sites.
                            if let Some(key) = self.thir_type_to_witness_key(*target) {
                                self.witness_bindings
                                    .insert((cst_binding.0, key), decl.binding);
                            }
                            if let Some(key) = self.thir_type_to_resolved_witness_key(*target) {
                                self.witness_bindings
                                    .insert((cst_binding.0, key), decl.binding);
                            }
                        } else {
                            // Conditional witness: its target carries the params as
                            // `TypeVar` holes, so it can never match a concrete key
                            // directly. Register it for structural matching instead.
                            self.conditional_witnesses.push(ConditionalWitness {
                                binding: decl.binding,
                                constraint: cst_binding.0,
                                target: *target,
                                params: params.clone(),
                                param_bounds: param_bounds.clone(),
                            });
                        }
                    }
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
            TypeKind::Con(b) => Some(WitnessTargetKey::Named(b.0)),
            TypeKind::Apply { .. } => {
                let (head, _) = self.thir_app_spine(ty);
                match &self.thir.type_arena[head.0 as usize].kind {
                    TypeKind::Alias(b) | TypeKind::Con(b) => Some(WitnessTargetKey::Named(b.0)),
                    _ => None,
                }
            }
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

    /// Flatten a curried THIR `Apply` chain into head + left-to-right args.
    fn thir_app_spine(&self, ty: TypeId) -> (TypeId, Vec<TypeId>) {
        let mut args: Vec<TypeId> = Vec::new();
        let mut cur = ty;
        while let TypeKind::Apply { func, arg } = self.thir.type_arena[cur.0 as usize].kind {
            args.push(arg);
            cur = func;
        }
        args.reverse();
        (cur, args)
    }
    /// The THIR signature of constraint method `name`, by scanning the constraint
    /// decl. Used at a call site to recover the method's exact type-var order.
    fn method_sig_for(&self, constraint: BindingId, name: &str) -> Option<TypeId> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint { methods, .. } = &decl.kind
            {
                return methods.iter().find(|m| m.name == name).map(|m| m.sig);
            }
            None
        })
    }

    /// Collect the `TypeVar` bindings free in a THIR type, deduped and sorted by
    /// binding id — exactly reproducing THIR's `collect_type_vars` order, so the
    /// result is positionally aligned with a call site's `instantiation` vector.
    fn collect_thir_type_vars(&self, ty: TypeId) -> Vec<BindingId> {
        let mut out: Vec<BindingId> = Vec::new();
        self.collect_thir_type_vars_into(ty, &mut out);
        out.sort_by_key(|b| b.0);
        out.dedup();
        out
    }

    fn collect_thir_type_vars_into(&self, ty: TypeId, out: &mut Vec<BindingId>) {
        match &self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(b) => out.push(*b),
            TypeKind::Function { from, to } => {
                self.collect_thir_type_vars_into(*from, out);
                self.collect_thir_type_vars_into(*to, out);
            }
            TypeKind::List(e) | TypeKind::Optional(e) => self.collect_thir_type_vars_into(*e, out),
            TypeKind::Apply { func, arg } => {
                self.collect_thir_type_vars_into(*func, out);
                self.collect_thir_type_vars_into(*arg, out);
            }
            TypeKind::AliasApply { args, .. } => {
                for &a in args {
                    self.collect_thir_type_vars_into(a, out);
                }
            }
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.collect_thir_type_vars_into(f.ty, out);
                }
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(p) = v.payload {
                        self.collect_thir_type_vars_into(p, out);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let t = match item {
                        TypeTupleItem::Named { ty, .. } => *ty,
                        TypeTupleItem::Positional(ty) => *ty,
                    };
                    self.collect_thir_type_vars_into(t, out);
                }
            }
            _ => {}
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
            TypeKind::Con(binding) => Some(format!("@{}", binding.0)),
            TypeKind::Apply { .. } => {
                let (head, args) = self.thir_app_spine(ty);
                // Saturated named-alias application keys like the AliasApply arm.
                if let TypeKind::Alias(binding) = self.thir.type_arena[head.0 as usize].kind {
                    if !seen.insert(binding) {
                        return None;
                    }
                    if let Some((params, body)) = self.type_alias_params_body(binding)
                        && params.len() == args.len()
                    {
                        return self.structural_witness_key_subst(
                            body,
                            &params.into_iter().zip(args).collect(),
                            seen,
                        );
                    }
                }
                let head_key = self.structural_witness_key(head, seen)?;
                let arg_keys: Vec<String> = args
                    .iter()
                    .map(|&a| self.structural_witness_key(a, seen))
                    .collect::<Option<_>>()?;
                Some(format!("{}[{}]", head_key, arg_keys.join(",")))
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
                // Concrete type — first try a directly registered witness.
                if let Some(key) = self.thir_type_to_witness_key(inst_type_id)
                    && let Some(&wb) = self.witness_bindings.get(&(cst_binding.0, key))
                {
                    let wb_ty = self
                        .decl_thir_types
                        .get(&wb)
                        .copied()
                        .map(|thir_ty| self.lower_type(thir_ty))
                        .unwrap_or_else(|| self.alloc_type(TlcType::Record(Row::REmpty)));
                    return self.alloc_expr(TlcExpr::Var(wb), wb_ty, span);
                }
                // Otherwise try to build the dict from a conditional witness whose
                // target structurally matches this concrete type.
                if let Some(dict) =
                    self.resolve_conditional_witness(cst_binding, inst_type_id, span)
                {
                    return dict;
                }
                let ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                self.alloc_expr(TlcExpr::Lit(Literal::Nothing), ty, span)
            }
        }
    }

    /// Build a dict expression for a concrete type from a conditional witness.
    ///
    /// Finds a registered conditional witness whose target structurally matches
    /// `concrete` (treating the witness params as holes), then emits
    /// `App(…App(TyApp(Var(witness), arg₀), dict₀₀), …)`: one `TyApp` per witness
    /// param and one `App` per param bound, where each bound's dict is resolved
    /// recursively at the matched argument type. Returns `None` when no witness
    /// matches or the search recurses (guarded against non-termination).
    fn resolve_conditional_witness(
        &mut self,
        cst_binding: BindingId,
        concrete: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        use crate::ir::{Row, TlcType};
        let guard = (cst_binding.0, concrete.0);
        if !self.resolving_dicts.insert(guard) {
            // Re-entry on the same (constraint, type): the witness search does not
            // make progress. Bail rather than recurse forever.
            return None;
        }
        let candidates: Vec<ConditionalWitness> = self
            .conditional_witnesses
            .iter()
            .filter(|cw| cw.constraint == cst_binding.0)
            .cloned()
            .collect();
        let mut result = None;
        for cw in candidates {
            let mut subst: HashMap<BindingId, TypeId> = HashMap::new();
            let holes: HashSet<BindingId> = cw.params.iter().copied().collect();
            if !self.unify_witness_target(cw.target, concrete, &holes, &mut subst) {
                continue;
            }
            // Each param must be pinned by the match; otherwise the witness is
            // not applicable to this concrete type.
            if cw.params.iter().any(|p| !subst.contains_key(p)) {
                continue;
            }
            let placeholder = self.alloc_type(TlcType::Record(Row::REmpty));
            let mut cur = self.alloc_expr(TlcExpr::Var(cw.binding), placeholder, span);
            let mut ok = true;
            for (param, bounds) in cw.params.iter().zip(cw.param_bounds.iter()) {
                let arg_ty_id = subst[param];
                let arg_ty = self.lower_type(arg_ty_id);
                cur = self.alloc_expr(TlcExpr::TyApp(cur, arg_ty), placeholder, span);
                for &bound in bounds {
                    let dict = self.get_dict_expr(bound, arg_ty_id, span);
                    if self.is_nothing_dict(dict) {
                        // A required component witness is missing; this candidate
                        // cannot produce a usable dict.
                        ok = false;
                        break;
                    }
                    cur = self.alloc_expr(TlcExpr::App(cur, dict), placeholder, span);
                }
                if !ok {
                    break;
                }
            }
            if ok {
                result = Some(cur);
                break;
            }
        }
        self.resolving_dicts.remove(&guard);
        result
    }

    /// True if `expr` is the `Lit(Nothing)` placeholder `get_dict_expr` returns
    /// when no witness resolves.
    fn is_nothing_dict(&self, expr: TlcExprId) -> bool {
        matches!(
            self.expr_arena[expr],
            TlcExpr::Lit(crate::ir::Literal::Nothing)
        )
    }

    /// Structurally match a witness `target` (with `holes` as wildcards) against
    /// a `concrete` type, recording each hole's binding in `subst`. Aliases on
    /// either side are expanded (with their type args substituted) so a witness
    /// target written as `Pair A` matches a concrete `{fst:Int,snd:Int}` that
    /// THIR already expanded. Returns `false` on a shape mismatch or an
    /// inconsistent re-binding of a hole.
    fn unify_witness_target(
        &self,
        target: TypeId,
        concrete: TypeId,
        holes: &HashSet<BindingId>,
        subst: &mut HashMap<BindingId, TypeId>,
    ) -> bool {
        self.unify_env(
            target,
            &HashMap::new(),
            concrete,
            &HashMap::new(),
            holes,
            subst,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn unify_env(
        &self,
        target: TypeId,
        tenv: &HashMap<BindingId, TypeId>,
        concrete: TypeId,
        cenv: &HashMap<BindingId, TypeId>,
        holes: &HashSet<BindingId>,
        subst: &mut HashMap<BindingId, TypeId>,
        depth: u32,
    ) -> bool {
        if depth > 64 {
            return false;
        }
        let no_holes = HashSet::new();
        let (target, tenv) = self.norm_ty(target, tenv, holes);
        let t_kind = self.thir.type_arena[target.0 as usize].kind.clone();
        // A hole matches any concrete type, but must bind consistently. Follow the
        // concrete's env-var chain (but do NOT alias-expand) so the binding stays
        // a self-contained type that `get_dict_expr` can re-resolve — expanding
        // here would strip an `AliasApply`'s args into dangling body variables.
        if let TypeKind::TypeVar(b) = t_kind
            && holes.contains(&b)
        {
            let resolved = self.resolve_env_var(concrete, cenv);
            return match subst.get(&b) {
                Some(&prev) => self.thir_types_equal(prev, resolved),
                None => {
                    subst.insert(b, resolved);
                    true
                }
            };
        }
        let (concrete, cenv) = self.norm_ty(concrete, cenv, &no_holes);
        let c_kind = self.thir.type_arena[concrete.0 as usize].kind.clone();
        match (t_kind, c_kind) {
            (TypeKind::List(ti), TypeKind::List(ci)) => {
                self.unify_env(ti, &tenv, ci, &cenv, holes, subst, depth + 1)
            }
            (TypeKind::Optional(ti), TypeKind::Optional(ci)) => {
                self.unify_env(ti, &tenv, ci, &cenv, holes, subst, depth + 1)
            }
            (TypeKind::Tuple(ti), TypeKind::Tuple(ci)) => {
                ti.len() == ci.len()
                    && ti.iter().zip(ci.iter()).all(|(t, c)| match (t, c) {
                        (TypeTupleItem::Positional(tt), TypeTupleItem::Positional(cc)) => {
                            self.unify_env(*tt, &tenv, *cc, &cenv, holes, subst, depth + 1)
                        }
                        (
                            TypeTupleItem::Named {
                                name: tn, ty: tt, ..
                            },
                            TypeTupleItem::Named {
                                name: cn, ty: cc, ..
                            },
                        ) => {
                            tn == cn
                                && self.unify_env(*tt, &tenv, *cc, &cenv, holes, subst, depth + 1)
                        }
                        _ => false,
                    })
            }
            (TypeKind::Record(tf, tt), TypeKind::Record(cf, ct)) => {
                tt == ct
                    && tf.len() == cf.len()
                    && tf.iter().zip(cf.iter()).all(|(t, c)| {
                        t.name == c.name
                            && t.optional == c.optional
                            && self.unify_env(t.ty, &tenv, c.ty, &cenv, holes, subst, depth + 1)
                    })
            }
            (TypeKind::Union(tv, tt), TypeKind::Union(cv, ct)) => {
                tt == ct
                    && tv.len() == cv.len()
                    && tv.iter().zip(cv.iter()).all(|(t, c)| {
                        t.name == c.name
                            && match (t.payload, c.payload) {
                                (Some(tp), Some(cp)) => {
                                    self.unify_env(tp, &tenv, cp, &cenv, holes, subst, depth + 1)
                                }
                                (None, None) => true,
                                _ => false,
                            }
                    })
            }
            (TypeKind::Function { from: tf, to: tt }, TypeKind::Function { from: cf, to: ct }) => {
                self.unify_env(tf, &tenv, cf, &cenv, holes, subst, depth + 1)
                    && self.unify_env(tt, &tenv, ct, &cenv, holes, subst, depth + 1)
            }
            // Non-hole leaves and everything else must match exactly.
            _ => self.thir_types_equal(target, concrete),
        }
    }

    /// Normalize a type for witness-target matching: follow `env` substitutions
    /// for non-hole `TypeVar`s and expand `Alias`/`AliasApply` (recording their
    /// type args in the env) until the head is a concrete constructor, a hole, or
    /// a free variable. Returns the resolved type and the env for its subterms.
    fn norm_ty(
        &self,
        ty: TypeId,
        env: &HashMap<BindingId, TypeId>,
        holes: &HashSet<BindingId>,
    ) -> (TypeId, HashMap<BindingId, TypeId>) {
        let mut ty = ty;
        let mut env = env.clone();
        let mut fuel = 64u32;
        while fuel > 0 {
            fuel -= 1;
            match self.thir.type_arena[ty.0 as usize].kind.clone() {
                TypeKind::TypeVar(b) if !holes.contains(&b) => match env.get(&b) {
                    Some(&next) => ty = next,
                    None => break,
                },
                TypeKind::Alias(b) => match self.type_alias_body(b) {
                    Some(body) => ty = body,
                    None => break,
                },
                TypeKind::AliasApply { binding, args } => {
                    match self.type_alias_params_body(binding) {
                        Some((params, body)) => {
                            for (p, a) in params.iter().zip(args.iter()) {
                                env.insert(*p, *a);
                            }
                            ty = body;
                        }
                        None => break,
                    }
                }
                _ => break,
            }
        }
        (ty, env)
    }
    /// Follow a `TypeVar` substitution chain through `env` (no alias expansion),
    /// yielding a self-contained `TypeId`. Used when binding a witness hole so the
    /// bound type keeps its `AliasApply` shape for later re-resolution.
    fn resolve_env_var(&self, ty: TypeId, env: &HashMap<BindingId, TypeId>) -> TypeId {
        let mut ty = ty;
        let mut fuel = 64u32;
        while fuel > 0 {
            fuel -= 1;
            match self.thir.type_arena[ty.0 as usize].kind {
                TypeKind::TypeVar(b) => match env.get(&b) {
                    Some(&next) => ty = next,
                    None => break,
                },
                _ => break,
            }
        }
        ty
    }

    /// Structural equality of two THIR types via their witness keys. Used to
    /// compare non-hole leaves and re-bound holes during target matching.
    fn thir_types_equal(&self, a: TypeId, b: TypeId) -> bool {
        if a == b {
            return true;
        }
        match (
            self.structural_witness_key(a, &mut HashSet::new()),
            self.structural_witness_key(b, &mut HashSet::new()),
        ) {
            (Some(ka), Some(kb)) => ka == kb,
            _ => false,
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
