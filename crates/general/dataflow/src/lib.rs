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
use zutai_syntax::Span;
use zutai_syntax::posit::{PositLiteral, PositSpec};

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

/// Fallible form of [`lower_tlc`] that preserves the Phase 19 no-erasure gate.
pub fn try_lower_tlc(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
) -> Result<DataflowGraph, &'static str> {
    if let Some(reason) = zutai_tlc::residual_effect_reason(module) {
        return Err(reason);
    }
    Ok(lower::lower_tlc(module, hir_bindings))
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

/// Check well-formedness invariants 1-6 of a [`DataflowGraph`].
///
/// In debug builds the lowerer calls this automatically after lowering.
pub fn validate(graph: &DataflowGraph) -> Result<(), Vec<ValidationError>> {
    validate::validate(graph)
}
