use rustc_hash::{FxHashMap, FxHashSet};

use la_arena::Arena;
use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{RowTail, ThirDeclKind, ThirFile, TypeId, TypeKind};

use crate::ir::{
    TlcDecl, TlcDeclId, TlcExpr, TlcExprId, TlcModule, TlcType, TlcTypeId, TlcTypeVar,
};

mod decl;
mod derive;
mod effects;
mod expr;
mod thir_query;
mod types;
mod witness;
mod witness_resolve;

pub use witness::ExternConditionalWitness;
use witness::{ConditionalWitness, ConstraintMethodInfo, WitnessTargetKey};

pub fn lower_thir(file: &ThirFile) -> TlcModule {
    let mut lowerer = Lowerer::new(file);
    let mut module = lowerer.lower_file();
    module.inline_effectful_calls();
    module.elaborate_effects();
    module.apply_entry_capabilities();
    if crate::residual_effect_reason(&module).is_none() {
        module.erase_effects();
    }
    module
}

/// Lower THIR to the pre-backend TLC shape.
///
/// Unlike [`lower_thir`], this does not run shared lexical effect inlining or
/// erasure. The native compile/dataflow path calls [`crate::lower_effects_for_backend`]
/// afterwards so effectful generator cells can be routed through the residual
/// reifier instead of being inlined for the interpreter oracle.
pub fn lower_thir_for_backend(file: &ThirFile) -> TlcModule {
    let mut lowerer = Lowerer::new(file);
    let mut module = lowerer.lower_file();
    module.apply_entry_capabilities();
    module
}

/// Lower a THIR file to TLC with extern witness information from imported dep modules.
///
/// `extern_witnesses` is a list of `(constraint_name, target_key_str, dc_global_name)` triples
/// for concrete imported witnesses. `extern_conditionals` carries imported *parametric*
/// witnesses (`Eq @(List A)`), matched structurally at a concrete call site.
///
/// When `get_dict_expr` fails to resolve a witness locally, it checks these; a matching
/// concrete entry is replaced with a virtual `Var` that the DC lowerer maps to
/// `GlobalRef(dc_global_name)`, and a matching conditional entry emits that virtual `Var`
/// applied (`TyApp`/`App`) to the recursively-resolved component dicts.
pub fn lower_thir_with_extern_witnesses(
    file: &ThirFile,
    extern_witnesses: Vec<(String, String, String)>,
    extern_conditionals: Vec<ExternConditionalWitness>,
) -> TlcModule {
    let mut lowerer = Lowerer::new(file);
    lowerer.extern_witnesses = extern_witnesses;
    lowerer.extern_conditionals = extern_conditionals;
    let mut module = lowerer.lower_file();
    module.inline_effectful_calls();
    module.elaborate_effects();
    module.apply_entry_capabilities();
    if crate::residual_effect_reason(&module).is_none() {
        module.erase_effects();
    }
    module
}

/// Backend variant of [`lower_thir_with_extern_witnesses`].
pub fn lower_thir_with_extern_witnesses_for_backend(
    file: &ThirFile,
    extern_witnesses: Vec<(String, String, String)>,
    extern_conditionals: Vec<ExternConditionalWitness>,
) -> TlcModule {
    let mut lowerer = Lowerer::new(file);
    lowerer.extern_witnesses = extern_witnesses;
    lowerer.extern_conditionals = extern_conditionals;
    let mut module = lowerer.lower_file();
    module.apply_entry_capabilities();
    module
}

