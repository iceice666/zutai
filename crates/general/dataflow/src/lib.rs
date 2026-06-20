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

// ── Literal ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DfLit {
    Bool(bool),
    Int(i64),
    Float(f64),
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
    Tuple(Vec<DfTupleNodeItem>),
    List(Vec<NodeId>),
    Variant(String, NodeId),

    // ── Data elimination ──
    Select {
        base: NodeId,
        field: String,
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
    Record(Vec<(String, DfPattern)>),
    Variant(String, Box<DfPattern>),
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
    Union(Vec<DfTyId>),
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

/// Lower a fully-elaborated TLC module into a Dataflow Core graph.
///
/// `hir_bindings` is `hir_file.bindings` — the flat binding table indexed by
/// `BindingId.0` — used to resolve `BindingId`s to their string names for
/// `GlobalRef` nodes.
pub fn lower_tlc(
    module: &zutai_tlc::TlcModule,
    hir_bindings: &[zutai_hir::Binding],
) -> DataflowGraph {
    lower::lower_tlc(module, hir_bindings)
}

/// Validation errors produced by [`validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// `spans.len() != nodes.len()` (invariant 6).
    SpanTableSizeMismatch { spans: usize, nodes: usize },
    /// A `Bind` node is owned by 0 or >1 Lambdas/arm-patterns (invariant 2).
    BindOwnershipViolation { count: usize },
    /// A `GlobalRef` names a symbol not present in `globals` (invariant 5).
    StrayGlobalRef { name: String },
}

/// Check a subset of the well-formedness invariants of a [`DataflowGraph`].
///
/// Currently checks invariants 2, 5, and 6 from `docs/dataflow-core.md`:
/// - Bind ownership (every Bind node owned by exactly one Lambda or arm pattern).
/// - No stray GlobalRefs (every GlobalRef name is in `globals`).
/// - Span table size (`spans.len() == nodes.len()`).
///
/// Invariants 1 (type consistency), 3 (arm-bind scope), and 4 (lambda capture)
/// require full graph traversal and are not yet implemented.
///
/// In debug builds the lowerer calls this automatically after lowering.
pub fn validate(graph: &DataflowGraph) -> Result<(), Vec<ValidationError>> {
    validate::validate(graph)
}
