//! NbE (Normalization-by-Evaluation) kernel for TLC types.
//!
//! Spec source of truth: `docs/tlc-core.md` §10 (NbE, reduction rules lines 468–469, fuel
//! limit line 475).
//!
//! ## Design
//!
//! * `normalize` / `normalize_with_fuel` are methods on `TlcModule` — the normalizer runs
//!   **post-hoc**, after `lower_thir` has built the complete module (all alias decls exist).
//! * An owned `alias_env: HashMap<u32, TlcTypeId>` maps each alias's `BindingId.0` to the
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

use std::collections::{HashMap, HashSet};

use la_arena::Arena;

use crate::ir::{Row, TlcDecl, TlcModule, TlcTupleField, TlcType, TlcTypeId, TlcTypeVar};

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
fn build_alias_env(decl_arena: &Arena<TlcDecl>) -> HashMap<u32, TlcTypeId> {
    decl_arena
        .iter()
        .filter_map(|(_, decl)| match decl {
            TlcDecl::TypeAlias { binding, body, .. } => Some((binding.0, *body)),
            TlcDecl::Value { .. } => None,
        })
        .collect()
}

// ── Recursive normalizer ──────────────────────────────────────────────────────

fn normalize_ty(
    arena: &mut Arena<TlcType>,
    alias_env: &HashMap<u32, TlcTypeId>,
    ty: TlcTypeId,
    fuel: &mut u32,
    fuel_limit: u32,
    next_fresh: &mut u32,
) -> Result<TlcTypeId, NormalizeError> {
    match arena[ty].clone() {
        // ── β-reduction: TyApp(TyLamK(a, _, body), arg) ─────────────────────
        TlcType::TyApp(func_id, arg_id) => {
            match arena[func_id].clone() {
                TlcType::TyLamK(binder, _kind, body_id) => {
                    consume_fuel(fuel, fuel_limit)?;
                    // Normalize the argument first, then substitute.
                    let norm_arg =
                        normalize_ty(arena, alias_env, arg_id, fuel, fuel_limit, next_fresh)?;
                    let substituted = subst(arena, body_id, binder, norm_arg, next_fresh);
                    normalize_ty(arena, alias_env, substituted, fuel, fuel_limit, next_fresh)
                }
                // ── Alias-head unfolding: TyApp(TyVar(alias, _), arg) ────────
                TlcType::TyVar(tyvar, _kind) => {
                    let binding_key = tyvar_key(tyvar);
                    if let Some(&alias_body) = alias_env.get(&binding_key) {
                        consume_fuel(fuel, fuel_limit)?;
                        // Rebuild TyApp with the unfolded alias body, then re-normalize.
                        let new_app = arena.alloc(TlcType::TyApp(alias_body, arg_id));
                        normalize_ty(arena, alias_env, new_app, fuel, fuel_limit, next_fresh)
                    } else {
                        // func_id is already a TyVar (normal form) — only normalize arg.
                        let na =
                            normalize_ty(arena, alias_env, arg_id, fuel, fuel_limit, next_fresh)?;
                        Ok(arena.alloc(TlcType::TyApp(func_id, na)))
                    }
                }
                _ => {
                    // Head is not immediately reducible: normalize both sides, then
                    // inspect the normalized head — it may now be a TyLamK (curried
                    // alias after partial application) or an alias TyVar.
                    let nf = normalize_ty(arena, alias_env, func_id, fuel, fuel_limit, next_fresh)?;
                    let na = normalize_ty(arena, alias_env, arg_id, fuel, fuel_limit, next_fresh)?;
                    match arena[nf].clone() {
                        TlcType::TyLamK(binder, _kind, body_id) => {
                            // β-reduce the now-exposed lambda.
                            consume_fuel(fuel, fuel_limit)?;
                            let substituted = subst(arena, body_id, binder, na, next_fresh);
                            normalize_ty(
                                arena,
                                alias_env,
                                substituted,
                                fuel,
                                fuel_limit,
                                next_fresh,
                            )
                        }
                        TlcType::TyVar(tyvar, _kind) => {
                            let binding_key = tyvar_key(tyvar);
                            if let Some(&alias_body) = alias_env.get(&binding_key) {
                                // Unfold the now-exposed alias head.
                                consume_fuel(fuel, fuel_limit)?;
                                let new_app = arena.alloc(TlcType::TyApp(alias_body, na));
                                normalize_ty(
                                    arena, alias_env, new_app, fuel, fuel_limit, next_fresh,
                                )
                            } else {
                                Ok(arena.alloc(TlcType::TyApp(nf, na)))
                            }
                        }
                        _ => Ok(arena.alloc(TlcType::TyApp(nf, na))),
                    }
                }
            }
        }

        // ── Structural recursion into children ───────────────────────────────
        TlcType::TyLamK(binder, kind, body_id) => {
            let nb = normalize_ty(arena, alias_env, body_id, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::TyLamK(binder, kind, nb)))
        }
        TlcType::ForAll(binder, kind, body_id) => {
            let nb = normalize_ty(arena, alias_env, body_id, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::ForAll(binder, kind, nb)))
        }
        TlcType::Fun(from_id, to_id, eff) => {
            let nf = normalize_ty(arena, alias_env, from_id, fuel, fuel_limit, next_fresh)?;
            let nt = normalize_ty(arena, alias_env, to_id, fuel, fuel_limit, next_fresh)?;
            let neff = normalize_row(arena, alias_env, &eff, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::Fun(nf, nt, neff)))
        }
        TlcType::List(inner_id) => {
            let ni = normalize_ty(arena, alias_env, inner_id, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::List(ni)))
        }
        TlcType::Optional(inner_id) => {
            let ni = normalize_ty(arena, alias_env, inner_id, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::Optional(ni)))
        }
        TlcType::Maybe(inner_id) => {
            let ni = normalize_ty(arena, alias_env, inner_id, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::Maybe(ni)))
        }
        TlcType::Record(row) => {
            let new_row = normalize_row(arena, alias_env, &row, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::Record(new_row)))
        }
        TlcType::VariantT(row) => {
            let new_row = normalize_row(arena, alias_env, &row, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::VariantT(new_row)))
        }
        TlcType::Tuple(items) => {
            let new_items =
                normalize_tuple_fields(arena, alias_env, &items, fuel, fuel_limit, next_fresh)?;
            Ok(arena.alloc(TlcType::Tuple(new_items)))
        }
        // Atoms — nothing to reduce.
        TlcType::Prim(_) | TlcType::Singleton(_) | TlcType::TyVar(_, _) => Ok(ty),
    }
}

