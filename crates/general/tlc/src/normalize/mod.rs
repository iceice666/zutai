//! NbE (Normalization-by-Evaluation) kernel for TLC types.
//!
//! Spec source of truth: `docs/tlc-core.md` §10 (NbE, reduction rules lines 468–469, fuel
//! limit line 475).
//!
//! ## Design
//!
//! * `normalize` / `normalize_with_fuel` are methods on `TlcModule` — the normalizer runs
//!   **post-hoc**, after `lower_thir` has built the complete module (all alias decls exist).
//! * An owned `alias_env: FxHashMap<u32, TlcTypeId>` maps each alias's `BindingId.0` to the
//!   `TyLamK`-chain body stored in its `TlcDecl::TypeAlias { body }`. This avoids the
//!   `&self.decl_arena` + `&mut self.type_arena` borrow conflict inside the recursive worker.
//! * The inner worker takes `(&mut Arena<TlcType>, &alias_env, &mut u32 /*fuel*/)` and
//!   returns `Result<TlcTypeId, NormalizeError>`.
//!
//! ## Reductions (each costs 1 fuel)
//!
//! * `TyApp(TyLamK(a, _, body), arg)` → `subst(body, a, arg)` then re-normalize (β-reduce).
//! * `TyApp(TyVar(alias, _), arg)` where `alias.0 ∈ alias_env` → unfold alias head and
//!   re-normalize.
//! * Otherwise, recurse structurally into children.
//!
//! ## Substitution (Phase 3: capture-avoiding)
//!
//! `subst` is capture-avoiding: before descending under a `TyLamK` / `ForAll` binder whose
//! variable appears free in the replacement, the binder is alpha-renamed to a fresh
//! `TlcTypeVar::Inferred(u32::MAX - counter)` variable (counting downward from `u32::MAX`
//! to avoid collision with THIR inference vars which start from 0).
//!
//! In v0 every replacement from the THIR lowering is a closed type (no free type variables),
//! so the freshening path is unreachable for any real `.zt` program. The upgrade is required
//! for v1 row polymorphism, where open-record/union types can carry free row-kinded type
//! variables.
//!
//! ## Type equality
//!
//! `types_equal` normalizes both sides, then does a deep structural comparison by dereferencing
//! arena indices — **not** via the derived `PartialEq` on `TlcTypeId` (which only compares the
//! index integers). Row equality is **order-insensitive** (permutation by label): `{a: Int, b: Str}`
//! equals `{b: Str, a: Int}`. α-equivalence over bound row *variables* (renaming a quantified
//! `RVar`) remains deferred.
//!
//! `Fun` carries an effect row (Phase 4); its effect row is compared via the same
//! order-insensitive `rows_equal_deep` — correct for effect sets. In v0, `eff = REmpty` always.

use rustc_hash::FxHashMap;

use la_arena::Arena;

use crate::ir::{TlcDecl, TlcModule, TlcTypeId, TlcTypeVar};

mod equal;
mod reduce;
mod subst;

use self::equal::*;
use self::reduce::*;

/// Fuel limit from spec §10 line 475.
pub const DEFAULT_FUEL: u32 = 1000;

/// Error returned when the normalizer exceeds its step budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizeError {
    FuelExhausted { limit: u32 },
}

impl TlcModule {
    /// Normalize `ty` using `DEFAULT_FUEL` steps.
    pub fn normalize(&mut self, ty: TlcTypeId) -> Result<TlcTypeId, NormalizeError> {
        self.normalize_with_fuel(ty, DEFAULT_FUEL)
    }

    /// Normalize `ty` using at most `fuel` β-reduction steps.
    pub fn normalize_with_fuel(
        &mut self,
        ty: TlcTypeId,
        fuel: u32,
    ) -> Result<TlcTypeId, NormalizeError> {
        let alias_env = build_alias_env(&self.decl_arena);
        let mut remaining = fuel;
        // Fresh-variable counter for capture-avoiding substitution.
        // Counts downward from u32::MAX; THIR infer vars count upward from 0, so no collision.
        let mut next_fresh = u32::MAX;
        normalize_ty(
            &mut self.type_arena,
            &alias_env,
            ty,
            &mut remaining,
            fuel,
            &mut next_fresh,
        )
    }

    /// Returns `true` iff `a` and `b` normalize to structurally identical types.
    ///
    /// Returns `false` conservatively on fuel exhaustion — unprovable-equal ⇒ not equal.
    /// (A wrong `true` is worse than a refused `false`.)
    pub fn types_equal(&mut self, a: TlcTypeId, b: TlcTypeId) -> bool {
        let Ok(na) = self.normalize(a) else {
            return false;
        };
        let Ok(nb) = self.normalize(b) else {
            return false;
        };
        types_equal_deep(&self.type_arena, na, nb)
    }
}

// ── Alias environment ─────────────────────────────────────────────────────────

/// Map every alias's `BindingId.0` → its `TyLamK`-chain body.
fn build_alias_env(decl_arena: &Arena<TlcDecl>) -> FxHashMap<u32, TlcTypeId> {
    decl_arena
        .iter()
        .filter_map(|(_, decl)| match decl {
            TlcDecl::TypeAlias { binding, body, .. } => Some((binding.0, *body)),
            TlcDecl::Value { .. } => None,
        })
        .collect()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tyvar_key(v: TlcTypeVar) -> u32 {
    match v {
        TlcTypeVar::Named(k) => k,
        TlcTypeVar::Inferred(k) => k,
    }
}

fn consume_fuel(fuel: &mut u32, limit: u32) -> Result<(), NormalizeError> {
    if *fuel == 0 {
        Err(NormalizeError::FuelExhausted { limit })
    } else {
        *fuel -= 1;
        Ok(())
    }
}
