use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{
    RowTail, TypeId, TypeKind, TypeTupleItem, WitnessPattern, WitnessPatternTupleItem,
};

use crate::ir::{TlcExpr, TlcExprId};

use super::witness::{ConditionalWitness, ExternConditionalWitness};
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

    /// Chain every dict-resolution strategy, returning `None` (rather than
    /// `Lit(Nothing)`) when none applies. Used for component-dict resolution
    /// inside conditional witnesses, where a missing component must fail the
    /// match instead of silently passing `Nothing`.
    pub(super) fn try_resolve_dict(
        &mut self,
        cst_binding: BindingId,
        inst_type_id: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        if let Some(e) = self.try_get_dict_expr(cst_binding, inst_type_id, span) {
            return Some(e);
        }
        let cst_name = self
            .thir
            .binding_names
            .get(cst_binding.0 as usize)
            .cloned()
            .unwrap_or_default();
        self.try_extern_dict_by_name(&cst_name, inst_type_id, span)
    }

    /// Resolve a component dict by constraint *name* (the form imported witness
    /// bounds carry). Prefers this module's own constraint declaration when it
    /// exists (so local witnesses / active dict params apply); otherwise resolves
    /// purely against the imported extern tables — a component constraint a dep's
    /// witness bound names need not be declared by the importer.
    fn get_dict_expr_by_constraint_name(
        &mut self,
        name: &str,
        inst_type_id: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        if let Some(cst_binding) = self.constraint_binding_by_name(name)
            && let Some(e) = self.try_get_dict_expr(cst_binding, inst_type_id, span)
        {
            return Some(e);
        }
        self.try_extern_dict_by_name(name, inst_type_id, span)
    }

    /// Resolve a dict from the imported extern tables (concrete then conditional)
    /// by constraint name alone.
    pub(super) fn try_extern_dict_by_name(
        &mut self,
        cst_name: &str,
        inst_type_id: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        if let Some(e) = self.try_extern_witness_expr(cst_name, inst_type_id, span) {
            return Some(e);
        }
        self.try_extern_conditional_witness(cst_name, inst_type_id, span)
    }

    /// Find a constraint declaration's binding by name in this module's THIR.
    fn constraint_binding_by_name(&self, name: &str) -> Option<BindingId> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if matches!(decl.kind, zutai_thir::ThirDeclKind::Constraint { .. })
                && self
                    .thir
                    .binding_names
                    .get(decl.binding.0 as usize)
                    .map(String::as_str)
                    == Some(name)
            {
                Some(decl.binding)
            } else {
                None
            }
        })
    }

    /// Try to build a dict expression from an imported conditional witness.
    ///
    /// Matches each registered imported conditional witness for constraint
    /// `cst_name`, recovering the type bound to each parameter hole, then emits the
    /// dep-namespaced witness global applied (`TyApp` per param, `App` per
    /// component bound) to the recursively-resolved component dicts.
    pub(super) fn try_extern_conditional_witness(
        &mut self,
        cst_name: &str,
        inst_type_id: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        use crate::ir::{Row, TlcType};
        let guard = (cst_name.to_string(), inst_type_id.0);
        if !self.resolving_extern.insert(guard.clone()) {
            return None;
        }
        let candidates: Vec<ExternConditionalWitness> = self
            .extern_conditionals
            .iter()
            .filter(|cw| cw.constraint == cst_name)
            .cloned()
            .collect();
        let mut result = None;
        for cw in candidates {
            let mut holes: Vec<Option<TypeId>> = vec![None; cw.param_bounds.len()];
            if !self.match_witness_pattern(
                &cw.pattern,
                inst_type_id,
                &FxHashMap::default(),
                &mut holes,
                0,
            ) {
                continue;
            }
            if holes.iter().any(Option::is_none) {
                continue;
            }
            let placeholder = self.alloc_type(TlcType::Record(Row::REmpty));
            let virtual_id = self.alloc_virtual_binding(cw.global.clone());
            let mut cur = self.alloc_expr(TlcExpr::Var(virtual_id), placeholder, span);
            let mut ok = true;
            for (i, bounds) in cw.param_bounds.iter().enumerate() {
                let arg_ty_id = holes[i].expect("pinned above");
                let arg_ty = self.lower_type(arg_ty_id);
                cur = self.alloc_expr(TlcExpr::TyApp(cur, arg_ty), placeholder, span);
                for bound_name in bounds {
                    let Some(dict) =
                        self.get_dict_expr_by_constraint_name(bound_name, arg_ty_id, span)
                    else {
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
        self.resolving_extern.remove(&guard);
        result
    }

    /// Structurally match an imported witness `pattern` (parameter holes) against
    /// a `concrete` THIR type, binding each hole's recovered type into `holes`.
    /// Mirrors [`Self::unify_env`] with a [`WitnessPattern`] on the target side.
    fn match_witness_pattern(
        &self,
        pat: &WitnessPattern,
        concrete: TypeId,
        cenv: &FxHashMap<BindingId, TypeId>,
        holes: &mut Vec<Option<TypeId>>,
        depth: u32,
    ) -> bool {
        if depth > 64 {
            return false;
        }
        if let WitnessPattern::Hole(i) = pat {
            let resolved = self.resolve_env_var(concrete, cenv);
            return match holes.get_mut(*i) {
                Some(slot @ None) => {
                    *slot = Some(resolved);
                    true
                }
                Some(Some(prev)) => self.thir_types_equal(*prev, resolved),
                None => false,
            };
        }
        if matches!(pat, WitnessPattern::Any) {
            return true;
        }
        if let WitnessPattern::ConApply {
            ctor,
            args,
            remaining,
        } = pat
        {
            let (binding, concrete_args) =
                match self.thir.type_arena[concrete.0 as usize].kind.clone() {
                    TypeKind::AliasApply { binding, args } => (binding, args),
                    TypeKind::Apply { .. } => {
                        let (head, args) = self.thir_app_spine(concrete);
                        let binding = match self.thir.type_arena[head.0 as usize].kind {
                            TypeKind::Alias(binding) | TypeKind::Con(binding) => binding,
                            _ => return false,
                        };
                        (binding, args)
                    }
                    _ => return false,
                };
            return self
                .thir
                .binding_names
                .get(binding.0 as usize)
                .is_some_and(|name| name == ctor)
                && concrete_args.len() == args.len() + remaining
                && args.iter().zip(concrete_args.iter()).all(|(pattern, ty)| {
                    self.match_witness_pattern(pattern, *ty, cenv, holes, depth + 1)
                });
        }
        let no_holes = FxHashSet::default();
        let (concrete, cenv) = self.norm_ty(concrete, cenv, &no_holes);
        let c_kind = self.thir.type_arena[concrete.0 as usize].kind.clone();
        match (pat, c_kind) {
            (WitnessPattern::Leaf(key), _) => {
                self.structural_witness_key(concrete, &mut FxHashSet::default())
                    .as_deref()
                    == Some(key.as_str())
            }
            (WitnessPattern::List(p), TypeKind::List(c)) => {
                self.match_witness_pattern(p, c, &cenv, holes, depth + 1)
            }
            (WitnessPattern::Optional(p), TypeKind::Optional(c)) => {
                self.match_witness_pattern(p, c, &cenv, holes, depth + 1)
            }
            (WitnessPattern::Maybe(p), TypeKind::Maybe(c)) => {
                self.match_witness_pattern(p, c, &cenv, holes, depth + 1)
            }
            (WitnessPattern::Record(pf), TypeKind::Record(cf, RowTail::Closed)) => {
                pf.len() == cf.len()
                    && pf.iter().all(|pfield| {
                        cf.iter().any(|cfield| {
                            cfield.name == pfield.name
                                && cfield.optional == pfield.optional
                                && self.match_witness_pattern(
                                    &pfield.ty,
                                    cfield.ty,
                                    &cenv,
                                    holes,
                                    depth + 1,
                                )
                        })
                    })
            }
            (WitnessPattern::Tuple(pi), TypeKind::Tuple(ci)) => {
                pi.len() == ci.len()
                    && pi.iter().zip(ci.iter()).all(|(p, c)| match (p, c) {
                        (
                            WitnessPatternTupleItem::Positional(pt),
                            TypeTupleItem::Positional(ct),
                        ) => self.match_witness_pattern(pt, *ct, &cenv, holes, depth + 1),
                        (
                            WitnessPatternTupleItem::Named { name: pn, ty: pt },
                            TypeTupleItem::Named {
                                name: cn, ty: ct, ..
                            },
                        ) => {
                            pn == cn && self.match_witness_pattern(pt, *ct, &cenv, holes, depth + 1)
                        }
                        _ => false,
                    })
            }
            (WitnessPattern::Union(pv), TypeKind::Union(cv, RowTail::Closed)) => {
                pv.len() == cv.len()
                    && pv.iter().all(|pvar| {
                        cv.iter().any(|cvar| {
                            cvar.name == pvar.name
                                && match (&pvar.payload, cvar.payload) {
                                    (Some(pp), Some(cp)) => {
                                        self.match_witness_pattern(pp, cp, &cenv, holes, depth + 1)
                                    }
                                    (None, None) => true,
                                    _ => false,
                                }
                        })
                    })
            }
            (WitnessPattern::Function(pf, pt), TypeKind::Function { from, to }) => {
                self.match_witness_pattern(pf, from, &cenv, holes, depth + 1)
                    && self.match_witness_pattern(pt, to, &cenv, holes, depth + 1)
            }
            _ => false,
        }
    }
}
