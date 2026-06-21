//! Effect-row eraser: sets all `Fun` effect rows to `REmpty` before DC emission.
//!
//! Spec source of truth: `docs/ARCHIVED.md` §"TLC Phase 4 — Effect rows".
//!
//! ## Design
//!
//! In v0 every function is pure, so the THIR→TLC lowering already emits `eff = REmpty` for
//! every `Fun` type. The eraser therefore runs as a structural no-op on v0 programs.
//!
//! The pass still exists for two reasons:
//!
//! 1. **Invariant enforcement** — it asserts (by construction, not by assertion) that Dataflow
//!    Core only ever sees pure-typed functions, regardless of any future upstream change.
//! 2. **v1 hook** — when algebraic effects lower past TLC, the eraser runs after
//!    the pre-DC free-monad/CPS elaboration and discards only the now-redundant
//!    type annotation. The call site stays the same; only the pass body changes.
//!
//! The pass mutates the `TlcModule` type arena in place. It must be called on every `TlcModule`
//! before it is passed to the Dataflow Core lowering.

use crate::ir::{Row, TlcModule, TlcType};

impl TlcModule {
    /// Erase all effect rows: replace every `Fun(from, to, eff)` with
    /// `Fun(from, to, REmpty)` regardless of the current value of `eff`.
    ///
    /// Must be called before Dataflow Core emission (see `docs/ARCHIVED.md`
    /// §"TLC Phase 4"). In v0 this is always a structural no-op; the call is required for
    /// forward compatibility when v1 effects are introduced.
    pub fn erase_effects(&mut self) {
        for (_, ty) in self.type_arena.iter_mut() {
            if let TlcType::Fun(_, _, eff) = ty {
                *eff = Row::REmpty;
            }
        }
    }
}
