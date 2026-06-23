//! Dataflow Core IR and TLC→DC lowering for Zutai.
//!
//! Dataflow Core sits between TLC (Type Lambda Calculus) and ANF in the
//! compilation pipeline. It is a directed graph where:
//! - Local bindings are lowered exactly once; all uses share a single NodeId.
//! - Laziness is topological: unreachable nodes are never emitted to ANF.
//! - Recursion is structural: back-edges via `GlobalRef` create cycles that
//!   the ANF phase resolves into `letrec` bindings via SCC analysis.

use indexmap::IndexMap;
use la_arena::{Arena, Idx};
use rustc_hash::FxHashMap;
use zutai_hir::HirImportSource;
use zutai_syntax::Span;
use zutai_syntax::posit::{PositLiteral, PositSpec};

pub use zutai_tlc::HostOp;

mod lower;
mod validate;

#[cfg(test)]
mod tests;

// ── Type variables ────────────────────────────────────────────────────────────

/// DC type variable — carries the BindingId.0 of the source type parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DfTyVar {
    Named(u32),
    Inferred(u32),
}

// ── Arena IDs ─────────────────────────────────────────────────────────────────

pub type NodeId = Idx<DfNode>;
pub type DfTyId = Idx<DfTy>;
pub type DfTypes = Arena<DfTy>;

// ── Literal ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DfLit {
    Bool(bool),
    Int(i64),
    Float(f64),
    Posit(PositLiteral),
    Text(String),
    Atom(String),
}

// ── Import kind ───────────────────────────────────────────────────────────────

/// Import source kind for `DfNodeKind::Import` nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Zti,
    Zt,
}

// ── Resolved imports ──────────────────────────────────────────────────────────

/// Imports resolved by the front end and available to native lowering.
///
/// Keyed by the import source (`ImportKey == HirImportSource`), matching how the
/// reference interpreter resolves imports. `.zti` data values are lowered inline
/// to Dataflow Core constants; `.zt` module imports are not yet supported by the
/// native backend (Phase A.a) and are gated out before lowering.
#[derive(Default)]
pub struct ImportEnv<'a> {
    /// `.zti` import values keyed by import source.
    pub zti: FxHashMap<HirImportSource, &'a zutai_types::Value>,
}

/// True when an import source designates a `.zti` data file.
fn is_zti_import(source: &HirImportSource) -> bool {
    matches!(source, HirImportSource::String(path) if path.ends_with(".zti"))
}

// ── Builtin binary ops ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DfPositOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DfBuiltinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Posit { op: DfPositOp, spec: PositSpec },
}