fn normalize_tuple_fields(
    arena: &mut Arena<TlcType>,
    alias_env: &HashMap<u32, TlcTypeId>,
    items: &[TlcTupleField],
    fuel: &mut u32,
    fuel_limit: u32,
    next_fresh: &mut u32,
) -> Result<Vec<TlcTupleField>, NormalizeError> {
    items
        .iter()
        .map(|item| match item {
            TlcTupleField::Positional(ty_id) => {
                let nt = normalize_ty(arena, alias_env, *ty_id, fuel, fuel_limit, next_fresh)?;
                Ok(TlcTupleField::Positional(nt))
            }
            TlcTupleField::Named { name, ty: ty_id } => {
                let nt = normalize_ty(arena, alias_env, *ty_id, fuel, fuel_limit, next_fresh)?;
                Ok(TlcTupleField::Named {
                    name: name.clone(),
                    ty: nt,
                })
            }
        })
        .collect()
}

fn normalize_row(
    arena: &mut Arena<TlcType>,
    alias_env: &HashMap<u32, TlcTypeId>,
    row: &Row,
    fuel: &mut u32,
    fuel_limit: u32,
    next_fresh: &mut u32,
) -> Result<Row, NormalizeError> {
    match row {
        Row::REmpty => Ok(Row::REmpty),
        // RVar is inert under type normalization — a row variable has no reduct.
        Row::RVar(v) => Ok(Row::RVar(*v)),
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } => {
            let nt = normalize_ty(arena, alias_env, *ty, fuel, fuel_limit, next_fresh)?;
            let ntail = normalize_row(arena, alias_env, tail, fuel, fuel_limit, next_fresh)?;
            Ok(Row::RExtend {
                label: label.clone(),
                ty: nt,
                optional: *optional,
                tail: Box::new(ntail),
            })
        }
    }
}

// ── Free type variables ───────────────────────────────────────────────────────

