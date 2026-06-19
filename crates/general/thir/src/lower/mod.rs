use std::collections::{HashMap, HashSet};

use la_arena::Arena;
use zutai_hir::{
    BindingId, BindingKind, HirDecl, HirDeclId, HirDeclKind, HirExpr, HirExprId, HirFile, HirPat,
    HirPatId, HirTypeExpr, HirTypeId,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::import::{ImportKey, ImportedType};
use crate::ir::{
    ThirDecl, ThirDeclId, ThirDeclKind, ThirExpr, ThirExprId, ThirExprKind, ThirFile, ThirPat,
    ThirPatId, Type, TypeId, TypeKind, TypeTupleItem,
};

pub(super) type BindingImportKey = HashMap<BindingId, ImportKey>;
use crate::pass::{ThirPassReport, run_default_passes};

mod decl;
mod exhaust;
mod expr;
mod import;
mod pat;
mod types;

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredThir {
    pub file: Option<ThirFile>,
    pub diagnostics: Vec<ThirDiagnostic>,
    pub pass_reports: Vec<ThirPassReport>,
}

#[derive(Debug, Clone)]
pub struct ThirLowerOptions {
    pub run_passes: bool,
    /// Pre-resolved import types, keyed by import source.  Built by the semantic
    /// layer (which owns filesystem access); empty when lowering with no module
    /// context, in which case every `import` becomes an unsupported `Error` node.
    pub imports: HashMap<ImportKey, ImportedType>,
    /// Override the type-level alias expansion fuel budget.  `None` uses the
    /// default (10 000 steps).  Set to a small value in tests to trigger the
    /// `TypeLevelEvalLimitExceeded` diagnostic deterministically.
    pub type_eval_fuel: Option<u32>,
}

impl Default for ThirLowerOptions {
    fn default() -> Self {
        Self {
            run_passes: true,
            imports: HashMap::new(),
            type_eval_fuel: None,
        }
    }
}

pub fn lower_hir(file: &zutai_hir::HirFile) -> LoweredThir {
    lower_hir_with_options(file, ThirLowerOptions::default())
}

pub fn lower_hir_with_options(file: &zutai_hir::HirFile, options: ThirLowerOptions) -> LoweredThir {
    let ThirLowerOptions {
        run_passes,
        imports,
        type_eval_fuel,
    } = options;
    let mut lowerer = Lowerer::new(file, imports);
    if let Some(fuel) = type_eval_fuel {
        lowerer.type_eval_fuel = fuel;
    }
    let mut lowered = lowerer.lower_file();
    if run_passes {
        lowered.pass_reports = run_default_passes(&mut lowered.file, &mut lowered.diagnostics);
    }
    lowered
}

struct Lowerer<'hir> {
    hir: &'hir HirFile,
    imports: HashMap<ImportKey, ImportedType>,
    decl_arena: Arena<ThirDecl>,
    expr_arena: Arena<ThirExpr>,
    pat_arena: Arena<ThirPat>,
    type_arena: Vec<Type>,
    aliases: HashMap<BindingId, TypeId>,
    value_types: HashMap<BindingId, TypeId>,
    diagnostics: Vec<ThirDiagnostic>,
    error_type: TypeId,
    type_type: TypeId,
    next_infer_var: u32,
    infer_subst: HashMap<u32, TypeId>,
    /// HM let-generalization schemes: for each generalized binding, the list of
    /// `InferVar` ids quantified over. A reference instantiates these with fresh
    /// independent `InferVar`s so unifying one use site does not monomorphize others.
    /// Bindings absent here are monomorphic (used at a single type, or shared with
    /// the surrounding environment).
    poly_schemes: HashMap<BindingId, Vec<u32>>,
    /// Params of each parametric type constructor (generic alias or type-level
    /// function), keyed by binding. Presence marks the binding as a parametric
    /// constructor applied via `AliasApply` at use sites.
    alias_params: HashMap<BindingId, Vec<BindingId>>,
    /// Bindings currently in scope as type-variable substitution targets while
    /// lowering a type-level function's body. Populated transiently during
    /// `lower_decl` for type-level functions so that `Param` bindings used in
    /// a type expression map to `TypeKind::TypeVar` instead of erroring.
    type_param_scope: HashSet<BindingId>,
    /// Total type-level alias expansion budget. Decremented in `resolve_alias`
    /// on every expansion step. When it reaches zero a `TypeLevelEvalLimitExceeded`
    /// diagnostic is emitted and expansion short-circuits to the error type.
    type_eval_fuel: u32,
    /// Maps each import-decl binding to its import source key.
    /// Populated during `lower_decl` when the value RHS is an `Import` expr.
    /// Used by the annotation-position `HirTypeKind::Access` arm to resolve
    /// e.g. `serverLib` → `"server.zt"` so it can look up `import_type_denotations`.
    pub(super) binding_import_key: BindingImportKey,
    /// Maps `(import_source, field_name)` → concrete denotation `TypeId` for
    /// type-valued fields exported by `.zt` modules.
    /// Populated during `intern_imported_type_with_source` when a field's
    /// `ImportedType` is `Type(inner)`.
    /// Queried by the `HirTypeKind::Access` arm when the field's type is
    /// `TypeKind::Type` and the receiver is a known import binding.
    pub(super) import_type_denotations: HashMap<(ImportKey, String), TypeId>,
}

