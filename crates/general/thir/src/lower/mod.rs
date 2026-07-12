use rustc_hash::{FxHashMap, FxHashSet};

use la_arena::Arena;
use zutai_hir::{
    BindingId, BindingKind, HirDecl, HirDeclId, HirDeclKind, HirExpr, HirExprId, HirFile, HirPat,
    HirPatId, HirTypeExpr, HirTypeId,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::import::{ImportKey, ImportedProvenance, ImportedType, ImportedTypeOrigin};
use crate::ir::{
    EffectOp, EffectRow, Kind, RowTail, ThirDecl, ThirDeclId, ThirDeclKind, ThirExpr, ThirExprId,
    ThirExprKind, ThirFile, ThirPat, ThirPatId, Type, TypeId, TypeKind, TypeRecordField,
    TypeTupleItem, UnionVariant, UniverseLevel,
};

pub(super) type BindingImportKey = FxHashMap<BindingId, ImportKey>;
use crate::pass::{ThirPassReport, run_default_passes};

mod arena;
mod builtins;
mod decl;
mod driver;
mod effects;
mod exhaust;
mod expr;
mod import;
mod pat;
mod types;
mod unify;
mod witnesses;

use types::WrapperKind;

const FIELD_SECTION_PARAM: &str = "__zt_field_section";
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
    pub imports: FxHashMap<ImportKey, ImportedType>,
    /// Source trees for `.zti` imports. `.zt` imports intentionally have no
    /// immediate-data provenance.
    pub import_provenance: FxHashMap<ImportKey, ImportedProvenance>,
    /// Override the type-level alias expansion fuel budget.  `None` uses the
    /// default (10 000 steps).  Set to a small value in tests to trigger the
    /// `TypeLevelEvalLimitExceeded` diagnostic deterministically.
    pub type_eval_fuel: Option<u32>,
}

