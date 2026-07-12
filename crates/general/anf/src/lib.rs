//! Administrative Normal Form (ANF) IR and DC→ANF lowering for Zutai.
//!
//! ANF sits between Dataflow Core and SSA in the compilation pipeline.
//! Every sub-expression is named by a let-binding, and every argument
//! to an application or primitive is an atom (variable, literal, or global).
//!
//! See `docs/compiler/anf.md` for the design specification.

pub use zutai_dataflow::{
    DataflowGraph, DfBuiltinOp, DfListPrimOp, DfLit, DfNumPrimOp, DfPositOp, DfTextPrimOp,
    DfTupleField, DfTy, DfTyId, DfTyVar, DfTypes, HostOp,
};

mod lower;
mod scc;

#[cfg(test)]
mod tests;

// ── Atoms ─────────────────────────────────────────────────────────────────────

/// A trivially-valued expression — can be used directly without introducing
/// a binding.
#[derive(Debug, Clone, PartialEq)]
pub enum AnfAtom {
    /// Reference to a local variable or lambda parameter.
    Var(String),
    /// Literal constant.
    Lit(DfLit),
    /// Reference to a top-level global.
    Global(String),
}

// ── Body ──────────────────────────────────────────────────────────────────────

/// A linear sequence of let-bindings followed by a result atom.
/// This is the body of a lambda, the RHS of a top-level decl, or a match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct AnfBody {
    /// The let-bindings in order; each `Var` in a subsequent binding or the
    /// result must appear as a preceding LHS here, or in an enclosing scope.
    pub bindings: Vec<(String, AnfExpr)>,
    /// The final value of this body.
    pub result: AnfAtom,
}

// ── Expressions ───────────────────────────────────────────────────────────────

/// The RHS of a let-binding in an `AnfBody`.
/// All arguments to complex forms must be `AnfAtom`s — this is the ANF invariant.
#[derive(Debug, Clone, PartialEq)]
pub enum AnfExpr {
    /// Trivial: just an atom.
    Atom(AnfAtom),
    /// Curried function application. Both operands must be atoms.
    Apply { func: AnfAtom, arg: AnfAtom },
    /// Runtime `io.print` dispatch. Prints the Text atom and returns it.
    HostPrint { value: AnfAtom },
    /// Runtime host operation authorized by an explicit host capability.
    HostOp { op: HostOp, value: AnfAtom },
    /// Type application at a polymorphic call site.
    TyApp { poly: AnfAtom, ty_args: Vec<DfTyId> },
    /// Lambda abstraction. `param` is the parameter name (from a DC Bind node).
    Lambda { param: String, body: AnfBody },
    /// Type-level lambda (wraps a polymorphic function).
    TyLam {
        ty_params: Vec<DfTyVar>,
        body: AnfBody,
    },
    /// Record literal; values are ordered by canonical field slot.
    Record(Vec<AnfAtom>),
    /// Record update; base and update values are atoms.
    RecordUpdate {
        base: AnfAtom,
        updates: Vec<(usize, AnfAtom)>,
    },
    /// Tuple literal.
    Tuple(Vec<AnfTupleItem>),
    /// List literal; all elements are atoms.
    List(Vec<AnfAtom>),
    /// Record field projection by canonical slot.
    Select { base: AnfAtom, slot: usize },
    /// Pattern match (scrutinee must be an atom).
    Match {
        scrutinee: AnfAtom,
        arms: Vec<AnfArm>,
    },
    /// Optional coalesce: `value ?? fallback`.
    Coalesce { value: AnfAtom, fallback: AnfAtom },
    /// Built-in binary operation.
    Builtin {
        op: DfBuiltinOp,
        lhs: AnfAtom,
        rhs: AnfAtom,
    },
    /// Type-directed structural equality/inequality for heap-shaped values.
    ValueEq {
        negated: bool,
        lhs: AnfAtom,
        rhs: AnfAtom,
        ty: DfTyId,
    },
    /// Scalar list-bridge primitive; all operands are atoms.
    ListPrim {
        op: DfListPrimOp,
        args: Vec<AnfAtom>,
    },
    /// Scalar numeric bridge primitive; all operands are atoms.
    NumPrim { op: DfNumPrimOp, args: Vec<AnfAtom> },
    /// Scalar text bridge primitive; all operands are atoms.
    TextPrim {
        op: DfTextPrimOp,
        args: Vec<AnfAtom>,
    },
    /// Variant construction: `tag(value)` with the dense per-union tag index.
    Variant {
        tag: String,
        tag_index: usize,
        value: AnfAtom,
    },
    /// Error sentinel — propagated from DC error nodes.
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnfTupleItem {
    Named { name: String, value: AnfAtom },
    Positional(AnfAtom),
}

// ── Match arms ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct AnfArm {
    pub pattern: AnfPattern,
    /// Guard condition (tested after the pattern matches); `None` = always matches.
    pub guard: Option<AnfBody>,
    pub body: AnfBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnfPattern {
    Wildcard,
    Lit(DfLit),
    /// Atom tag match (e.g. `#ok`).
    Atom(String),
    /// Bind: introduce a name for the matched value.
    Bind(String),
    Tuple(Vec<AnfTuplePatItem>),
    ListNil,
    ListCons {
        head: Box<AnfPattern>,
        tail: Box<AnfPattern>,
    },
    Record(Vec<(usize, AnfPattern)>),
    Variant {
        tag: String,
        tag_index: usize,
        pattern: Box<AnfPattern>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnfTuplePatItem {
    Named { name: String, pattern: AnfPattern },
    Positional(AnfPattern),
}

// ── Top-level declarations ────────────────────────────────────────────────────

/// A top-level ANF declaration, in topological dependency order.
#[derive(Debug, Clone, PartialEq)]
pub enum AnfDecl {
    /// Non-recursive: `let name = body`.
    Let { name: String, body: AnfBody },
    /// Recursive or mutually-recursive: `letrec { name₁ = body₁; ... }`.
    Letrec { bindings: Vec<(String, AnfBody)> },
}

// ── Module ────────────────────────────────────────────────────────────────────

/// The ANF representation of a complete Zutai module.
///
/// `decls` are top-level bindings in forward topological order (each binding
/// may reference only bindings that appear earlier in the list, modulo
/// `Letrec` groups).  `root` is the module's output expression.
#[derive(Debug, Clone, PartialEq)]
pub struct AnfModule {
    pub decls: Vec<AnfDecl>,
    pub root: AnfBody,
    pub root_ty: DfTy,
    pub root_ty_id: DfTyId,
    pub types: DfTypes,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Lower a Dataflow Core graph into ANF.
pub fn lower_dc(graph: &DataflowGraph) -> AnfModule {
    lower::lower_dc(graph)
}