impl<'hir> Lowerer<'hir> {
    fn new(hir: &'hir HirFile, imports: HashMap<ImportKey, ImportedType>) -> Self {
        let mut lowerer = Self {
            hir,
            imports,
            decl_arena: Arena::new(),
            expr_arena: Arena::new(),
            pat_arena: Arena::new(),
            type_arena: Vec::new(),
            aliases: HashMap::new(),
            value_types: HashMap::new(),
            diagnostics: Vec::new(),
            error_type: TypeId(0),
            type_type: TypeId(0),
            next_infer_var: 0,
            infer_subst: HashMap::new(),
            poly_schemes: HashMap::new(),
            alias_params: HashMap::new(),
            type_param_scope: HashSet::new(),
            type_eval_fuel: 10_000,
            binding_import_key: HashMap::new(),
            import_type_denotations: HashMap::new(),
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
        // D5: Two-phase lowering.  Witness field RHSs may forward-reference later
        // top-level bindings that are unannotated (not pre-declared by
        // `predeclare_decl_types`).  Lowering normal decls first populates
        // `value_types` for all of them, letting constraint/witness lowering see a
        // complete top-level environment and avoiding `ValueTypeUnavailable` errors.
        //
        // Output order is always the original `hir.decls` source order so downstream
        // positional assumptions stay intact — the partition controls *lowering*
        // order, not *output* order.
        let (cw_ids, normal_ids): (Vec<_>, Vec<_>) =
            self.hir.decls.iter().copied().partition(|&id| {
                matches!(
                    self.hir_decl(id).kind,
                    HirDeclKind::Constraint { .. } | HirDeclKind::Witness { .. }
                )
            });
        let mut id_map: HashMap<HirDeclId, ThirDeclId> = HashMap::new();
        for id in normal_ids {
            id_map.insert(id, self.lower_decl(id));
        }
        for id in cw_ids {
            id_map.insert(id, self.lower_decl(id));
        }
        self.check_witnesses();
        self.check_witness_coherence();
        // Reassemble in source order.
        let decls: Vec<_> = self
            .hir
            .decls
            .iter()
            .copied()
            .map(|id| id_map[&id])
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
            poly_schemes: std::mem::take(&mut self.poly_schemes),
            binding_names: self
                .hir
                .bindings
                .iter()
                .map(|binding| binding.name.clone())
                .collect(),
        };
        let diagnostics = std::mem::take(&mut self.diagnostics);

        LoweredThir {
            file: diagnostics.is_empty().then_some(file),
            diagnostics,
            pass_reports: Vec::new(),
        }
    }

