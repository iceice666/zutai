use std::collections::HashMap;

use la_arena::Arena;

use crate::ir::{Row, TlcTupleField, TlcType, TlcTypeId};

use super::*;

use super::subst::*;

// ── Recursive normalizer ──────────────────────────────────────────────────────

pub(super) fn normalize_ty(
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
        TlcType::Prim(_) | TlcType::Opaque(_) | TlcType::Singleton(_) | TlcType::TyVar(_, _) => {
            Ok(ty)
        }
    }
}

pub(super) fn normalize_tuple_fields(
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

pub(super) fn normalize_row(
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
