//! MIR function, basic block, and instruction definitions.
//!
//! # Block-based CFG shape
//!
//! A [`MirFunc`] is a list of [`MirBlock`]s. Control enters at `blocks[0]`
//! and exits only through [`MirTerminator`]s. There are no implicit fall-
//! throughs — every block ends with an explicit terminator.
//!
//! Blocks are in ANF: every [`MirInstr`] binds exactly one [`MirVar`] to a
//! primitive operation whose operands are already-bound `MirVar`s or
//! constants. No nested calls.
//!
//! ```text
//! MirFunc
//!   params: [v0, v1]          // function arguments
//!   blocks:
//!     bb0:
//!       v2 = Add(v0, v1)
//!       v3 = Call(f, v2)
//!       Jump(bb1)
//!     bb1:
//!       Return(v3)
//! ```
//!
//! # Implementing lowering
//!
//! See `docs/plans/mir-lowering.md` for:
//! - ANF transformation algorithm (name-introduction order, let-flattening)
//! - Closure conversion (free-variable analysis, `MakeClosure` emission)
//! - Pattern-match compilation (Maranget algorithm → `Switch` terminators)
//! - Open questions: strict vs lazy, monomorphize vs box, curried vs uncurried

use crate::module::MirFuncId;

// ── MirVar ────────────────────────────────────────────────────────────────────

/// A local variable in MIR. Corresponds to one SSA value.
///
/// Variables are introduced by `MirFunc::params` and `MirInstr` bindings.
/// They are never reassigned (SSA discipline). The lowering pass is
/// responsible for generating fresh `MirVar`s for each let-binding it
/// introduces during ANF transformation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirVar(pub u32);

// ── MirBlockId ────────────────────────────────────────────────────────────────

/// Index into `MirFunc::blocks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirBlockId(pub u32);

// ── MirInstr ──────────────────────────────────────────────────────────────────

/// A single ANF let-binding instruction: `dest = <operation>`.
///
/// Every operand is a [`MirVar`] (already bound earlier in the same block or
/// in a dominating block's params). This invariant makes LLVM emission and
/// interpreter evaluation mechanical.
///
/// # TODO — fill in variants as lowering progresses
///
/// The variants below are a starter set. Add new ones as you encounter HIR
/// constructs that need MIR-level representation. Key additions needed:
///
/// - **Arithmetic / comparison** (`Add`, `Sub`, `Mul`, `Div`, `Eq`, `Lt`, …)
///   for built-in `BinOp` lowering.
/// - **Record construction / projection** (`MakeRecord`, `GetField`)
///   for record expressions and field access.
/// - **Variant construction / tag read** (`MakeVariant`, `GetTag`, `GetField`)
///   for variant expressions and pattern guards.
/// - **List construction** (`MakeList`, `ListHead`, `ListTail`) for list
///   literals and pattern matching.
/// - **Closure operations** (`MakeClosure`, `LoadCapture`) for closure
///   conversion — see `docs/plans/mir-lowering.md §Closure conversion`.
/// - **Thunk operations** (`MakeThunk`, `Force`) if lazy evaluation is chosen
///   — see `docs/plans/mir-lowering.md §Strict vs lazy`.
#[derive(Debug, Clone)]
pub enum MirInstr {
    /// `dest = src` — copy/move a variable. Used during ANF flattening.
    Copy { dest: MirVar, src: MirVar },

    /// `dest = <literal>` — introduce a compile-time constant.
    ///
    /// Replace `MirConst` with a richer type once you have a `MirTy`-indexed
    /// constant representation (Int, Float, Text, Bool, None, Atom).
    Const { dest: MirVar, val: MirConst },

    /// `dest = Call(func, arg)` — apply a function to one argument.
    ///
    /// Functions in Zutai are curried; a two-argument call `f a b` lowers to
    /// two consecutive `Call` instructions:
    /// ```text
    /// v1 = Call(f, a)
    /// v2 = Call(v1, b)
    /// ```
    Call {
        dest: MirVar,
        func: MirVar,
        arg: MirVar,
    },

    /// `dest = MakeClosure(func_id, env)` — capture a function + environment.
    ///
    /// `env` is a record-shaped `MirVar` holding all free variables.
    /// See `docs/plans/mir-lowering.md §Closure conversion`.
    MakeClosure {
        dest: MirVar,
        func: MirFuncId,
        env: MirVar,
    },

    /// `dest = LoadCapture(closure, index)` — extract a captured variable.
    ///
    /// Generated at the top of a closure body to reconstruct free variables
    /// from the environment parameter.
    LoadCapture {
        dest: MirVar,
        closure: MirVar,
        index: u32,
    },
    // ── Add more variants here as you implement lowering ──────────────────
}

// ── MirConst ──────────────────────────────────────────────────────────────────