/// Collect all type variables (`TyVar`) that appear free in `ty`.
/// Row variables (`RVar`) are ignored — they have a different kind and cannot be
/// captured by `TyLamK`/`ForAll` binders, which bind type-kinded variables.
fn free_type_vars(arena: &Arena<TlcType>, ty: TlcTypeId) -> HashSet<TlcTypeVar> {
    let mut free = HashSet::new();
    collect_free(arena, ty, &HashSet::new(), &mut free);
    free
}

fn collect_free(
    arena: &Arena<TlcType>,
    ty: TlcTypeId,
    bound: &HashSet<TlcTypeVar>,
    free: &mut HashSet<TlcTypeVar>,
) {
    match arena[ty].clone() {
        TlcType::TyVar(v, _) => {
            if !bound.contains(&v) {
                free.insert(v);
            }
        }
        TlcType::TyLamK(binder, _, body) => {
            let mut new_bound = bound.clone();
            new_bound.insert(binder);
            collect_free(arena, body, &new_bound, free);
        }
        TlcType::ForAll(binder, _, body) => {
            let mut new_bound = bound.clone();
            new_bound.insert(binder);
            collect_free(arena, body, &new_bound, free);
        }
        TlcType::TyApp(f, a) => {
            collect_free(arena, f, bound, free);
            collect_free(arena, a, bound, free);
        }
        TlcType::Fun(from, to, eff) => {
            collect_free(arena, from, bound, free);
            collect_free(arena, to, bound, free);
            collect_free_row(arena, &eff, bound, free);
        }
        TlcType::List(inner) | TlcType::Optional(inner) | TlcType::Maybe(inner) => {
            collect_free(arena, inner, bound, free);
        }
        TlcType::Record(row) | TlcType::VariantT(row) => {
            collect_free_row(arena, &row, bound, free);
        }
        TlcType::Tuple(items) => {
            for item in &items {
                let t = match item {
                    TlcTupleField::Positional(t) => *t,
                    TlcTupleField::Named { ty, .. } => *ty,
                };
                collect_free(arena, t, bound, free);
            }
        }
        TlcType::Prim(_) | TlcType::Singleton(_) => {}
    }
}

fn collect_free_row(
    arena: &Arena<TlcType>,
    row: &Row,
    bound: &HashSet<TlcTypeVar>,
    free: &mut HashSet<TlcTypeVar>,
) {
    match row {
        Row::REmpty => {}
        // RVar shares the TlcTypeVar namespace with type-kinded variables. A TyLamK/ForAll
        // binder whose id appears as a free RVar in the replacement must be freshened.
        Row::RVar(v) => {
            if !bound.contains(v) {
                free.insert(*v);
            }
        }
        Row::RExtend { ty, tail, .. } => {
            collect_free(arena, *ty, bound, free);
            collect_free_row(arena, tail, bound, free);
        }
    }
}

// ── Alpha-renaming ────────────────────────────────────────────────────────────

