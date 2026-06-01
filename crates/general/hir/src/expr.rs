use text_size::TextRange;

use crate::arena::Idx;
use crate::pat::HirPatId;
use crate::symbol::SymbolId;
use crate::ty::{HirTypeId, LitVal};

// ── HirExpr identifiers ───────────────────────────────────────────────────────

pub type HirExprId = Idx<HirExpr>;

// ── HirArm ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HirArm {
    pub pat: HirPatId,
    /// Guard expression. Per spec, guards do NOT count toward exhaustiveness coverage.
    pub guard: Option<HirExprId>,
    pub body: HirExprId,
}

// ── BinaryOp ──────────────────────────────────────────────────────────────────

/// Binary operators — kept as a distinct HIR node rather than desugaring to
/// `Apply(Apply(builtin_op, lhs), rhs)`. Rationale: desugaring would require
/// pre-seeding the symbol table with ~15 overloaded built-ins (a v1 constraint
/// feature). M2 pattern-matches on `BinaryOp` and dispatches to primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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

impl BinaryOp {
    pub fn from_syntax(op: zutai_syntax::ast::operators::BinaryOp) -> Option<Self> {
        use zutai_syntax::ast::operators::BinaryOp as SOp;
        Some(match op {
            SOp::Add => Self::Add,
            SOp::Sub => Self::Sub,
            SOp::Mul => Self::Mul,
            SOp::Div => Self::Div,
            SOp::Eq => Self::Eq,
            SOp::Ne => Self::Ne,
            SOp::Lt => Self::Lt,
            SOp::Le => Self::Le,
            SOp::Gt => Self::Gt,
            SOp::Ge => Self::Ge,
            SOp::And => Self::And,
            SOp::Or => Self::Or,
            // `??` desugars to Match during lowering, not a BinOp.
            SOp::Coalesce => return None,
        })
    }
}

// ── ImportKind ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// `.zt` file — yields the file's final expression.
    Zt,
    /// `.zti` file — yields inert data.
    Zti,
}

// ── HirExpr ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub range: TextRange,
}

#[derive(Debug, Clone)]
pub enum HirExprKind {
    // ── Literals ──────────────────────────────────────────────────────────────
    Lit(LitVal),

    // ── Variable reference (name-resolved) ───────────────────────────────────
    Var(SymbolId),

    // ── Construction ─────────────────────────────────���───────────────────────
    Record {
        fields: Vec<(String, HirExprId)>,
    },
    /// Tagged variant construction: `(#tag, field = expr, ...)`.
    /// Kept distinct from Record; see plan §"Variant vs Record".
    Variant {
        tag: String,
        fields: Vec<(String, HirExprId)>,
    },
    List {
        items: Vec<HirExprId>,
    },

    // ── Functions ─────────────────────────────────────────────────────────────
    /// Lambda with one or more patterns. Multi-clause functions are lowered to
    /// a single Lambda with a nested Match (see plan §"Multi-clause lowering").
    Lambda {
        params: Vec<HirPatId>,
        body: HirExprId,
    },
    /// Curried application — one argument per node.
    Apply {
        fun: HirExprId,
        arg: HirExprId,
    },

    // ── Operators ─────────────────────────────────────────────────────────────
    /// Binary operator. `??` is NOT here — it desugars to `Match` during lowering.
    BinOp {
        op: BinaryOp,
        lhs: HirExprId,
        rhs: HirExprId,
    },

    // ── Bindings ──────────────────────────────────────────────────────────────
    /// Sequential local binding. `name` is in scope only in `body`, not `value`.
    /// Top-level mutual recursion is modelled at the `HirFile` level, not here.
    Let {
        name: SymbolId,
        ty: Option<HirTypeId>,
        value: HirExprId,
        body: HirExprId,
    },

    // ── Control ───────────────────────────────────────────────────────────────
    /// Kept distinct from `Match` for better diagnostic messages.
    If {
        cond: HirExprId,
        then_: HirExprId,
        else_: HirExprId,
    },
    Match {
        scrutinee: HirExprId,
        arms: Vec<HirArm>,
    },

    // ── Access ────────────────────────────────────────────────────────────────
    Field {
        value: HirExprId,
        label: String,
    },

    // ── Module boundary ───────────────────────────────────────────────────────
    Import {
        path: String,
        kind: ImportKind,
    },

    // ── Type annotation ───────────────────────────────────────────────────────
    Annot {
        expr: HirExprId,
        ty: HirTypeId,
    },

    // ── Error sentinel ───────────────────────────────────────────────────────
    Error,
}