/// A compile-time constant value embedded in [`MirInstr::Const`].
///
/// This is intentionally minimal. Expand as needed:
/// - Add `Atom(SmolStr)` when lowering atom literals.
/// - Decide on `Int` representation: `i64` (current) vs arbitrary-precision.
///   See `docs/plans/mir-lowering.md §Int representation`.
#[derive(Debug, Clone, PartialEq)]
pub enum MirConst {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

// ── MirTerminator ─────────────────────────────────────────────────────────────

/// Control-flow transfer at the end of a [`MirBlock`].
///
/// Every block ends with exactly one terminator. The terminator either exits
/// the function (`Return`) or transfers to another block (`Jump`, `Switch`).
///
/// # Implementing pattern-match compilation
///
/// HIR `Match { scrutinee, arms }` compiles to a tree of `Switch` terminators.
/// Each `Switch` tests one constructor tag (or literal value) of the
/// scrutinee and branches to different blocks for each case.
///
/// The Maranget algorithm (see plan doc §Pattern-match compilation) produces
/// this tree by column-major specialization of the pattern matrix.
///
/// Example: `match x { #ok => bb2; #err => bb3 }` →
/// ```text
/// Switch { scrutinee: x, arms: [(Tag("#ok"), bb2), (Tag("#err"), bb3)], default: bb_wildcard }
/// ```
#[derive(Debug, Clone)]
pub enum MirTerminator {
    /// Return a value from this function.
    Return(MirVar),

    /// Unconditional jump to another block, passing arguments to its params.
    ///
    /// Block params (φ-nodes) are used for values that differ across
    /// predecessors — e.g., the result of an if-then-else.
    Jump {
        target: MirBlockId,
        args: Vec<MirVar>,
    },

    /// Conditional dispatch on a scrutinee's shape.
    ///
    /// `arms` is a list of `(test, target_block)` pairs tried in order.
    /// `default` is taken if no arm matches (required for exhaustiveness;
    /// for exhaustive matches this can point to an `Unreachable` block).
    Switch {
        scrutinee: MirVar,
        arms: Vec<(MirTest, MirBlockId)>,
        default: MirBlockId,
    },

    /// Marker for unreachable code (exhaustive match, proven-never branch).
    /// LLVM maps this to `unreachable`. Interpreter should panic.
    Unreachable,
}

// ── MirTest ───────────────────────────────────────────────────────────────────

/// The condition tested by a [`MirTerminator::Switch`] arm.
///
/// Add constructors as you implement each pattern form in match compilation.
#[derive(Debug, Clone, PartialEq)]
pub enum MirTest {
    /// Test that the scrutinee is the given boolean literal.
    Bool(bool),
    /// Test that the scrutinee's integer value equals `n`.
    Int(i64),
    /// Test that the scrutinee's text value equals `s`.
    Text(String),
    /// Test that the scrutinee carries the given atom tag (e.g., `#ok`).
    Tag(String),
    /// Test that the scrutinee is `none`.
    IsNone,
    /// Test that the scrutinee is a `some` value (for optional patterns).
    IsSome,
}

// ── MirBlock ──────────────────────────────────────────────────────────────────

/// A basic block: a sequence of [`MirInstr`]s followed by a [`MirTerminator`].
///
/// Blocks may declare [`params`] — these act as φ-nodes (SSA join points).
/// A `Jump { target, args }` passes values to `target.params[i]`.
///
/// # Interpreter note
/// When entering a block via `Jump { args }`, bind `params[i] = args[i]`
/// before executing instructions.
pub struct MirBlock {
    /// φ-node parameters (values that differ across predecessors).
    pub params: Vec<MirVar>,
    /// ANF instructions in execution order.
    pub instrs: Vec<MirInstr>,
    /// Control-flow transfer at the end.
    pub terminator: MirTerminator,
}

// ── MirFunc ───────────────────────────────────────────────────────────────────

/// A MIR function: a CFG of [`MirBlock`]s with explicit parameters.
///
/// Functions are always in **curried** form: a two-argument Zutai function
/// lowers to a MIR function that takes one argument and returns a closure
/// (another `MirFunc`) that takes the second.
///
/// Alternatively (open question): lower to n-ary MIR functions and apply
/// uncurrying. See `docs/plans/mir-lowering.md §Curried vs uncurried`.
///
/// Entry block is always `blocks[0]`. The entry block's `params` are the
/// function's formal parameters.
pub struct MirFunc {
    /// Debug name (from source, or synthetic for closures/thunks).
    pub name: String,
    /// Entry block params serve as the function's formal parameters.
    /// For a curried function this is exactly one `MirVar`.
    pub params: Vec<MirVar>,
    /// All basic blocks. Block 0 is the entry.
    pub blocks: Vec<MirBlock>,
    /// Counter for generating fresh `MirVar` IDs during lowering.
    /// (Used by the lowering pass; irrelevant to consumers.)
    pub next_var: u32,
}

impl MirFunc {
    /// Allocate a fresh [`MirVar`] (used during lowering).
    pub fn fresh_var(&mut self) -> MirVar {
        let v = MirVar(self.next_var);
        self.next_var += 1;
        v
    }

    /// Allocate a fresh [`MirBlockId`] and push an empty placeholder block.
    ///
    /// Fill in `instrs` and `terminator` after creating all blocks so that
    /// forward references work.
    pub fn alloc_block(&mut self) -> MirBlockId {
        let id = MirBlockId(self.blocks.len() as u32);
        self.blocks.push(MirBlock {
            params: vec![],
            instrs: vec![],
            terminator: MirTerminator::Unreachable,
        });
        id
    }
}