/// Rename all free occurrences of `old` to `fresh` in `ty`.
/// `fresh` must not appear anywhere in `ty` (guaranteed when allocated from the
/// downward counter in `subst_inner`), so no capture check is needed here.
fn alpha_rename(
    arena: &mut Arena<TlcType>,
    ty: TlcTypeId,
    old: TlcTypeVar,
    fresh: TlcTypeVar,
) -> TlcTypeId {
    match arena[ty].clone() {
        TlcType::TyVar(v, k) if v == old => arena.alloc(TlcType::TyVar(fresh, k)),
        TlcType::TyVar(_, _) => ty,
        TlcType::TyLamK(binder, kind, body) => {
            if binder == old {
                return ty; // old is re-bound here — stop descending
            }
            let new_body = alpha_rename(arena, body, old, fresh);
            arena.alloc(TlcType::TyLamK(binder, kind, new_body))
        }
        TlcType::ForAll(binder, kind, body) => {
            if binder == old {
                return ty; // old is re-bound here — stop descending
            }
            let new_body = alpha_rename(arena, body, old, fresh);
            arena.alloc(TlcType::ForAll(binder, kind, new_body))
        }
        TlcType::TyApp(f, a) => {
            let nf = alpha_rename(arena, f, old, fresh);
            let na = alpha_rename(arena, a, old, fresh);
            arena.alloc(TlcType::TyApp(nf, na))
        }
        TlcType::Fun(from, to, eff) => {
            let nf = alpha_rename(arena, from, old, fresh);
            let nt = alpha_rename(arena, to, old, fresh);
            let neff = alpha_rename_row(arena, eff, old, fresh);
            arena.alloc(TlcType::Fun(nf, nt, neff))
        }
        TlcType::List(inner) => {
            let ni = alpha_rename(arena, inner, old, fresh);
            arena.alloc(TlcType::List(ni))
        }
        TlcType::Optional(inner) => {
            let ni = alpha_rename(arena, inner, old, fresh);
            arena.alloc(TlcType::Optional(ni))
        }
        TlcType::Maybe(inner) => {
            let ni = alpha_rename(arena, inner, old, fresh);
            arena.alloc(TlcType::Maybe(ni))
        }
        TlcType::Record(row) => {
            let new_row = alpha_rename_row(arena, row, old, fresh);
            arena.alloc(TlcType::Record(new_row))
        }
        TlcType::VariantT(row) => {
            let new_row = alpha_rename_row(arena, row, old, fresh);
            arena.alloc(TlcType::VariantT(new_row))
        }
        TlcType::Tuple(items) => {
            let new_items: Vec<TlcTupleField> = items
                .iter()
                .map(|item| match item {
                    TlcTupleField::Positional(t) => {
                        TlcTupleField::Positional(alpha_rename(arena, *t, old, fresh))
                    }
                    TlcTupleField::Named { name, ty } => TlcTupleField::Named {
                        name: name.clone(),
                        ty: alpha_rename(arena, *ty, old, fresh),
                    },
                })
                .collect();
            arena.alloc(TlcType::Tuple(new_items))
        }
        TlcType::Prim(_) | TlcType::Singleton(_) => ty,
    }
}

fn alpha_rename_row(
    arena: &mut Arena<TlcType>,
    row: Row,
    old: TlcTypeVar,
    fresh: TlcTypeVar,
) -> Row {
    match row {
        Row::REmpty => Row::REmpty,
        // RVar shares the TlcTypeVar namespace; rename it when it matches `old`.
        Row::RVar(v) if v == old => Row::RVar(fresh),
        Row::RVar(v) => Row::RVar(v),
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } => Row::RExtend {
            label,
            ty: alpha_rename(arena, ty, old, fresh),
            optional,
            tail: Box::new(alpha_rename_row(arena, *tail, old, fresh)),
        },
    }
}

// ── Capture-avoiding substitution ────────────────────────────────────────────

/// Entry point: substitute all free occurrences of `var` in `ty` with `replacement`.
///
/// Capture-avoiding: if a `TyLamK`/`ForAll` binder `b ≠ var` has the same id as a free
/// variable in `replacement`, `b` is alpha-renamed to a fresh `Inferred(next_fresh)` before
/// descending. This prevents the replacement's free variable from being shadowed by `b`.
///
/// In v0 all replacements from THIR lowering are closed types, so the freshening path is
/// unreachable for any real `.zt` program. The upgrade is mandatory for v1 row-polymorphic
/// types where open-record/union types can carry free type variables.
fn subst(
    arena: &mut Arena<TlcType>,
    ty: TlcTypeId,
    var: TlcTypeVar,
    replacement: TlcTypeId,
    next_fresh: &mut u32,
) -> TlcTypeId {
    let replacement_free = free_type_vars(arena, replacement);
    subst_inner(arena, ty, var, replacement, &replacement_free, next_fresh)
}