    fn seed_builtin_value_types(&mut self) {
        for index in 0..self.hir.bindings.len() {
            let (kind, name) = {
                let binding = &self.hir.bindings[index];
                (binding.kind, binding.name.clone())
            };
            let id = BindingId(index as u32);
            match kind {
                BindingKind::BuiltinType => {
                    self.value_types.insert(id, self.type_type);
                }
                BindingKind::BuiltinValue => {
                    if let Some(ty) = self.builtin_value_type(&name) {
                        self.value_types.insert(id, ty);
                    }
                }
                _ => {}
            }
        }
    }

    /// Type of a compiler-provided value binding (the prelude). `print` is the
    /// only one in v0: a string-only `Text -> Text` debugging builtin that
    /// returns its argument.
    fn builtin_value_type(&mut self, name: &str) -> Option<TypeId> {
        let span = self.hir.span;
        match name {
            "print" => {
                let text = self.text_type(span);
                Some(self.alloc_type(Type {
                    kind: TypeKind::Function {
                        from: text,
                        to: text,
                    },
                    span,
                }))
            }
            _ => None,
        }
    }

    fn alloc_decl(&mut self, decl: ThirDecl) -> ThirDeclId {
        self.decl_arena.alloc(decl)
    }

    fn alloc_expr(&mut self, expr: ThirExpr) -> ThirExprId {
        self.expr_arena.alloc(expr)
    }

    fn alloc_pat(&mut self, pat: ThirPat) -> ThirPatId {
        self.pat_arena.alloc(pat)
    }

    fn alloc_type(&mut self, ty: Type) -> TypeId {
        let id = TypeId(self.type_arena.len() as u32);
        self.type_arena.push(ty);
        id
    }