// ── Node kinds ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DfNodeKind {
    // ── Leaves ──
    Lit(DfLit),
    /// A binding site for a lambda parameter or a match-arm pattern variable.
    /// Owned by exactly one Lambda (as `param`) or one DfArm (via Bind pattern).
    Bind,
    GlobalRef(String),
    Import {
        path: String,
        kind: ImportKind,
    },
    Error,

    // ── Abstraction / application ──
    Lambda {
        param: NodeId,
        body: NodeId,
    },
    Apply {
        func: NodeId,
        arg: NodeId,
    },
    /// Runtime host `io.print` dispatch. Evaluates `arg`, prints the resulting
    /// Text through the runtime ABI, and returns that same Text value.
    HostPrint {
        arg: NodeId,
    },
    /// Runtime host operation authorized by an explicit v2 capability.
    HostOp {
        op: HostOp,
        arg: NodeId,
    },

    // ── Type polymorphism ──
    TyLam {
        ty_params: Vec<DfTyVar>,
        body: NodeId,
    },
    TyApp {
        poly: NodeId,
        ty_args: Vec<DfTyId>,
    },

    // ── Data construction ──
    Record(Vec<(String, NodeId)>),
    RecordUpdate {
        base: NodeId,
        updates: Vec<(String, usize, NodeId)>,
    },
    Tuple(Vec<DfTupleNodeItem>),
    List(Vec<NodeId>),
    Variant {
        tag: String,
        tag_index: usize,
        value: NodeId,
    },
    // ── Data elimination ──
    Select {
        base: NodeId,
        field: String,
        slot: usize,
    },
    Match {
        scrutinee: NodeId,
        arms: Vec<DfArm>,
    },
    Coalesce {
        value: NodeId,
        fallback: NodeId,
    },

    // ── Primitive binary operations ──
    Builtin(DfBuiltinOp, NodeId, NodeId),
    /// Explicit left-to-right runtime sequence. Every item is lowered in order;
    /// the sequence result is the last item, or Error for an empty sequence.
    Sequence(Vec<NodeId>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum DfTupleNodeItem {
    Named { name: String, value: NodeId },
    Positional(NodeId),
}

// ── Match arms ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct DfArm {
    pub pattern: DfPattern,
    pub guard: Option<NodeId>,
    pub body: NodeId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DfPattern {
    Wildcard,
    Lit(DfLit),
    Atom(String),
    /// `Bind(n)` — `n` must be a Bind node owned by this arm.
    Bind(NodeId),
    Tuple(Vec<DfTuplePatItem>),
    Record(Vec<(String, usize, DfPattern)>),
    Variant {
        tag: String,
        tag_index: usize,
        pattern: Box<DfPattern>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DfTuplePatItem {
    Named { name: String, pattern: DfPattern },
    Positional(DfPattern),
}

// ── Node ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DfNode {
    pub ty: DfTyId,
    pub kind: DfNodeKind,
}

// ── Type representation ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DfTy {
    // Primitives
    Int,
    Float,
    Posit(PositSpec),
    Bool,
    Text,
    Opaque(String),
    Atom,
    True,
    False,

    // Composite
    List(DfTyId),
    Optional(DfTyId),
    Maybe(DfTyId),
    Record(Vec<DfRecordField>),
    Union(Vec<DfUnionVariant>),
    Tuple(Vec<DfTupleField>),
    Fun(DfTyId, DfTyId),

    // Polymorphism
    TyVar(DfTyVar),
    TyFun(Vec<DfTyVar>, DfTyId),
    TyApp(DfTyId, Vec<DfTyId>),

    // Meta
    Type,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DfRecordField {
    pub name: String,
    pub optional: bool,
    pub ty: DfTyId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DfUnionVariant {
    pub tag: String,
    pub ty: DfTyId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DfTupleField {
    Named { name: String, ty: DfTyId },
    Positional(DfTyId),
}

// ── Graph ─────────────────────────────────────────────────────────────────────

/// The Dataflow Core graph for one module.
///
/// `globals` maps each top-level declared name to its root NodeId.
/// `root` is the NodeId of the module's final expression.
/// `spans` is parallel to `nodes`: `spans[raw_node_idx]` is the source
/// location for the node at that index (if available).
#[derive(Debug)]
pub struct DataflowGraph {
    pub nodes: Arena<DfNode>,
    pub types: Arena<DfTy>,
    pub globals: IndexMap<String, NodeId>,
    pub root: NodeId,
    pub spans: Vec<Option<Span>>,
}

// ── Public entry points ───────────────────────────────────────────────────────

/// Lower a fully-elaborated, pure TLC module into a Dataflow Core graph.
///
/// `hir_bindings` is `hir_file.bindings` — the flat binding table indexed by
/// `BindingId.0` — used to resolve `BindingId`s to their string names for
/// `GlobalRef` nodes.
///
/// Panics when residual effect syntax or non-empty function effect rows would be
/// erased. Use [`try_lower_tlc`] when the caller wants a diagnostic instead.
pub fn lower_tlc(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
) -> DataflowGraph {
    try_lower_tlc(module, hir_bindings)
        .expect("residual TLC effects must be handled before Dataflow Core")
}

/// Lower a TLC module under an explicit v2 host grant set.
pub fn lower_tlc_with_host_grants(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
    grants: zutai_tlc::HostEffectSet,
) -> DataflowGraph {
    try_lower_tlc_with_host_grants(module, hir_bindings, grants)
        .expect("ungranted residual TLC effects must not enter Dataflow Core")
}

/// Returns a static reason string when `module` contains a value-record
/// `GetField` on an open-row type — a case the slot-based Dataflow backend
/// cannot lower soundly.
///
/// An open record type `{ f : T; …rest }` used as a function parameter hides
/// the tail fields from the compiled body. The runtime slot of `f` depends on
/// ALL the concrete record's fields, but the compiled code sees only the view
/// type. The interpreter resolves fields by name; the native backend cannot.
///
/// Returns `None` when the module is safe to lower to Dataflow Core.
fn open_row_select_reason(module: &zutai_tlc::TlcModule) -> Option<&'static str> {
    use zutai_tlc::{Row, TlcExpr, TlcType};

    fn row_is_open(row: &Row) -> bool {
        match row {
            Row::REmpty => false,
            Row::RVar(_) => true,
            Row::RExtend { tail, .. } => row_is_open(tail),
        }
    }

    for (id, expr) in module.expr_arena.iter() {
        let TlcExpr::GetField(base, _) = expr else {
            continue;
        };
        // Dict-method GetField slots are pre-computed by the TLC witness pass and
        // remain correct at runtime — only value-record selects can miscompile.
        if module.dict_field_slots.contains_key(&id) {
            continue;
        }
        let Some(&ty_id) = module.expr_types.get(base) else {
            continue;
        };
        if let TlcType::Record(row) = &module.type_arena[ty_id]
            && row_is_open(row)
        {
            return Some(
                "native backend cannot select a field from an open record row: \
                    the field's runtime slot depends on hidden tail fields that are unknown \
                    inside the compiled function. Use `zutai run` (interpreter) or restrict \
                    the parameter to a closed record type",
            );
        }
    }
    None
}

/// Returns a static reason when `module` contains a module import that native
/// lowering cannot yet resolve. `.zti` data imports present in `imports` lower
/// inline to Dataflow Core constants (Phase A.a); `.zt` module imports are not
/// yet linked into the compiled binary, so they are gated here and the
/// interpreter resolves them instead.
///
/// Returns `None` when every import is resolvable.
fn unresolved_import_reason(
    module: &zutai_tlc::TlcModule,
    imports: &ImportEnv,
) -> Option<&'static str> {
    use zutai_tlc::TlcExpr;
    module
        .expr_arena
        .iter()
        .any(|(_, expr)| match expr {
            TlcExpr::Import(source) => !(is_zti_import(source) && imports.zti.contains_key(source)),
            _ => false,
        })
        .then_some(
            "native backend does not support `.zt` module imports yet: imported modules are \
            not linked into the compiled binary. Use `zutai run` (interpreter)",
        )
}

/// Shared lowering gate: residual-effect (caller-computed), open-row select, and
/// unresolved-import checks, then lower.
fn try_lower_inner(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
    imports: &ImportEnv,
    effect_reason: Option<&'static str>,
) -> Result<DataflowGraph, &'static str> {
    if let Some(reason) = effect_reason {
        return Err(reason);
    }
    if let Some(reason) = open_row_select_reason(module) {
        return Err(reason);
    }
    if let Some(reason) = unresolved_import_reason(module, imports) {
        return Err(reason);
    }
    Ok(lower::lower_tlc(module, hir_bindings, imports))
}

/// Fallible form of [`lower_tlc`] that preserves the Phase 19 no-erasure gate.
pub fn try_lower_tlc(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
) -> Result<DataflowGraph, &'static str> {
    try_lower_inner(
        module,
        hir_bindings,
        &ImportEnv::default(),
        zutai_tlc::residual_effect_reason(module),
    )
}

