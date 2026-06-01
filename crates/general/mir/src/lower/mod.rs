//! HIR → MIR lowering entry point.
//!
//! # Status: STUB
//!
//! This module is a placeholder. The data types in `crate::func` and
//! `crate::module` are fully defined; the actual lowering algorithms are
//! left for you to implement.
//!
//! # Where to start
//!
//! Read `docs/plans/mir-lowering.md` first — it contains:
//!
//! - The ANF transformation algorithm with worked examples.
//! - Closure conversion (free-variable analysis → `MakeClosure` emission).
//! - Pattern-match compilation (Maranget algorithm → `Switch` tree).
//! - Open design questions you must resolve before implementing:
//!   - **Strict vs lazy**: Do you want `Thunk`/`Force` in the MIR, or do you
//!     make the interpreter strict and handle laziness at a higher level?
//!   - **Monomorphize vs box**: Does every generic instantiation get its own
//!     MIR function, or do you use a uniform `*mut ()` + vtable rep?
//!   - **Curried vs uncurried**: One `MirFunc` per lambda, or multi-arg funcs?
//!
//! # Suggested implementation order
//!
//! 1. Implement `lower_expr` for literals and `Var` — trivial, establishes the
//!    recursive skeleton and the `MirVar` counter discipline.
//! 2. Add `Let` — introduces the core ANF flattening loop.
//! 3. Add `Apply` — first place you need to sequence two sub-expressions.
//! 4. Add `If` — first use of `Switch` terminator + block params (φ-nodes).
//! 5. Add `Lambda` — first closure (even if initially without capture).
//! 6. Add `Match` — pattern-match compilation; the hardest piece.
//! 7. Add the remaining expression forms (`Record`, `Field`, `BinOp`, …).
//! 8. Implement free-variable analysis and `MakeClosure` / `LoadCapture`.
//!
//! # Invariants to maintain
//!
//! - **ANF**: After `lower_expr(e)` returns a `MirVar`, every sub-expression
//!   of `e` is already bound to its own `MirVar`. No `MirInstr::Call` may
//!   appear with a non-variable argument.
//! - **SSA**: Each `MirVar` is assigned exactly once. Use `MirFunc::fresh_var`
//!   to allocate; never reuse.
//! - **Block termination**: Every `MirBlock` must end with a `MirTerminator`
//!   before the function is returned from `lower_func`. The `alloc_block` /
//!   fill-later pattern in `MirFunc::alloc_block` supports forward references.

use zutai_hir::HirFile;
use zutai_semantic::ty::TyInterner;

use crate::module::MirModule;

/// Lower a typed HIR file to a [`MirModule`].
///
/// # Precondition
///
/// `hir` must have been produced by `zutai_hir::lower_file` and then passed
/// through `zutai_semantic::analyze` so that `Symbol::ty` is populated for
/// all annotated bindings.
///
/// # Panics
///
/// Currently always panics — this is a stub. Remove the `todo!` as you
/// implement lowering.
#[allow(unused_variables)]
pub fn lower_module(hir: &HirFile, types: &TyInterner) -> MirModule {
    // TODO: implement HIR → MIR lowering.
    //
    // Sketch:
    //   1. Create a MirModule with an empty `funcs` vec.
    //   2. For each decl in `hir.decls`, call `lower_decl(decl, hir, types, &mut module)`.
    //      - Value decls: wrap the body in a zero-arg MirFunc (or inline into entry).
    //      - Function decls: `lower_func(body_expr, params, …)`.
    //   3. Lower `hir.final_expr` into the entry function.
    //   4. Return the completed MirModule.
    //
    // See docs/plans/mir-lowering.md for the full algorithm.
    todo!("HIR → MIR lowering not yet implemented — see docs/plans/mir-lowering.md")
}

// ── Internal helpers (add as you implement) ───────────────────────────────────
//
// Suggested signatures — uncomment and fill in as you go:
//
// fn lower_func(
//     body: zutai_hir::expr::HirExprId,
//     params: &[zutai_hir::symbol::SymbolId],
//     hir: &HirFile,
//     module: &mut MirModule,
// ) -> MirFuncId { todo!() }
//
// fn lower_expr(
//     expr_id: zutai_hir::expr::HirExprId,
//     hir: &HirFile,
//     func: &mut crate::func::MirFunc,
//     block: crate::func::MirBlockId,
//     module: &mut MirModule,
// ) -> (crate::func::MirVar, crate::func::MirBlockId) {
//     // Returns the variable holding the result and the (possibly new) current block.
//     todo!()
// }
//
// fn free_vars(
//     expr_id: zutai_hir::expr::HirExprId,
//     hir: &HirFile,
//     bound: &std::collections::HashSet<zutai_hir::symbol::SymbolId>,
// ) -> Vec<zutai_hir::symbol::SymbolId> { todo!() }
