use std::collections::{HashMap, HashSet};

use la_arena::Arena;
use zutai_hir::{
    BindingId, BindingKind, HirDecl, HirDeclId, HirDeclKind, HirExpr, HirExprId, HirFile, HirPat,
    HirPatId, HirTypeExpr, HirTypeId, HirTypeKind,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::import::{ImportKey, ImportedType};
use crate::ir::{
    Kind, RowTail, ThirDecl, ThirDeclId, ThirDeclKind, ThirExpr, ThirExprId, ThirExprKind,
    ThirFile, ThirPat, ThirPatId, Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
    UnionVariant,
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

/// The solution of a flexible row variable (`RowTail::Infer`): the extra
/// fields/members it captured plus the residual tail. Solved during row
/// unification and flattened away by zonking.
#[derive(Debug, Clone, PartialEq)]
enum RowSolution {
    Record {
        fields: Vec<TypeRecordField>,
        tail: RowTail,
    },
    Union {
        variants: Vec<UnionVariant>,
        tail: RowTail,
    },
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
    /// Next flexible row-variable id (`RowTail::Infer`). A separate id space from
    /// `next_infer_var` because row variables range over fields/members, not types.
    next_row_var: u32,
    /// Solutions for flexible row variables, mirroring `infer_subst` for types.
    row_subst: HashMap<u32, RowSolution>,
    /// HM let-generalization schemes: for each generalized binding, the list of
    /// `InferVar` ids quantified over. A reference instantiates these with fresh
    /// independent `InferVar`s so unifying one use site does not monomorphize others.
    /// Bindings absent here are monomorphic (used at a single type, or shared with
    /// the surrounding environment).
    poly_schemes: HashMap<BindingId, Vec<u32>>,
    /// Declared kind of each type parameter, from `<F :: Type -> Type>` kind
    /// annotations. Absent params are `Star`. Used for kind-checking higher-kinded
    /// constraints/witnesses and carried into `ThirFile` for TLC.
    type_param_kinds: HashMap<BindingId, Kind>,
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
            next_row_var: 0,
            row_subst: HashMap::new(),
            poly_schemes: HashMap::new(),
            type_param_kinds: HashMap::new(),
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
        self.collect_type_param_kinds();
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
        // Constraints before witnesses: a witness checks its fields against the
        // constraint's (instantiated) method signatures, so the constraint decl
        // must already be in `decl_arena`.
        let (constraint_ids, witness_ids): (Vec<_>, Vec<_>) = cw_ids
            .into_iter()
            .partition(|&id| matches!(self.hir_decl(id).kind, HirDeclKind::Constraint { .. }));
        for id in constraint_ids {
            id_map.insert(id, self.lower_decl(id));
        }
        for id in witness_ids {
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
            type_param_kinds: std::mem::take(&mut self.type_param_kinds),
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
    /// Populate `type_param_kinds` from every type parameter's `<.. :: Kind>`
    /// annotation across constraint, witness, function, and constraint-method
    /// param lists. Params without an annotation default to `Star` (absent).
    fn collect_type_param_kinds(&mut self) {
        let mut pending: Vec<(BindingId, Kind)> = Vec::new();
        for &decl_id in &self.hir.decls {
            let decl = self.hir_decl(decl_id);
            match &decl.kind {
                HirDeclKind::Constraint {
                    params, methods, ..
                } => {
                    for p in params {
                        if let Some(kind_ty) = p.kind {
                            pending.push((p.binding, self.hir_kind_of(kind_ty)));
                        }
                    }
                    for m in methods {
                        for p in &m.params {
                            if let Some(kind_ty) = p.kind {
                                pending.push((p.binding, self.hir_kind_of(kind_ty)));
                            }
                        }
                    }
                }
                HirDeclKind::Witness { params, .. } | HirDeclKind::Function { params, .. } => {
                    for p in params {
                        if let Some(kind_ty) = p.kind {
                            pending.push((p.binding, self.hir_kind_of(kind_ty)));
                        }
                    }
                }
                _ => {}
            }
        }
        for (binding, kind) in pending {
            self.type_param_kinds.insert(binding, kind);
        }
    }

    /// Interpret a kind annotation type-expression: `Type -> Type` → `Arrow`,
    /// everything else (the `Type` leaf) → `Star`.
    fn hir_kind_of(&self, hir_ty: HirTypeId) -> Kind {
        match &self.hir_type(hir_ty).kind {
            HirTypeKind::Arrow { from, to } => Kind::Arrow(
                Box::new(self.hir_kind_of(*from)),
                Box::new(self.hir_kind_of(*to)),
            ),
            _ => Kind::Star,
        }
    }

    /// Compute the kind of a type. `TypeVar` looks up its declared kind; `Con`
    /// (builtin `List`/`Optional`) is `Type -> Type`; a bare named `Alias` is the
    /// arrow chain of its arity; `Apply` drops one arrow off its head's kind.
    /// All saturated/ground forms are `Star`.
    pub(super) fn kind_of(&self, ty: TypeId) -> Kind {
        let ty = self.resolve(ty);
        match &self.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(b) => self.type_param_kinds.get(b).cloned().unwrap_or(Kind::Star),
            TypeKind::Con(_) => Kind::Arrow(Box::new(Kind::Star), Box::new(Kind::Star)),
            TypeKind::Alias(b) => {
                let arity = self.alias_params.get(b).map(|p| p.len()).unwrap_or(0);
                (0..arity).fold(Kind::Star, |acc, _| {
                    Kind::Arrow(Box::new(Kind::Star), Box::new(acc))
                })
            }
            TypeKind::Apply { func, .. } => match self.kind_of(*func) {
                Kind::Arrow(_, res) => *res,
                Kind::Star => Kind::Star,
            },
            _ => Kind::Star,
        }
    }
    /// Verify a type used in a value position (value annotation, function
    /// signature) is fully applied — kind `Star`. A partial application
    /// (`Pair Text`, kind `Type -> Type`) is not a value type; re-emit the
    /// `TypeConstructorArityMismatch` so v1's new partial-application support
    /// does not silently accept under-applied constructors outside witness
    /// targets. A saturated `F A` (kind `Star`) is fine; recurse its arguments.
    pub(super) fn require_ground_type(&mut self, ty: TypeId, span: Span) {
        let r = self.resolve(ty);
        match self.type_arena[r.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                self.require_ground_type(from, span);
                self.require_ground_type(to, span);
            }
            TypeKind::List(e) | TypeKind::Optional(e) => self.require_ground_type(e, span),
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.require_ground_type(f.ty, span);
                }
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(p) = v.payload {
                        self.require_ground_type(p, span);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let t = match item {
                        TypeTupleItem::Named { ty, .. } => ty,
                        TypeTupleItem::Positional(ty) => ty,
                    };
                    self.require_ground_type(t, span);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in args {
                    self.require_ground_type(a, span);
                }
            }
            TypeKind::Apply { .. } => {
                if self.kind_of(r) == Kind::Star {
                    let (_, args) = self.app_spine(r);
                    for a in args {
                        self.require_ground_type(a, span);
                    }
                } else {
                    self.report_underapplied(r, span);
                }
            }
            TypeKind::Con(binding) => {
                let name = self.hir.bindings[binding.0 as usize].name.clone();
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: 1,
                        found: 0,
                    },
                    span,
                });
            }
            TypeKind::Alias(_) if self.kind_of(r) != Kind::Star => {
                self.report_underapplied(r, span);
            }
            _ => {}
        }
    }

    /// Emit a `TypeConstructorArityMismatch` for an under-applied constructor
    /// spine (`Pair Text` → expected 2, found 1).
    fn report_underapplied(&mut self, ty: TypeId, span: Span) {
        let (head, args) = self.app_spine(ty);
        let head = self.resolve(head);
        match self.type_arena[head.0 as usize].kind.clone() {
            TypeKind::Alias(b) => {
                let name = self.hir.bindings[b.0 as usize].name.clone();
                let expected = self
                    .alias_params
                    .get(&b)
                    .map(|p| p.len())
                    .unwrap_or(args.len());
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected,
                        found: args.len(),
                    },
                    span,
                });
            }
            TypeKind::Con(b) => {
                let name = self.hir.bindings[b.0 as usize].name.clone();
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: 1,
                        found: args.len(),
                    },
                    span,
                });
            }
            _ => {
                self.invalid_type(
                    "higher-kinded type used where a concrete type is required",
                    span,
                );
            }
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

    /// Mint a fresh flexible row variable.
    pub(super) fn fresh_row_var(&mut self) -> RowTail {
        let id = self.next_row_var;
        self.next_row_var += 1;
        RowTail::Infer(id)
    }

    /// Flatten a record row `(fields, tail)` by appending every solved flexible
    /// tail's captured fields until the tail is rigid or unsolved.
    fn flatten_record_row(
        &self,
        mut fields: Vec<TypeRecordField>,
        mut tail: RowTail,
    ) -> (Vec<TypeRecordField>, RowTail) {
        while let RowTail::Infer(r) = tail {
            match self.row_subst.get(&r) {
                Some(RowSolution::Record {
                    fields: extra,
                    tail: next,
                }) => {
                    fields.extend(extra.iter().cloned());
                    tail = *next;
                }
                _ => break,
            }
        }
        (fields, tail)
    }

    /// Flatten a union row, analogous to `flatten_record_row`.
    fn flatten_union_row(
        &self,
        mut variants: Vec<UnionVariant>,
        mut tail: RowTail,
    ) -> (Vec<UnionVariant>, RowTail) {
        while let RowTail::Infer(r) = tail {
            match self.row_subst.get(&r) {
                Some(RowSolution::Union {
                    variants: extra,
                    tail: next,
                }) => {
                    variants.extend(extra.iter().cloned());
                    tail = *next;
                }
                _ => break,
            }
        }
        (variants, tail)
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
            TypeKind::Union(variants, _) => variants
                .iter()
                .any(|v| v.payload.is_some_and(|p| self.occurs(var_id, p))),
            TypeKind::Tuple(items) => items.iter().any(|item| {
                let inner = match item {
                    TypeTupleItem::Named { ty, .. } => *ty,
                    TypeTupleItem::Positional(ty) => *ty,
                };
                self.occurs(var_id, inner)
            }),
            TypeKind::Record(fields, _) => fields.iter().any(|f| self.occurs(var_id, f.ty)),
            TypeKind::Apply { func, arg } => self.occurs(var_id, func) || self.occurs(var_id, arg),
            TypeKind::AliasApply { args, .. } => args.iter().any(|&a| self.occurs(var_id, a)),
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

            // Higher-kinded application: decompose head and argument. Required so
            // method-level / constraint type params solve when unifying `F A`
            // shapes (`F A ~ F B`, `?f A ~ F A`). Structural `!=` would spuriously
            // mismatch two separately-built but equal `Apply` nodes.
            (TypeKind::Apply { func: f1, arg: a1 }, TypeKind::Apply { func: f2, arg: a2 }) => {
                self.unify(f1, f2, span);
                self.unify(a1, a2, span);
            }

            (left, right) => {
                // Cross-form applications: canonicalize via `resolve_alias` (folds
                // builtin `Con` apps and expands saturated named-alias apps) so a
                // saturated `Apply`/`AliasApply` meets its concrete form. Only retry
                // when reduction made progress, else fall through to mismatch.
                let app_like = |k: &TypeKind| {
                    matches!(
                        k,
                        TypeKind::Apply { .. } | TypeKind::AliasApply { .. } | TypeKind::Con(_)
                    )
                };
                if app_like(&left) || app_like(&right) {
                    let r1 = self.resolve_alias(t1, &mut HashSet::new(), span);
                    let r2 = self.resolve_alias(t2, &mut HashSet::new(), span);
                    if r1 != t1 || r2 != t2 {
                        self.unify(r1, r2, span);
                        return;
                    }
                }
                // NOTE: an abstract-headed application against a concrete
                // constructor (`Apply{?f, X} ~ List Y`) is intentionally *not*
                // bridged here (would need Miller-pattern `?f := Con(List)` then
                // `unify(X, Y)`). Concrete higher-kinded application is outside the
                // Phase 14 gate and a refused check is the safe direction; the arm
                // belongs to the later concrete-HKT-dispatch milestone.
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

        #[derive(Clone)]
        struct ConstraintInfo {
            name: String,
            params: Vec<BindingId>,
            methods: Vec<(String, bool, bool, TypeId)>,
            method_params: HashMap<String, Vec<BindingId>>,
            derivable: bool,
        }

        let mut constraint_map: HashMap<BindingId, ConstraintInfo> = HashMap::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Constraint {
                params,
                methods,
                derivable,
                ..
            } = &decl.kind
            {
                let owned_methods: Vec<(String, bool, bool, TypeId)> = methods
                    .iter()
                    .map(|m| (m.name.clone(), m.optional, m.default.is_some(), m.sig))
                    .collect();
                let owned_method_params: HashMap<String, Vec<BindingId>> = methods
                    .iter()
                    .map(|m| (m.name.clone(), m.params.clone()))
                    .collect();
                let name = self.hir.bindings[decl.binding.0 as usize].name.clone();
                constraint_map.insert(
                    decl.binding,
                    ConstraintInfo {
                        name,
                        params: params.clone(),
                        methods: owned_methods,
                        method_params: owned_method_params,
                        derivable: *derivable,
                    },
                );
            }
        }
        struct WitnessTask {
            span: Span,
            target: TypeId,
            constraint_param: BindingId,
            constraint_name: String,
            methods: Vec<(String, bool, bool, TypeId)>,
            method_params: HashMap<String, Vec<BindingId>>,
            fields: Vec<(String, ThirExprId, Span)>,
        }
        struct DeriveTask {
            span: Span,
            target: TypeId,
            constraint: BindingId,
            constraint_name: String,
            methods: Vec<(String, bool, bool, TypeId)>,
            derivable: bool,
        }
        let mut tasks: Vec<WitnessTask> = Vec::new();
        let mut derive_tasks: Vec<DeriveTask> = Vec::new();
        // Multi-param constraint names and their witness spans, collected for
        // diagnostic emission after the immutable scan loop ends.
        let mut multi_param_errors: Vec<(String, Span)> = Vec::new();
        // Conditional witnesses whose target may be self-referential; resolved and
        // checked after the immutable scan.
        #[allow(clippy::type_complexity)]
        let mut recursive_candidates: Vec<(
            String,
            TypeId,
            Vec<BindingId>,
            Vec<Vec<BindingId>>,
            BindingId,
            Span,
        )> = Vec::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Witness {
                constraint,
                target,
                params,
                param_bounds,
                derive,
                fields,
                ..
            } = &decl.kind
            {
                let Some(cst_binding) = constraint else {
                    continue;
                };
                let Some(cst_info) = constraint_map.get(cst_binding) else {
                    continue;
                };
                if cst_info.params.len() != 1 {
                    // Multi-param constraints are not yet supported: collect for
                    // diagnostic emission below (outside the immutable-borrow loop).
                    multi_param_errors.push((cst_info.name.clone(), decl.span));
                    continue;
                }
                if !params.is_empty() {
                    recursive_candidates.push((
                        cst_info.name.clone(),
                        *target,
                        params.clone(),
                        param_bounds.clone(),
                        *cst_binding,
                        decl.span,
                    ));
                }
                if *derive {
                    derive_tasks.push(DeriveTask {
                        span: decl.span,
                        target: *target,
                        constraint: *cst_binding,
                        constraint_name: cst_info.name.clone(),
                        methods: cst_info.methods.clone(),
                        derivable: cst_info.derivable,
                    });
                    continue;
                }
                let fields_owned: Vec<(String, ThirExprId, Span)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), f.value, f.span))
                    .collect();
                tasks.push(WitnessTask {
                    span: decl.span,
                    target: *target,
                    constraint_param: cst_info.params[0],
                    constraint_name: cst_info.name.clone(),
                    methods: cst_info.methods.clone(),
                    method_params: cst_info.method_params.clone(),
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

        // A conditional witness whose target is one of its own params loops only
        // when that param's bound requires the *same* constraint being defined
        // (`Eq @A :: <A: Eq>`): resolving `Eq A` then needs `Eq A` again. A bound
        // by a *different* constraint (`Eq @A :: <A: Ord>`) makes progress —
        // consuming an `Ord` dict to produce an `Eq` dict — and is not recursive.
        for (name, target, params, param_bounds, cst_binding, span) in recursive_candidates {
            let resolved = self.resolve_alias(target, &mut HashSet::new(), span);
            if let TypeKind::TypeVar(b) = self.type_arena[resolved.0 as usize].kind
                && let Some(idx) = params.iter().position(|p| *p == b)
                && param_bounds
                    .get(idx)
                    .is_some_and(|bounds| bounds.contains(&cst_binding))
            {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::RecursiveWitness { constraint: name },
                    span,
                });
            }
        }

        // Phase 2: mutable checks over owned task data.
        for task in tasks {
            // Kind-check the witness target against the constraint's target kind
            // (`Functor @Int` is rejected: `Int : Type` but `Functor` wants
            // `Type -> Type`). Skip field checks for a mis-kinded witness.
            let expected_kind = self
                .type_param_kinds
                .get(&task.constraint_param)
                .cloned()
                .unwrap_or(Kind::Star);
            let target_kind = self.kind_of(task.target);
            if expected_kind != target_kind {
                let target_name = self.type_name(task.target);
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::WitnessTargetKindMismatch {
                        constraint: task.constraint_name.clone(),
                        target: target_name,
                    },
                    span: task.span,
                });
                continue;
            }
            let subst: HashMap<BindingId, TypeId> =
                [(task.constraint_param, task.target)].into_iter().collect();

            let field_names: HashSet<String> =
                task.fields.iter().map(|(n, _, _)| n.clone()).collect();

            for (fname, value_expr, fspan) in &task.fields {
                if let Some((_, _, _, method_sig)) =
                    task.methods.iter().find(|(n, _, _, _)| n == fname)
                {
                    let mut field_subst = subst.clone();
                    if let Some(mps) = task.method_params.get(fname) {
                        let mspan = self.ty(*method_sig).span;
                        for &mp in mps {
                            let fresh = self.fresh_infer_var(mspan);
                            field_subst.insert(mp, fresh);
                        }
                    }
                    let expected = self.instantiate_type_vars(*method_sig, &field_subst);
                    let found = self.expr(*value_expr).ty;
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

        let explicit_witnesses = self.collect_explicit_witness_keys();
        for task in derive_tasks {
            if !task.derivable {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::DeriveConstraintNotDerivable {
                        constraint: task.constraint_name.clone(),
                    },
                    span: task.span,
                });
                continue;
            }
            let has_eq_method = task
                .methods
                .iter()
                .any(|(name, _, _, _)| derive_method_is_eq(name));
            let mut unsupported = false;
            for (name, optional, has_default, _) in &task.methods {
                if *optional || *has_default {
                    continue;
                }
                // A method is structurally derivable only if it is equality-family
                // AND a positive `eq`/`==` recipe exists to build on (a lone
                // `neq`/`!=` cannot be derived: there is nothing to negate).
                if !derive_method_is_equality(name) || !has_eq_method {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::DeriveUnsupportedMethod {
                            constraint: task.constraint_name.clone(),
                            method: name.clone(),
                        },
                        span: task.span,
                    });
                    unsupported = true;
                }
            }
            if unsupported {
                continue;
            }
            for component in self.derive_components(task.target) {
                if !self.derive_component_has_witness(
                    task.constraint,
                    component,
                    &task.methods,
                    &explicit_witnesses,
                ) {
                    let component_name = self.type_name(component);
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::DeriveComponentMissingWitness {
                            constraint: task.constraint_name.clone(),
                            component: component_name,
                        },
                        span: task.span,
                    });
                }
            }
        }
    }

    fn collect_explicit_witness_keys(&mut self) -> HashSet<(BindingId, String)> {
        let witnesses: Vec<(BindingId, TypeId)> = self
            .decl_arena
            .iter()
            .filter_map(|(_, decl)| {
                if let ThirDeclKind::Witness {
                    constraint: Some(constraint),
                    target,
                    ..
                } = &decl.kind
                {
                    Some((*constraint, *target))
                } else {
                    None
                }
            })
            .collect();

        witnesses
            .into_iter()
            .map(|(constraint, target)| (constraint, self.witness_target_key(target)))
            .collect()
    }

    fn derive_components(&mut self, target: TypeId) -> Vec<TypeId> {
        let span = self.type_arena[target.0 as usize].span;
        let target = self.resolve_alias(target, &mut HashSet::new(), span);
        match self.type_arena[target.0 as usize].kind.clone() {
            TypeKind::Record(fields, _) => fields.into_iter().map(|field| field.ty).collect(),
            TypeKind::Tuple(items) => items
                .into_iter()
                .map(|item| match item {
                    TypeTupleItem::Named { ty, .. } | TypeTupleItem::Positional(ty) => ty,
                })
                .collect(),
            TypeKind::Union(variants, _) => {
                let mut components = Vec::new();
                for variant in variants {
                    if let Some(payload) = variant.payload {
                        let payload_span = self.type_arena[payload.0 as usize].span;
                        let payload =
                            self.resolve_alias(payload, &mut HashSet::new(), payload_span);
                        match self.type_arena[payload.0 as usize].kind.clone() {
                            TypeKind::Record(fields, _) => {
                                components.extend(fields.into_iter().map(|field| field.ty));
                            }
                            _ => components.push(payload),
                        }
                    }
                }
                components
            }
            _ => Vec::new(),
        }
    }

    fn derive_component_has_witness(
        &mut self,
        constraint: BindingId,
        component: TypeId,
        methods: &[(String, bool, bool, TypeId)],
        witness_keys: &HashSet<(BindingId, String)>,
    ) -> bool {
        if self.derive_can_use_builtin_leaf(component, methods) {
            return true;
        }

        let key = self.witness_target_key(component);
        witness_keys.contains(&(constraint, key))
    }

    fn derive_can_use_builtin_leaf(
        &mut self,
        ty: TypeId,
        methods: &[(String, bool, bool, TypeId)],
    ) -> bool {
        if !methods
            .iter()
            .any(|(name, _, _, _)| derive_method_is_equality(name))
        {
            return false;
        }

        let span = self.type_arena[ty.0 as usize].span;
        let ty = self.resolve_alias(ty, &mut HashSet::new(), span);
        matches!(
            self.type_arena[ty.0 as usize].kind,
            TypeKind::Bool
                | TypeKind::True
                | TypeKind::False
                | TypeKind::Text
                | TypeKind::Int
                | TypeKind::Float
                | TypeKind::Atom(_)
        )
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
        // Phase 1: immutable scan — collect (constraint, target, params, span).
        let mut entries: Vec<(BindingId, TypeId, Vec<BindingId>, Span)> = Vec::new();
        for (_, decl) in self.decl_arena.iter() {
            if let ThirDeclKind::Witness {
                constraint: Some(cst),
                target,
                params,
                ..
            } = &decl.kind
            {
                entries.push((*cst, *target, params.clone(), decl.span));
            }
        }

        // Phase 2: mutable — compute param-normalized keys, detect duplicates so
        // two conditional witnesses that overlap (e.g. two `Eq @(List A)`) are
        // flagged as ambiguous.
        let mut seen: HashMap<(BindingId, String), ()> = HashMap::new();
        for (cst, target, params, span) in entries {
            let norm: HashMap<BindingId, usize> =
                params.iter().enumerate().map(|(i, &p)| (p, i)).collect();
            let target_key = self.witness_target_key_with(target, &norm);
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
        // Flatten solved flexible row tails in record/union types so consumers
        // see the captured fields/members inline with a rigid residual tail.
        for i in 0..self.type_arena.len() {
            match self.type_arena[i].kind.clone() {
                TypeKind::Record(fields, tail @ RowTail::Infer(_)) => {
                    let (fields, tail) = self.flatten_record_row(fields, tail);
                    self.type_arena[i].kind = TypeKind::Record(fields, tail);
                }
                TypeKind::Union(variants, tail @ RowTail::Infer(_)) => {
                    let (variants, tail) = self.flatten_union_row(variants, tail);
                    self.type_arena[i].kind = TypeKind::Union(variants, tail);
                }
                _ => {}
            }
        }
    }
}

fn derive_method_is_equality(method_name: &str) -> bool {
    matches!(method_name, "eq" | "==" | "neq" | "!=")
}

fn derive_method_is_eq(method_name: &str) -> bool {
    matches!(method_name, "eq" | "==")
}