fn subst_inner(
    arena: &mut Arena<TlcType>,
    ty: TlcTypeId,
    var: TlcTypeVar,
    replacement: TlcTypeId,
    replacement_free: &HashSet<TlcTypeVar>,
    next_fresh: &mut u32,
) -> TlcTypeId {
    match arena[ty].clone() {
        TlcType::TyVar(v, _) if v == var => replacement,
        TlcType::TyVar(_, _) => ty,

        // Capture-avoiding: freshen any binder whose id appears free in `replacement`.
        TlcType::TyLamK(binder, kind, body) => {
            if binder == var {
                return ty; // shadowed — do not descend
            }
            let (new_binder, new_body) = if replacement_free.contains(&binder) {
                let fresh = TlcTypeVar::Inferred(*next_fresh);
                *next_fresh -= 1;
                let renamed = alpha_rename(arena, body, binder, fresh);
                (fresh, renamed)
            } else {
                (binder, body)
            };
            let subst_body = subst_inner(
                arena,
                new_body,
                var,
                replacement,
                replacement_free,
                next_fresh,
            );
            arena.alloc(TlcType::TyLamK(new_binder, kind, subst_body))
        }
        TlcType::ForAll(binder, kind, body) => {
            if binder == var {
                return ty; // shadowed
            }
            let (new_binder, new_body) = if replacement_free.contains(&binder) {
                let fresh = TlcTypeVar::Inferred(*next_fresh);
                *next_fresh -= 1;
                let renamed = alpha_rename(arena, body, binder, fresh);
                (fresh, renamed)
            } else {
                (binder, body)
            };
            let subst_body = subst_inner(
                arena,
                new_body,
                var,
                replacement,
                replacement_free,
                next_fresh,
            );
            arena.alloc(TlcType::ForAll(new_binder, kind, subst_body))
        }

        TlcType::TyApp(f, a) => {
            let nf = subst_inner(arena, f, var, replacement, replacement_free, next_fresh);
            let na = subst_inner(arena, a, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::TyApp(nf, na))
        }
        TlcType::Fun(from, to, eff) => {
            let nf = subst_inner(arena, from, var, replacement, replacement_free, next_fresh);
            let nt = subst_inner(arena, to, var, replacement, replacement_free, next_fresh);
            let neff = subst_row_inner(arena, &eff, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Fun(nf, nt, neff))
        }
        TlcType::List(inner) => {
            let ni = subst_inner(arena, inner, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::List(ni))
        }
        TlcType::Optional(inner) => {
            let ni = subst_inner(arena, inner, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Optional(ni))
        }
        TlcType::Maybe(inner) => {
            let ni = subst_inner(arena, inner, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Maybe(ni))
        }
        TlcType::Record(row) => {
            let new_row =
                subst_row_inner(arena, &row, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::Record(new_row))
        }
        TlcType::VariantT(row) => {
            let new_row =
                subst_row_inner(arena, &row, var, replacement, replacement_free, next_fresh);
            arena.alloc(TlcType::VariantT(new_row))
        }
        TlcType::Tuple(items) => {
            let new_items: Vec<TlcTupleField> = items
                .iter()
                .map(|item| match item {
                    TlcTupleField::Positional(ty_id) => TlcTupleField::Positional(subst_inner(
                        arena,
                        *ty_id,
                        var,
                        replacement,
                        replacement_free,
                        next_fresh,
                    )),
                    TlcTupleField::Named { name, ty: ty_id } => TlcTupleField::Named {
                        name: name.clone(),
                        ty: subst_inner(
                            arena,
                            *ty_id,
                            var,
                            replacement,
                            replacement_free,
                            next_fresh,
                        ),
                    },
                })
                .collect();
            arena.alloc(TlcType::Tuple(new_items))
        }
        // Atoms — nothing to substitute.
        TlcType::Prim(_) | TlcType::Singleton(_) => ty,
    }
}

fn subst_row_inner(
    arena: &mut Arena<TlcType>,
    row: &Row,
    var: TlcTypeVar,
    replacement: TlcTypeId,
    replacement_free: &HashSet<TlcTypeVar>,
    next_fresh: &mut u32,
) -> Row {
    match row {
        Row::REmpty => Row::REmpty,
        // Type-variable substitution is inert on row variables — different kind discipline.
        Row::RVar(v) => Row::RVar(*v),
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } => Row::RExtend {
            label: label.clone(),
            ty: subst_inner(arena, *ty, var, replacement, replacement_free, next_fresh),
            optional: *optional,
            tail: Box::new(subst_row_inner(
                arena,
                tail,
                var,
                replacement,
                replacement_free,
                next_fresh,
            )),
        },
    }
}

// ── Deep structural equality ──────────────────────────────────────────────────

/// Collect `(label, ty, optional)` fields from a row spine (stopping at `REmpty` or `RVar`),
/// and return the tail kind (`None` = closed, `Some(v)` = open via `RVar(v)`).
fn row_fields_and_tail(row: &Row) -> (Vec<(&str, TlcTypeId, bool)>, Option<TlcTypeVar>) {
    let mut fields = Vec::new();
    let mut cur = row;
    loop {
        match cur {
            Row::REmpty => return (fields, None),
            Row::RVar(v) => return (fields, Some(*v)),
            Row::RExtend {
                label,
                ty,
                optional,
                tail,
            } => {
                fields.push((label.as_str(), *ty, *optional));
                cur = tail;
            }
        }
    }
}

