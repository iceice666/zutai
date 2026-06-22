use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{TypeId, TypeKind, TypeTupleItem};

use crate::ir::{TlcExpr, TlcExprId};

use super::witness::ConditionalWitness;
use super::*;

impl<'thir> Lowerer<'thir> {
    /// Build a dict expression for a concrete type from a conditional witness.
    ///
    /// Finds a registered conditional witness whose target structurally matches
    /// `concrete` (treating the witness params as holes), then emits
    /// `App(…App(TyApp(Var(witness), arg₀), dict₀₀), …)`: one `TyApp` per witness
    /// param and one `App` per param bound, where each bound's dict is resolved
    /// recursively at the matched argument type. Returns `None` when no witness
    /// matches or the search recurses (guarded against non-termination).
    pub(super) fn resolve_conditional_witness(
        &mut self,
        cst_binding: BindingId,
        concrete: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        use crate::ir::{Row, TlcType};
        let guard = (cst_binding.0, concrete.0);
        if !self.resolving_dicts.insert(guard) {
            // Re-entry on the same (constraint, type): the witness search does not
            // make progress. Bail rather than recurse forever.
            return None;
        }
        let candidates: Vec<ConditionalWitness> = self
            .conditional_witnesses
            .iter()
            .filter(|cw| cw.constraint == cst_binding.0)
            .cloned()
            .collect();
        let mut result = None;
        for cw in candidates {
            let mut subst: FxHashMap<BindingId, TypeId> = FxHashMap::default();
            let holes: FxHashSet<BindingId> = cw.params.iter().copied().collect();
            if !self.unify_witness_target(cw.target, concrete, &holes, &mut subst) {
                continue;
            }
            // Each param must be pinned by the match; otherwise the witness is
            // not applicable to this concrete type.
            if cw.params.iter().any(|p| !subst.contains_key(p)) {
                continue;
            }
            let placeholder = self.alloc_type(TlcType::Record(Row::REmpty));
            let mut cur = self.alloc_expr(TlcExpr::Var(cw.binding), placeholder, span);
            let mut ok = true;
            for (param, bounds) in cw.params.iter().zip(cw.param_bounds.iter()) {
                let arg_ty_id = subst[param];
                let arg_ty = self.lower_type(arg_ty_id);
                cur = self.alloc_expr(TlcExpr::TyApp(cur, arg_ty), placeholder, span);
                for &bound in bounds {
                    let Some(dict) = self.try_get_dict_expr(bound, arg_ty_id, span) else {
                        // A required component witness is missing; this candidate
                        // cannot produce a usable dict.
                        ok = false;
                        break;
                    };
                    cur = self.alloc_expr(TlcExpr::App(cur, dict), placeholder, span);
                }
                if !ok {
                    break;
                }
            }
            if ok {
                result = Some(cur);
                break;
            }
        }
        self.resolving_dicts.remove(&guard);
        result
    }

