//! Mid-level intermediate representation (MIR) for Zutai general mode (`.zt`).
//!
//! # Pipeline position
//!
//! ```text
//! CST (zutai-syntax)
//!   ↓  lower_file
//! HIR (zutai-hir)        ← name-resolved, desugared, near-surface
//!   ↓  lower_module      ← YOU ARE HERE (to be implemented)
//! MIR (zutai-mir)        ← ANF, explicit closures, compiled patterns, typed
//!   ↓  emit_llvm / eval  ← LLVM IR or bytecode interpreter
//! ```
//!
//! # Shape: block-based CFG in ANF
//!
//! MIR is a **Control-Flow Graph (CFG)** of basic blocks. Each block contains
//! a sequence of let-bindings in **A-Normal Form** (every sub-expression is a
//! variable — no nested calls) and ends with a single **terminator** that
//! transfers control to another block or returns.
//!
//! This shape is deliberately close to what LLVM IR expects:
//! - One `MirBlock` ≈ one LLVM `BasicBlock`.
//! - One `MirInstr` ≈ one LLVM instruction.
//! - `MirTerminator::Switch` ≈ LLVM `switch` / conditional branch.
//! - `MirTerminator::Return` ≈ LLVM `ret`.
//!
//! The same shape also supports a **tree-walking interpreter**: the evaluator
//! maintains a `HashMap<MirVar, Value>` and steps through blocks.
//!
//! # What lowering must do (see `docs/plans/mir-lowering.md` for details)
//!
//! 1. **ANF transform** — flatten nested HIR expressions into a sequence of
//!    let-bound `MirVar`s. Every operand becomes a variable reference.
//! 2. **Closure conversion** — replace implicit lexical captures with explicit
//!    `MirInstr::MakeClosure { func, env }` and `MirInstr::LoadCapture` nodes.
//! 3. **Pattern-match compilation** — compile `HirExprKind::Match` into a
//!    decision tree of `MirTerminator::Switch` branches (Maranget algorithm).
//! 4. **Laziness** (open question) — optionally wrap values in
//!    `MirInstr::MakeThunk` / introduce `MirInstr::Force`. See plan doc.
//! 5. **Monomorphization vs boxing** (open question) — see plan doc.
//!
//! # Entry point (stub — implement in `lower/mod.rs`)
//!
//! ```text
//! let parsed = zutai_syntax::parse("x := 42\nx");
//! let result = zutai_semantic::analyze(&parsed.syntax());
//! // lower_module consumes the typed HIR and produces a MirModule.
//! // let mir = zutai_mir::lower::lower_module(&result.hir, &result.types);
//! ```

pub mod func;
pub mod lower;
pub mod module;

pub use func::{MirBlock, MirBlockId, MirFunc, MirInstr, MirTerminator, MirVar};
pub use module::{MirFuncId, MirModule};