struct Lowerer<'thir> {
    thir: &'thir ThirFile,
    decl_arena: Arena<TlcDecl>,
    expr_arena: Arena<TlcExpr>,
    type_arena: Arena<TlcType>,
    expr_types: FxHashMap<TlcExprId, TlcTypeId>,
    spans: FxHashMap<TlcExprId, Span>,
    dict_field_slots: FxHashMap<TlcExprId, usize>,
    /// Concrete witness-dispatch target key per constraint-method `GetField`
    /// (operand type's `target_key` string). Collected into
    /// `TlcModule::dict_dispatch_keys` for runtime imported-witness dispatch.
    dict_dispatch_keys: FxHashMap<TlcExprId, String>,
    type_cache: FxHashMap<u32, TlcTypeId>,
    infer_to_tyvar: FxHashMap<u32, TlcTypeVar>,
    named_to_tyvar: FxHashMap<u32, TlcTypeVar>,
    decl_thir_types: FxHashMap<BindingId, TypeId>,
    next_synth: u32,
    /// constraint method BindingId → (constraint BindingId, method name).
    /// Used in the Apply arm to dispatch to `GetField` on the active dict param.
    constraint_methods: FxHashMap<BindingId, ConstraintMethodInfo>,
    /// Constraint operator methods in declaration order for binary operator lowering.
    operator_methods: Vec<ConstraintMethodInfo>,
    /// (constraint BindingId.0, WitnessTargetKey) → witness decl BindingId.
    /// Populated for every `Witness` THIR decl; queried at concrete call sites.
    witness_bindings: FxHashMap<(u32, WitnessTargetKey), BindingId>,
    /// function BindingId → vec of (type-param BindingId, constraint BindingIds),
    /// sorted ascending by type-param BindingId.0 to match THIR `collect_type_vars`.
    fn_explicit_params: FxHashMap<BindingId, Vec<(BindingId, Vec<BindingId>)>>,
    /// (constraint BindingId.0, type-param BindingId.0) → active dict Lam BindingId.
    /// Set when entering a bounded function body; cleared on exit.
    active_dict_params: FxHashMap<(u32, u32), BindingId>,
    /// dict Lam BindingId → its TLC type (Record placeholder).
    active_dict_types: FxHashMap<BindingId, TlcTypeId>,
    /// Next fresh row-variable id for anonymous open rows (`...`). Allocated from
    /// the top of the id space and mapped to `TlcTypeVar::Inferred`, so it never
    /// collides with a THIR `InferVar` id (small, counted from zero).
    next_row_var: u32,
    /// Parametric witnesses, matched structurally at concrete call sites.
    conditional_witnesses: Vec<ConditionalWitness>,
    /// Recursion guard for conditional-witness resolution: `(constraint.0,
    /// concrete TypeId.0)` pairs currently being resolved. Re-entry signals a
    /// non-terminating witness search; resolution bails to avoid a stack overflow.
    resolving_dicts: FxHashSet<(u32, u32)>,
    /// Recursion guard for imported conditional-witness resolution, keyed by
    /// `(constraint name, concrete TypeId.0)`. Name-keyed (not binding-keyed)
    /// because a component constraint may not be declared in this module.
    resolving_extern: FxHashSet<(String, u32)>,
    /// Operator-method witness body currently being lowered, as `(witness decl
    /// binding, operator name)`. While set, an operator call inside the body
    /// whose dispatch would resolve back to *this same* witness method falls back
    /// to the builtin instead of re-dispatching — otherwise `(==) = \a b. a == b`
    /// would call itself forever. Cleared once the body is lowered.
    defining_op_witness: Option<(BindingId, String)>,
    /// Extern witness entries from imported dep modules.
    /// Each entry is `(constraint_name, target_key_str, dc_global_name)`.
    /// Checked by `get_dict_expr` after local lookup fails.
    extern_witnesses: Vec<(String, String, String)>,
    /// Imported parametric (conditional) witnesses, matched structurally at a
    /// concrete call site after concrete local/extern lookup fails.
    extern_conditionals: Vec<ExternConditionalWitness>,
    /// Virtual bindings allocated for extern witness globals.
    /// Keys are synthetic `BindingId`s above the THIR binding range; values are
    /// the DC global names they resolve to.  Collected into `TlcModule::extern_global_bindings`.
    extern_global_bindings: FxHashMap<BindingId, String>,
    /// Counter for virtual BindingId allocation; starts just above the THIR
    /// binding-names array length and counts upward so it never collides with a
    /// real BindingId (`0..len`), no matter how many virtual globals are needed.
    next_virtual_binding: u32,
}

impl<'thir> Lowerer<'thir> {
    fn new(thir: &'thir ThirFile) -> Self {
        let next_virtual_binding = thir.binding_names.len() as u32 + 1;
        Self {
            thir,
            decl_arena: Arena::new(),
            expr_arena: Arena::new(),
            type_arena: Arena::new(),
            expr_types: FxHashMap::default(),
            spans: FxHashMap::default(),
            dict_field_slots: FxHashMap::default(),
            dict_dispatch_keys: FxHashMap::default(),
            type_cache: FxHashMap::default(),
            infer_to_tyvar: FxHashMap::default(),
            named_to_tyvar: FxHashMap::default(),
            decl_thir_types: FxHashMap::default(),
            next_synth: u32::MAX,
            constraint_methods: FxHashMap::default(),
            operator_methods: Vec::new(),
            witness_bindings: FxHashMap::default(),
            fn_explicit_params: FxHashMap::default(),
            active_dict_params: FxHashMap::default(),
            active_dict_types: FxHashMap::default(),
            next_row_var: u32::MAX,
            conditional_witnesses: Vec::new(),
            resolving_dicts: FxHashSet::default(),
            resolving_extern: FxHashSet::default(),
            defining_op_witness: None,
            extern_witnesses: Vec::new(),
            extern_conditionals: Vec::new(),
            extern_global_bindings: FxHashMap::default(),
            next_virtual_binding,
        }
    }

    /// Allocate a fresh virtual `BindingId` for an extern witness global and
    /// record the mapping `virtual_id → dc_global_name`.
    fn alloc_virtual_binding(&mut self, dc_global: String) -> BindingId {
        let id = self.next_virtual_binding;
        self.next_virtual_binding += 1; // count upward, staying above the real range
        let bid = BindingId(id);
        self.extern_global_bindings.insert(bid, dc_global);
        bid
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
            dict_field_slots: std::mem::take(&mut self.dict_field_slots),
            dict_dispatch_keys: std::mem::take(&mut self.dict_dispatch_keys),
            spans: std::mem::take(&mut self.spans),
            extern_global_bindings: std::mem::take(&mut self.extern_global_bindings),
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
                            let info = ConstraintMethodInfo {
                                constraint: decl.binding,
                                name: method.name.clone(),
                                constraint_param,
                                method_params: method.params.clone(),
                            };
                            self.constraint_methods.insert(binding, info.clone());
                            if method.is_operator {
                                self.operator_methods.push(info);
                            }
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

    fn register_dict_field_slot(&mut self, expr: TlcExprId, constraint: BindingId, method: &str) {
        let slot = self.dict_method_slot(constraint, method);
        self.dict_field_slots.insert(expr, slot);
    }

    fn dict_method_slot(&self, constraint: BindingId, method: &str) -> usize {
        let mut names = self.constraint_method_names(constraint);
        names.sort_unstable();
        names.iter().position(|&name| name == method).unwrap_or(0)
    }

    fn constraint_method_names(&self, constraint: BindingId) -> Vec<&str> {
        self.thir
            .decls
            .iter()
            .find_map(|&decl_id| {
                let decl = &self.thir.decl_arena[decl_id];
                if decl.binding == constraint
                    && let ThirDeclKind::Constraint { methods, .. } = &decl.kind
                {
                    return Some(methods.iter().map(|method| method.name.as_str()).collect());
                }
                None
            })
            .unwrap_or_default()
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
}