    /// Structurally match a witness `target` (with `holes` as wildcards) against
    /// a `concrete` type, recording each hole's binding in `subst`. Aliases on
    /// either side are expanded (with their type args substituted) so a witness
    /// target written as `Pair A` matches a concrete `{fst:Int,snd:Int}` that
    /// THIR already expanded. Returns `false` on a shape mismatch or an
    /// inconsistent re-binding of a hole.
    pub(super) fn unify_witness_target(
        &self,
        target: TypeId,
        concrete: TypeId,
        holes: &FxHashSet<BindingId>,
        subst: &mut FxHashMap<BindingId, TypeId>,
    ) -> bool {
        self.unify_env(
            target,
            &FxHashMap::default(),
            concrete,
            &FxHashMap::default(),
            holes,
            subst,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn unify_env(
        &self,
        target: TypeId,
        tenv: &FxHashMap<BindingId, TypeId>,
        concrete: TypeId,
        cenv: &FxHashMap<BindingId, TypeId>,
        holes: &FxHashSet<BindingId>,
        subst: &mut FxHashMap<BindingId, TypeId>,
        depth: u32,
    ) -> bool {
        if depth > 64 {
            return false;
        }
        let no_holes = FxHashSet::default();
        let (target, tenv) = self.norm_ty(target, tenv, holes);
        let t_kind = self.thir.type_arena[target.0 as usize].kind.clone();
        // A hole matches any concrete type, but must bind consistently. Follow the
        // concrete's env-var chain (but do NOT alias-expand) so the binding stays
        // a self-contained type that `get_dict_expr` can re-resolve — expanding
        // here would strip an `AliasApply`'s args into dangling body variables.
        if let TypeKind::TypeVar(b) = t_kind
            && holes.contains(&b)
        {
            let resolved = self.resolve_env_var(concrete, cenv);
            return match subst.get(&b) {
                Some(&prev) => self.thir_types_equal(prev, resolved),
                None => {
                    subst.insert(b, resolved);
                    true
                }
            };
        }
        let (concrete, cenv) = self.norm_ty(concrete, cenv, &no_holes);
        let c_kind = self.thir.type_arena[concrete.0 as usize].kind.clone();
        match (t_kind, c_kind) {
            (TypeKind::List(ti), TypeKind::List(ci)) => {
                self.unify_env(ti, &tenv, ci, &cenv, holes, subst, depth + 1)
            }
            (TypeKind::Optional(ti), TypeKind::Optional(ci)) => {
                self.unify_env(ti, &tenv, ci, &cenv, holes, subst, depth + 1)
            }
            (TypeKind::Maybe(ti), TypeKind::Maybe(ci)) => {
                self.unify_env(ti, &tenv, ci, &cenv, holes, subst, depth + 1)
            }
            (
                TypeKind::Patch {
                    target: tt,
                    deep: td,
                },
                TypeKind::Patch {
                    target: ct,
                    deep: cd,
                },
            ) => td == cd && self.unify_env(tt, &tenv, ct, &cenv, holes, subst, depth + 1),
            (TypeKind::Tuple(ti), TypeKind::Tuple(ci)) => {
                ti.len() == ci.len()
                    && ti.iter().zip(ci.iter()).all(|(t, c)| match (t, c) {
                        (TypeTupleItem::Positional(tt), TypeTupleItem::Positional(cc)) => {
                            self.unify_env(*tt, &tenv, *cc, &cenv, holes, subst, depth + 1)
                        }
                        (
                            TypeTupleItem::Named {
                                name: tn, ty: tt, ..
                            },
                            TypeTupleItem::Named {
                                name: cn, ty: cc, ..
                            },
                        ) => {
                            tn == cn
                                && self.unify_env(*tt, &tenv, *cc, &cenv, holes, subst, depth + 1)
                        }
                        _ => false,
                    })
            }
            (TypeKind::Record(tf, tt), TypeKind::Record(cf, ct)) => {
                tt == ct
                    && tf.len() == cf.len()
                    && tf.iter().zip(cf.iter()).all(|(t, c)| {
                        t.name == c.name
                            && t.optional == c.optional
                            && self.unify_env(t.ty, &tenv, c.ty, &cenv, holes, subst, depth + 1)
                    })
            }
            (TypeKind::Union(tv, tt), TypeKind::Union(cv, ct)) => {
                tt == ct
                    && tv.len() == cv.len()
                    && tv.iter().zip(cv.iter()).all(|(t, c)| {
                        t.name == c.name
                            && match (t.payload, c.payload) {
                                (Some(tp), Some(cp)) => {
                                    self.unify_env(tp, &tenv, cp, &cenv, holes, subst, depth + 1)
                                }
                                (None, None) => true,
                                _ => false,
                            }
                    })
            }
            (TypeKind::Function { from: tf, to: tt }, TypeKind::Function { from: cf, to: ct }) => {
                self.unify_env(tf, &tenv, cf, &cenv, holes, subst, depth + 1)
                    && self.unify_env(tt, &tenv, ct, &cenv, holes, subst, depth + 1)
            }
            // Non-hole leaves and everything else must match exactly.
            _ => self.thir_types_equal(target, concrete),
        }
    }

    /// Normalize a type for witness-target matching: follow `env` substitutions
    /// for non-hole `TypeVar`s and expand `Alias`/`AliasApply` (recording their
    /// type args in the env) until the head is a concrete constructor, a hole, or
    /// a free variable. Returns the resolved type and the env for its subterms.
    pub(super) fn norm_ty(
        &self,
        ty: TypeId,
        env: &FxHashMap<BindingId, TypeId>,
        holes: &FxHashSet<BindingId>,
    ) -> (TypeId, FxHashMap<BindingId, TypeId>) {
        let mut ty = ty;
        let mut env = env.clone();
        let mut fuel = 64u32;
        while fuel > 0 {
            fuel -= 1;
            match self.thir.type_arena[ty.0 as usize].kind.clone() {
                TypeKind::TypeVar(b) if !holes.contains(&b) => match env.get(&b) {
                    Some(&next) => ty = next,
                    None => break,
                },
                TypeKind::Alias(b) => match self.type_alias_body(b) {
                    Some(body) => ty = body,
                    None => break,
                },
                TypeKind::AliasApply { binding, args } => {
                    match self.type_alias_params_body(binding) {
                        Some((params, body)) => {
                            for (p, a) in params.iter().zip(args.iter()) {
                                env.insert(*p, *a);
                            }
                            ty = body;
                        }
                        None => break,
                    }
                }
                _ => break,
            }
        }
        (ty, env)
    }

    /// Follow a `TypeVar` substitution chain through `env` (no alias expansion),
    /// yielding a self-contained `TypeId`. Used when binding a witness hole so the
    /// bound type keeps its `AliasApply` shape for later re-resolution.
    pub(super) fn resolve_env_var(&self, ty: TypeId, env: &FxHashMap<BindingId, TypeId>) -> TypeId {
        let mut ty = ty;
        let mut fuel = 64u32;
        while fuel > 0 {
            fuel -= 1;
            match self.thir.type_arena[ty.0 as usize].kind {
                TypeKind::TypeVar(b) => match env.get(&b) {
                    Some(&next) => ty = next,
                    None => break,
                },
                _ => break,
            }
        }
        ty
    }

    /// Structural equality of two THIR types via their witness keys. Used to
    /// compare non-hole leaves and re-bound holes during target matching.
    pub(super) fn thir_types_equal(&self, a: TypeId, b: TypeId) -> bool {
        if a == b {
            return true;
        }
        match (
            self.structural_witness_key(a, &mut FxHashSet::default()),
            self.structural_witness_key(b, &mut FxHashSet::default()),
        ) {
            (Some(ka), Some(kb)) => ka == kb,
            _ => false,
        }
    }
}
