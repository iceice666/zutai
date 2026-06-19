//! Static Single-Assignment (SSA) IR and ANF→SSA lowering for Zutai.
//!
//! SSA sits between ANF and LLVM IR in the compilation pipeline.
//! Every value is assigned exactly once. Control flow is organized into
//! basic blocks with explicit terminators. Phi nodes merge values at join
//! points (e.g. after match arms or conditional branches).

pub use zutai_anf::{DfBuiltinOp, DfLit, DfTyId, DfTyVar};

mod lower;

#[cfg(test)]
mod tests;

// ── Values ────────────────────────────────────────────────────────────────────

/// SSA value — a register name or literal.
#[derive(Debug, Clone, PartialEq)]
pub enum SsaValue {
    /// A named register (from an ANF binding or phi node).
    Reg(String),
    /// A literal constant.
    Lit(DfLit),
    /// A global function name.
    Global(String),
}

// ── Instructions ───────────────────────────────────────────────────────────────

/// SSA instruction — a named computation that writes its result to `dest`.
#[derive(Debug, Clone, PartialEq)]
pub struct SsaInstr {
    /// The register that receives the result.
    pub dest: String,
    /// The operation producing the value.
    pub op: SsaOp,
}

/// SSA operations.
#[derive(Debug, Clone, PartialEq)]
pub enum SsaOp {
    /// Function call: dest = func(arg).
    Call { func: SsaValue, arg: SsaValue },
    /// Type application (erased in v0 — just returns the polymorphic value).
    TyApp {
        poly: SsaValue,
        ty_args: Vec<DfTyId>,
    },
    /// Record construction: dest = { field1 = v1, field2 = v2, ... }.
    Record { fields: Vec<(String, SsaValue)> },
    /// Tuple construction.
    Tuple { items: Vec<SsaTupleItem> },
    /// List construction.
    List { elems: Vec<SsaValue> },
    /// Record field selection: dest = base.field.
    Select { base: SsaValue, field: String },
    /// Variant construction: dest = tag(value).
    Variant { tag: String, value: SsaValue },
    /// Binary builtin operation.
    Builtin {
        op: DfBuiltinOp,
        lhs: SsaValue,
        rhs: SsaValue,
    },
    /// Optional coalesce: dest = value ?? fallback.
    Coalesce { value: SsaValue, fallback: SsaValue },
    /// Error sentinel.
    Error,
    /// Phi node: selects a value based on which predecessor block transferred control.
    /// `branches` maps predecessor block labels to the value selected from that path.
    Phi { branches: Vec<(String, SsaValue)> },
    /// Match discriminator: pattern-match on scrutinee.
    /// Each arm tests the pattern; if it matches, bind pattern variables and
    /// jump to the arm's block. This is a high-level instruction that will be
    /// lowered in codegen.
    MatchDiscriminant { scrutinee: SsaValue },
}

/// A tuple item in SSA form.
#[derive(Debug, Clone, PartialEq)]
pub enum SsaTupleItem {
    Named { name: String, value: SsaValue },
    Positional(SsaValue),
}

// ── Terminators ─────────────────────────────────────────────────────────────────

/// Block terminator — how control leaves a basic block.
#[derive(Debug, Clone, PartialEq)]
pub enum SsaTerminator {
    /// Return a value from the current function.
    Return(SsaValue),
    /// Unconditional jump to a label.
    Jump(String),
    /// Conditional branch.
    Branch {
        cond: SsaValue,
        then_label: String,
        else_label: String,
    },
}

// ── Blocks ──────────────────────────────────────────────────────────────────────

/// A basic block: label, instructions, terminator.
#[derive(Debug, Clone, PartialEq)]
pub struct SsaBlock {
    pub label: String,
    pub instructions: Vec<SsaInstr>,
    pub terminator: SsaTerminator,
}

// ── Functions ───────────────────────────────────────────────────────────────────

/// An SSA function.
#[derive(Debug, Clone, PartialEq)]
pub struct SsaFunc {
    /// Function name (for top-level decls, this is the global name;
    /// for lambdas, a generated name like `__lambda_0`).
    pub name: String,
    /// Parameter names.
    pub params: Vec<String>,
    /// Basic blocks. The first block is the entry block.
    pub blocks: Vec<SsaBlock>,
}

// ── Declarations ────────────────────────────────────────────────────────────────

/// A top-level declaration in SSA form.
#[derive(Debug, Clone, PartialEq)]
pub enum SsaDecl {
    /// Non-recursive declaration.
    Func(SsaFunc),
    /// Mutually recursive declarations.
    RecGroup(Vec<SsaFunc>),
}

// ── Module ──────────────────────────────────────────────────────────────────────

/// An SSA module — the compilation unit.
#[derive(Debug, Clone, PartialEq)]
pub struct SsaModule {
    pub decls: Vec<SsaDecl>,
    /// The module's entry-point function (evaluates the root expression).
    pub entry: SsaFunc,
}

// ── Public entry point ──────────────────────────────────────────────────────────

/// Lower an ANF module into SSA form.
pub fn lower_anf(module: &zutai_anf::AnfModule) -> SsaModule {
    lower::lower_anf(module)
}