/// Deep structural equality by dereferencing arena IDs (post-normalization).
/// Row comparison is order-insensitive (permutation by label). Binder α-equivalence
/// (`∀a.F(a)` ≡ `∀b.F(b)`) is NOT implemented — binders must be syntactically identical.
fn types_equal_deep(arena: &Arena<TlcType>, a: TlcTypeId, b: TlcTypeId) -> bool {
    if a == b {
        return true; // fast path: same index
    }
    match (arena[a].clone(), arena[b].clone()) {
        (TlcType::Prim(pa), TlcType::Prim(pb)) => pa == pb,
        (TlcType::Singleton(la), TlcType::Singleton(lb)) => la == lb,
        (TlcType::TyVar(va, ka), TlcType::TyVar(vb, kb)) => va == vb && ka == kb,
        (TlcType::Fun(f1, t1, e1), TlcType::Fun(f2, t2, e2)) => {
            types_equal_deep(arena, f1, f2)
                && types_equal_deep(arena, t1, t2)
                && rows_equal_deep(arena, &e1, &e2)
        }
        (TlcType::List(i1), TlcType::List(i2)) => types_equal_deep(arena, i1, i2),
        (TlcType::Optional(i1), TlcType::Optional(i2)) => types_equal_deep(arena, i1, i2),
        (TlcType::Maybe(i1), TlcType::Maybe(i2)) => types_equal_deep(arena, i1, i2),
        (TlcType::TyApp(f1, a1), TlcType::TyApp(f2, a2)) => {
            types_equal_deep(arena, f1, f2) && types_equal_deep(arena, a1, a2)
        }
        (TlcType::TyLamK(v1, k1, b1), TlcType::TyLamK(v2, k2, b2)) => {
            v1 == v2 && k1 == k2 && types_equal_deep(arena, b1, b2)
        }
        (TlcType::ForAll(v1, k1, b1), TlcType::ForAll(v2, k2, b2)) => {
            v1 == v2 && k1 == k2 && types_equal_deep(arena, b1, b2)
        }
        (TlcType::Record(r1), TlcType::Record(r2)) => rows_equal_deep(arena, &r1, &r2),
        (TlcType::VariantT(r1), TlcType::VariantT(r2)) => rows_equal_deep(arena, &r1, &r2),
        (TlcType::Tuple(i1), TlcType::Tuple(i2)) => {
            i1.len() == i2.len()
                && i1.iter().zip(i2.iter()).all(|(a, b)| match (a, b) {
                    (TlcTupleField::Positional(ta), TlcTupleField::Positional(tb)) => {
                        types_equal_deep(arena, *ta, *tb)
                    }
                    (
                        TlcTupleField::Named { name: na, ty: ta },
                        TlcTupleField::Named { name: nb, ty: tb },
                    ) => na == nb && types_equal_deep(arena, *ta, *tb),
                    _ => false,
                })
        }
        _ => false,
    }
}

/// Order-insensitive (permutation by label) row equality (post-normalization).
///
/// Two rows are equal iff they have the same label set (matching `optional` and field type) and
/// the same tail (`REmpty` or the same `RVar`). No α-equivalence over quantified row variables.
fn rows_equal_deep(arena: &Arena<TlcType>, a: &Row, b: &Row) -> bool {
    let (fields_a, tail_a) = row_fields_and_tail(a);
    let (fields_b, tail_b) = row_fields_and_tail(b);

    // Tails must match (both closed, or same row variable).
    if tail_a != tail_b {
        return false;
    }
    if fields_a.len() != fields_b.len() {
        return false;
    }

    // Build a label → (ty, optional) map for b's fields.
    let map_b: HashMap<&str, (TlcTypeId, bool)> = fields_b
        .iter()
        .map(|&(l, ty, opt)| (l, (ty, opt)))
        .collect();

    fields_a.iter().all(|&(label, ty_a, opt_a)| {
        if let Some(&(ty_b, opt_b)) = map_b.get(label) {
            opt_a == opt_b && types_equal_deep(arena, ty_a, ty_b)
        } else {
            false
        }
    })
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