/// Fallible lowering under an explicit v2 host grant set.
pub fn try_lower_tlc_with_host_grants(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
    grants: zutai_tlc::HostEffectSet,
) -> Result<DataflowGraph, &'static str> {
    try_lower_inner(
        module,
        hir_bindings,
        &ImportEnv::default(),
        zutai_tlc::residual_effect_reason_with_grants(module, grants),
    )
}

/// Fallible lowering under a host grant set with resolved imports. `.zti` data
/// imports in `imports` lower inline; `.zt` imports are still gated (Phase A.a).
pub fn try_lower_tlc_with_host_grants_and_imports(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
    grants: zutai_tlc::HostEffectSet,
    imports: &ImportEnv,
) -> Result<DataflowGraph, &'static str> {
    try_lower_inner(
        module,
        hir_bindings,
        imports,
        zutai_tlc::residual_effect_reason_with_grants(module, grants),
    )
}

/// Validation errors produced by [`validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// `spans.len() != nodes.len()` (invariant 6).
    SpanTableSizeMismatch {
        spans: usize,
        nodes: usize,
    },
    /// A `Bind` node is owned by 0 or >1 Lambdas/arm-patterns (invariant 2).
    BindOwnershipViolation {
        count: usize,
    },
    /// A `GlobalRef` names a symbol not present in `globals` (invariant 5).
    StrayGlobalRef {
        name: String,
    },
    InvalidRootNode {
        target: NodeId,
    },
    InvalidNodeRef {
        owner: NodeId,
        field: &'static str,
        target: NodeId,
    },
    InvalidTypeRef {
        owner: DfTyId,
        field: &'static str,
        target: DfTyId,
    },
    InvalidNodeType {
        node: NodeId,
        ty: DfTyId,
    },
    UnexpectedNodeKind {
        owner: NodeId,
        field: &'static str,
        target: NodeId,
        expected: &'static str,
    },
    UnexpectedTypeKind {
        owner: NodeId,
        field: &'static str,
        expected: &'static str,
        actual: DfTyId,
    },
    TypeMismatch {
        owner: NodeId,
        field: &'static str,
        expected: DfTyId,
        actual: DfTyId,
    },
    MissingRequiredField {
        owner: NodeId,
        field: String,
    },
    ArmBindScopeViolation {
        bind: NodeId,
        match_node: NodeId,
        arm_index: usize,
        use_site: NodeId,
    },
    LambdaCaptureViolation {
        bind: NodeId,
        owner_lambda: NodeId,
        use_site: NodeId,
    },
}

/// Check the cheap O(n) structural well-formedness invariants of a [`DataflowGraph`]:
/// span/root validity, per-node type and reference bounds, type-shape compatibility,
/// bind ownership, and stray `GlobalRef`s.
///
/// This subset runs unconditionally in every build — including release — because structural
/// corruption (dangling refs, type-shape mismatches) would otherwise silently miscompile in
/// ANF→SSA→codegen.
///
/// Scope-walk invariants (arm-bind scope and lambda capture) are checked only by the
/// full [`validate`].
pub fn validate_structural(graph: &DataflowGraph) -> Result<(), Vec<ValidationError>> {
    validate::validate_structural(graph)
}

/// Check well-formedness invariants 1-6 of a [`DataflowGraph`].
///
/// In debug builds the lowerer calls this automatically after lowering. In every build
/// the lowerer calls [`validate_structural`] for the cheaper structural subset.
pub fn validate(graph: &DataflowGraph) -> Result<(), Vec<ValidationError>> {
    validate::validate(graph)
}