    fn hir_decl(&self, id: HirDeclId) -> &'hir HirDecl {
        &self.hir.decl_arena[id]
    }

    fn hir_expr(&self, id: HirExprId) -> &'hir HirExpr {
        &self.hir.expr_arena[id]
    }

    fn hir_type(&self, id: HirTypeId) -> &'hir HirTypeExpr {
        &self.hir.type_arena[id]
    }

    fn hir_pat(&self, id: HirPatId) -> &'hir HirPat {
        &self.hir.pat_arena[id]
    }

    fn expr(&self, id: ThirExprId) -> &ThirExpr {
        &self.expr_arena[id]
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

    fn unsupported_type(&mut self, feature: &'static str, span: Span) -> TypeId {
        self.unsupported(feature, span);
        self.error_type
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
            TypeKind::Union(variants) => variants
                .iter()
                .any(|v| v.payload.is_some_and(|p| self.occurs(var_id, p))),
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

    /// Check each non-derive witness's fields against the corresponding constraint's
    /// method signatures, with the constraint's type param substituted by the witness
    /// target type. Emits WitnessFieldTypeMismatch, MissingWitnessField, and
    /// UnknownWitnessField diagnostics. Must run after the entire cw-lowering loop
    /// (D7) and before zonk_type_arena() so infer-var solutions get zonked.
    fn check_witnesses(&mut self) {
        // Phase 1: immutable scan — collect owned data to avoid borrow conflicts.

        // constraint binding → (params, methods: (name, optional, has_default, sig))
        #[allow(clippy::type_complexity)]
        let mut constraint_map: HashMap<
            BindingId,
            (Vec<BindingId>, Vec<(String, bool, bool, TypeId)>),
        > = HashMap::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Constraint {
                params, methods, ..
            } = &decl.kind
            {
                let owned_methods: Vec<(String, bool, bool, TypeId)> = methods
                    .iter()
                    .map(|m| (m.name.clone(), m.optional, m.default.is_some(), m.sig))
                    .collect();
                constraint_map.insert(decl.binding, (params.clone(), owned_methods));
            }
        }

        struct WitnessTask {
            span: Span,
            target: TypeId,
            constraint_param: BindingId,
            methods: Vec<(String, bool, bool, TypeId)>,
            fields: Vec<(String, TypeId, Span)>,
        }
        let mut tasks: Vec<WitnessTask> = Vec::new();
        // Multi-param constraint names and their witness spans, collected for
        // diagnostic emission after the immutable scan loop ends.
        let mut multi_param_errors: Vec<(String, Span)> = Vec::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Witness {
                constraint,
                target,
                derive,
                fields,
                ..
            } = &decl.kind
            {
                if *derive {
                    continue;
                }
                let Some(cst_binding) = constraint else {
                    continue;
                };
                let Some((cst_params, cst_methods)) = constraint_map.get(cst_binding) else {
                    continue;
                };
                if cst_params.len() != 1 {
                    // Multi-param constraints are not yet supported: collect for
                    // diagnostic emission below (outside the immutable-borrow loop).
                    let cst_name = self.hir.bindings[cst_binding.0 as usize].name.clone();
                    multi_param_errors.push((cst_name, decl.span));
                    continue;
                }
                let fields_owned: Vec<(String, TypeId, Span)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.expr(f.value).ty, f.span))
                    .collect();
                tasks.push(WitnessTask {
                    span: decl.span,
                    target: *target,
                    constraint_param: cst_params[0],
                    methods: cst_methods.clone(),
                    fields: fields_owned,
                });
            }
        }

        // Emit UnsupportedMultiParamConstraint diagnostics (collected above).
        for (name, span) in multi_param_errors {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::UnsupportedMultiParamConstraint { name },
                span,
            });
        }

        // Phase 2: mutable checks over owned task data.
        for task in tasks {
            let subst: HashMap<BindingId, TypeId> =
                [(task.constraint_param, task.target)].into_iter().collect();

            let field_names: HashSet<String> =
                task.fields.iter().map(|(n, _, _)| n.clone()).collect();

            for (fname, value_ty, fspan) in &task.fields {
                if let Some((_, _, _, method_sig)) =
                    task.methods.iter().find(|(n, _, _, _)| n == fname)
                {
                    let expected = self.instantiate_type_vars(*method_sig, &subst);
                    let found = *value_ty;
                    let expected_name = self.type_name(expected);
                    let found_name = self.type_name(found);
                    if !self.type_matches(expected, found) {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::WitnessFieldTypeMismatch {
                                name: fname.clone(),
                                expected: expected_name,
                                found: found_name,
                            },
                            span: *fspan,
                        });
                    }
                } else {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::UnknownWitnessField {
                            name: fname.clone(),
                        },
                        span: *fspan,
                    });
                }
            }

            for (mname, optional, has_default, _) in &task.methods {
                // D6/4a: suppress MissingWitnessField when the method has a default body.
                if !optional && !has_default && !field_names.contains(mname) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::MissingWitnessField {
                            name: mname.clone(),
                        },
                        span: task.span,
                    });
                }
            }
        }
    }

    /// Enforce coherence: at most one witness per `(Constraint, Type)` pair.
    ///
    /// For each non-`derive` or `derive` witness whose `constraint` binding is
    /// resolved, compute a structural key `(constraint_binding, target_key)`.
    /// If a prior witness already claimed that key, emit `ConflictingWitness` at
    /// the later witness's span. Witnesses with `constraint == None` (unresolved
    /// constraint name) are skipped — that error is reported elsewhere.
    ///
    /// Must run after `check_witnesses` and before `zonk_type_arena()`.
    fn check_witness_coherence(&mut self) {
        // Phase 1: immutable scan — collect (constraint, target, span) triples.
        let mut triples: Vec<(BindingId, TypeId, Span)> = Vec::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Witness {
                constraint: Some(cst),
                target,
                ..
            } = &decl.kind
            {
                triples.push((*cst, *target, decl.span));
            }
        }

        // Phase 2: mutable — compute keys, detect duplicates.
        let mut seen: HashMap<(BindingId, String), ()> = HashMap::new();
        for (cst, target, span) in triples {
            let target_key = self.witness_target_key(target);
            let key = (cst, target_key);
            if let std::collections::hash_map::Entry::Vacant(entry) = seen.entry(key) {
                entry.insert(());
            } else {
                let constraint_name = self.hir.bindings[cst.0 as usize].name.clone();
                let target_name = self.type_name(target);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ConflictingWitness {
                        constraint: constraint_name,
                        target: target_name,
                    },
                    span,
                });
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
