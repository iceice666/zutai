//! Static Single-Assignment (SSA) IR and ANF→SSA lowering for Zutai.
//!
//! SSA sits between ANF and LLVM IR in the compilation pipeline.
//! Every value is assigned exactly once. Control flow is organized into
//! basic blocks with explicit terminators. Phi nodes merge values at join
//! points (e.g. after match arms or conditional branches).

pub use zutai_anf::{
    DfBuiltinOp, DfListPrimOp, DfLit, DfPositOp, DfTupleField, DfTy, DfTyId, DfTyVar, DfTypes,
    HostOp,
};

mod lower;
mod tco;
mod uncurry;

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
    /// A top-level non-function value/thunk symbol.
    Global(String),
    /// A top-level function value represented by a static closure object.
    GlobalClosure(String),
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
    /// Function application through a D-0003 closure object: loads the code
    /// slot from `closure` and calls it as `i64 fn(i64 self, i64 arg)`.
    /// `tail` marks a verified tail call (last instruction whose result the
    /// block returns), which codegen emits as an LLVM `musttail call` so deep
    /// tail recursion runs in constant stack space.
    ApplyClosure {
        closure: SsaValue,
        arg: SsaValue,
        tail: bool,
    },
    /// Direct call to a known top-level worker by name with all arguments at
    /// once: `[musttail] call i64 @func(args…)`. Emitted by the uncurrying pass
    /// when a saturated curried call to a known function collapses to one direct
    /// multi-argument call, eliminating the per-application closure and arg-tuple
    /// allocations. `tail` marks a verified tail call for `musttail`.
    CallKnown {
        func: String,
        args: Vec<SsaValue>,
        tail: bool,
    },
    /// Runtime host `io.print`: print Text, append newline, return the same Text.
    HostPrint { value: SsaValue },
    /// Runtime host operation authorized by an explicit v2 capability.
    HostOp { op: HostOp, value: SsaValue },
    /// Allocate a closure object for a lambda value: `{ header, code, caps[] }`.
    MakeClosure {
        code: String,
        captures: Vec<SsaValue>,
    },
    /// Load capture `index` from the enclosing closure (slot `2 + index`).
    LoadCapture { closure: SsaValue, index: usize },
    /// Force a top-level non-function thunk.
    CallGlobal { name: String },
    /// Type application (erased in v0 — just returns the polymorphic value).
    TyApp {
        poly: SsaValue,
        ty_args: Vec<DfTyId>,
    },
    /// Record construction; fields are ordered by canonical slot.
    Record { fields: Vec<SsaValue> },
    /// Record update by canonical slot.
    RecordUpdate {
        base: SsaValue,
        updates: Vec<(usize, SsaValue)>,
    },
    /// Tuple construction.
    Tuple { items: Vec<SsaTupleItem> },
    /// List construction.
    List { elems: Vec<SsaValue> },
    /// Record field selection by canonical slot: dest = base[slot].
    Select { base: SsaValue, slot: usize },
    /// Variant construction with a dense per-union tag index.
    Variant {
        tag: String,
        tag_index: usize,
        value: SsaValue,
    },
    /// Variant payload extraction.
    VariantValue { scrutinee: SsaValue },
    /// Binary builtin operation.
    Builtin {
        op: DfBuiltinOp,
        lhs: SsaValue,
        rhs: SsaValue,
    },
    /// Scalar list-bridge primitive: a single runtime `zutai.list_*` call.
    ListPrim {
        op: DfListPrimOp,
        args: Vec<SsaValue>,
    },
    /// Optional coalesce: dest = value ?? fallback.
    Coalesce { value: SsaValue, fallback: SsaValue },
    /// Error sentinel.
    Error,
    /// Plain value alias. Kept as an instruction so pattern bindings and ANF
    /// atom lets can introduce the destination register without abusing LLVM
    /// phi nodes in blocks with no matching predecessor edge.
    Alias { value: SsaValue },
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
    pub entry_ty: DfTy,
    pub entry_ty_id: DfTyId,
    pub types: DfTypes,
    /// Top-level function names that receive static empty-capture closure
    /// objects, in declaration order. Drives `@zutai.closure.<name>` emission.
    pub closure_exports: Vec<String>,
}

// ── Public entry point ──────────────────────────────────────────────────────────

/// Lower an ANF module into SSA form, collapse saturated known calls into direct
/// multi-argument worker calls (uncurrying), then run tail-call optimization so
/// deep tail recursion compiles to `musttail` calls in constant stack space with
/// no per-call closure/arg-tuple churn.
pub fn lower_anf(anf: &zutai_anf::AnfModule) -> SsaModule {
    let (mut module, fresh) = lower::lower_anf(anf);
    uncurry::uncurry(&mut module, anf, fresh);
    uncurry::scalar_replace_tuples(&mut module);
    tco::optimize_tail_calls(&mut module);
    module
}