impl Default for ThirLowerOptions {
    fn default() -> Self {
        Self {
            run_passes: true,
            imports: FxHashMap::default(),
            import_provenance: FxHashMap::default(),
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
        import_provenance,
        type_eval_fuel,
    } = options;
    let mut lowerer = Lowerer::new(file, imports, import_provenance);
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
    Effect {
        ops: Vec<EffectOp>,
        tail: RowTail,
    },
}

struct Lowerer<'hir> {
    hir: &'hir HirFile,
    imports: FxHashMap<ImportKey, ImportedType>,
    import_provenance: FxHashMap<ImportKey, ImportedProvenance>,
    imported_type_origins: FxHashMap<TypeId, ImportedTypeOrigin>,
    decl_arena: Arena<ThirDecl>,
    expr_arena: Arena<ThirExpr>,
    pat_arena: Arena<ThirPat>,
    type_arena: Vec<Type>,
    aliases: FxHashMap<BindingId, TypeId>,
    value_types: FxHashMap<BindingId, TypeId>,
    diagnostics: Vec<ThirDiagnostic>,
    error_type: TypeId,
    type_type: TypeId,
    next_infer_var: u32,
    infer_subst: FxHashMap<u32, TypeId>,
    /// Next flexible row-variable id (`RowTail::Infer`). A separate id space from
    /// `next_infer_var` because row variables range over fields/members, not types.
    next_row_var: u32,
    /// Solutions for flexible row variables, mirroring `infer_subst` for types.
    row_subst: FxHashMap<u32, RowSolution>,
    effect_ambient: EffectRow,
    handled_stack: Vec<FxHashMap<String, (TypeId, TypeId)>>,
    resume_stack: Vec<(TypeId, TypeId)>,
    /// HM let-generalization schemes: for each generalized binding, the list of
    /// `InferVar` ids quantified over. A reference instantiates these with fresh
    /// independent `InferVar`s so unifying one use site does not monomorphize others.
    /// Bindings absent here are monomorphic (used at a single type, or shared with
    /// the surrounding environment).
    poly_schemes: FxHashMap<BindingId, Vec<u32>>,
    /// Per-import working map from an exported type-parameter id (`ImportedType::
    /// TyVar`) to the single fresh `InferVar` it interns to, so repeated
    /// occurrences of one exported variable share a var (`∀A. A -> A` stays
    /// `?a -> ?a`). Cleared before interning each import binding's type.
    import_tyvar_cache: FxHashMap<u32, TypeId>,
    /// Per-import row-parameter cache for exported row variables (`...e`).
    /// Shared row tails in one imported signature must map to one synthetic
    /// rigid parameter so call-site instantiation preserves sharing.
    import_rowvar_cache: FxHashMap<u32, BindingId>,
    /// Row-parameter caches captured during import predeclaration and replayed
    /// when the corresponding import expression is lowered later.
    import_rowvar_caches: FxHashMap<ImportKey, FxHashMap<u32, BindingId>>,
    /// For each import binding, the inference variables interned from its exported
    /// type parameters (`ImportedType::TyVar`) — the *only* vars to generalize.
    /// `Unknown` positions (un-exportable types) are deliberately excluded so they
    /// stay monomorphic-by-use rather than being unsoundly quantified.
    import_poly_candidates: FxHashMap<BindingId, Vec<TypeId>>,
    /// Declared kind of each type parameter, from `<F :: Type -> Type>` kind
    /// annotations. Absent params default to `Kind::ground()`. Used for
    /// kind-checking higher-kinded constraints/witnesses and carried into
    /// `ThirFile` for TLC.
    type_param_kinds: FxHashMap<BindingId, Kind>,
    next_level_meta: u32,
    level_lower_bounds: FxHashMap<u32, u32>,
    level_equalities: FxHashMap<u32, UniverseLevel>,
    /// Per-decl scope mapping each in-scope `<$l>` level binder to the single
    /// fresh meta shared by all its occurrences (per-use linking, not prenex
    /// polymorphism). Populated before a signature is lowered, cleared after.
    level_param_metas: FxHashMap<BindingId, UniverseLevel>,
    type_universe_cache: FxHashMap<TypeId, UniverseLevel>,
    /// Alias bindings whose universe level is currently being computed. Guards
    /// `alias_apply_universe` against equirecursive generic aliases (e.g.
    /// `Tree :: <A> type { #node : { left : Tree A; ... } }`): re-instantiating
    /// the body on each call mints fresh `TypeId`s that defeat the
    /// `type_universe_cache` cycle break, so a recursive occurrence is treated as
    /// the fixpoint base (`Known(0)`) instead of expanding forever.
    alias_universe_in_progress: FxHashSet<BindingId>,
    /// Type pairs currently being matched. Recursive aliases can unfold back to
    /// the same pair through record/union fields; re-entry means the equirecursive
    /// comparison has reached its fixpoint.
    type_match_in_progress: FxHashSet<(TypeId, TypeId)>,
    /// Alias-head pairs currently being matched. Recursive alias instantiation
    /// mints fresh `TypeId`s at each unfold, so `type_match_in_progress` alone
    /// cannot see a cross-module recursive pair (`Stream` vs imported
    /// `s.Stream`) re-entering through its own tail.
    alias_match_in_progress: FxHashSet<(BindingId, BindingId)>,
    /// Memoized "is this alias (directly or mutually) recursive?" — gates the
    /// bidirectional same-binding AliasApply fast path in `type_matches`.
    alias_recursive_cache: FxHashMap<BindingId, bool>,
    /// Params of each parametric type constructor (generic alias or type-level
    /// function), keyed by binding. Presence marks the binding as a parametric
    /// constructor applied via `AliasApply` at use sites.
    alias_params: FxHashMap<BindingId, Vec<BindingId>>,
    /// Generated field-section lambda params currently being checked. While one
    /// is active, `_.field` bodies may constrain an unknown receiver into an open
    /// record; ordinary `\x. x.field` and no-signature functions keep the
    /// annotation-required diagnostic.
    field_section_params: FxHashSet<BindingId>,
    /// Bindings currently in scope as type-variable substitution targets while
    /// lowering a type-level function's body. Populated transiently during
    /// `lower_decl` for type-level functions so that `Param` bindings used in
    /// a type expression map to `TypeKind::TypeVar` instead of erroring.
    type_param_scope: FxHashSet<BindingId>,
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
    pub(super) import_type_denotations: FxHashMap<(ImportKey, String), TypeId>,
    /// Synthetic bindings minted during import interning for imported parametric
    /// type constructors and their type parameters. HIR cannot mint `BindingId`s,
    /// so these get ids past the HIR range (`hir.bindings.len() + index`); name
    /// and kind reads route through `binding_name`/`binding_kind`, and the names
    /// are appended to `ThirFile::binding_names`/`binding_kinds` on finalize.
    pub(super) synthetic_bindings: Vec<(String, BindingKind)>,
    /// `(import source, exported constructor field name)` → the synthetic
    /// constructor binding it was rebuilt as, so `s.Stream Int` resolves to the
    /// local parametric alias. Mirrors `import_type_denotations` for the
    /// *parametric* (constructor) case.
    pub(super) import_type_constructors: FxHashMap<(ImportKey, String), BindingId>,
    /// Active map from an imported constructor's exported type-parameter id
    /// (`ImportedType::TyVar`) to the synthetic `TypeParam` binding it interns to,
    /// set while interning that constructor's body so a `TyVar` in the body
    /// becomes the matching `TypeVar`. Empty outside constructor-body interning.
    pub(super) ctor_param_map: FxHashMap<u32, BindingId>,
    /// Synthetic `TypeAlias` decls materialized for imported constructors,
    /// appended to `ThirFile::decls` so TLC and the evaluators resolve them like
    /// any local parametric alias.
    pub(super) synthetic_decls: Vec<ThirDeclId>,
    /// The import declaration currently being predeclared, if any. Imported
    /// parametric constructors are defined exactly once — during
    /// `predeclare_import_decls`, where this is `Some` — so the later re-interning
    /// when the `Import` expr node is lowered does not redefine them, and the
    /// materialized `TypeAlias` decl has a real HIR source to point at.
    pub(super) current_import_decl: Option<HirDeclId>,
}
