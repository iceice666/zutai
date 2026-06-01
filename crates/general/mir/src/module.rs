//! Top-level MIR container: [`MirModule`].

use crate::func::MirFunc;

// ── Index types ───────────────────────────────────────────────────────────────

/// Opaque index into `MirModule::funcs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirFuncId(pub u32);

// ── MirModule ─────────────────────────────────────────────────────────────────

/// A fully-lowered MIR module corresponding to one `.zt` source file.
///
/// One `MirFunc` per top-level declaration (value bindings are wrapped in a
/// zero-argument function). The entry point — the file's final expression —
/// is the function at `entry`.
///
/// # Interpreter note
/// Walk `funcs[entry]` to evaluate the file. Calls create new stack frames
/// that index into `funcs` by `MirFuncId`.
///
/// # LLVM note
/// Each `MirFunc` maps to one LLVM function definition. `entry` maps to the
/// module's `main`-equivalent (or a wrapper `zutai_eval_main`).
pub struct MirModule {
    /// All functions in this module (top-level decls + anonymous closures).
    pub funcs: Vec<MirFunc>,
    /// The function that produces the file's output value.
    pub entry: MirFuncId,
}
