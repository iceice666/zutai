use super::*;

impl<'hir> Lowerer<'hir> {
    // ── Inference / unification ──────────────────────────────────────────────

    pub(in crate::lower) fn fresh_infer_var(&mut self, span: Span) -> TypeId {
        let id = self.next_infer_var;
        self.next_infer_var += 1;
        self.alloc_type(Type {
            kind: TypeKind::InferVar(id),
            span,
        })
    }

    /// Mint a fresh flexible row variable.
    pub(in crate::lower) fn fresh_row_var(&mut self) -> RowTail {
        let id = self.next_row_var;
        self.next_row_var += 1;
        RowTail::Infer(id)
    }

    /// Flatten a record row `(fields, tail)` by appending every solved flexible
    /// tail's captured fields until the tail is rigid or unsolved.
    pub(in crate::lower) fn flatten_record_row(
        &self,
        mut fields: Vec<TypeRecordField>,
        mut tail: RowTail,
    ) -> (Vec<TypeRecordField>, RowTail) {
        while let RowTail::Infer(r) = tail {
            match self.row_subst.get(&r) {
                Some(RowSolution::Record {
                    fields: extra,
                    tail: next,
                }) => {
                    fields.extend(extra.iter().cloned());
                    tail = *next;
                }
                _ => break,
            }
        }
        (fields, tail)
    }

    /// Flatten a union row, analogous to `flatten_record_row`.
    pub(in crate::lower) fn flatten_union_row(
        &self,
        mut variants: Vec<UnionVariant>,
        mut tail: RowTail,
    ) -> (Vec<UnionVariant>, RowTail) {
        while let RowTail::Infer(r) = tail {
            match self.row_subst.get(&r) {
                Some(RowSolution::Union {
                    variants: extra,
                    tail: next,
                }) => {
                    variants.extend(extra.iter().cloned());
                    tail = *next;
                }
                _ => break,
            }
        }
        (variants, tail)
    }

    /// Flatten an effect row `(ops, tail)` by appending every solved flexible
    /// tail's captured ops until the tail is rigid or unsolved. The dual of
    /// `flatten_record_row`/`flatten_union_row` for effect rows.
    pub(in crate::lower) fn flatten_effect_row(
        &self,
        mut ops: Vec<EffectOp>,
        mut tail: RowTail,
    ) -> (Vec<EffectOp>, RowTail) {
        while let RowTail::Infer(r) = tail {
            match self.row_subst.get(&r) {
                Some(RowSolution::Effect {
                    ops: extra,
                    tail: next,
                }) => {
                    ops.extend(extra.iter().cloned());
                    tail = *next;
                }
                _ => break,
            }
        }
        (ops, tail)
    }

    /// Chase InferVar substitution chains to find the canonical representative.
    pub(in crate::lower) fn resolve(&self, ty: TypeId) -> TypeId {
        let mut current = ty;
        loop {
            match self.type_arena[current.0 as usize].kind {
                TypeKind::InferVar(v) => {
                    if let Some(&next) = self.infer_subst.get(&v) {
                        current = next;
                    } else {
                        return current;
                    }
                }
                _ => return current,
            }
        }
    }

    /// Occurs check: true if `var_id` appears free in `ty`.
    pub(in crate::lower) fn occurs(&self, var_id: u32, ty: TypeId) -> bool {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::InferVar(v) => v == var_id,
            TypeKind::Function { from, to } => self.occurs(var_id, from) || self.occurs(var_id, to),
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Code(inner)
            | TypeKind::Patch { target: inner, .. } => self.occurs(var_id, inner),
            TypeKind::Union(variants, _) => variants
                .iter()
                .any(|v| v.payload.is_some_and(|p| self.occurs(var_id, p))),
            TypeKind::Tuple(items) => items.iter().any(|item| {
                let inner = match item {
                    TypeTupleItem::Named { ty, .. } => *ty,
                    TypeTupleItem::Positional(ty) => *ty,
                };
                self.occurs(var_id, inner)
            }),
            TypeKind::Record(fields, _) => fields.iter().any(|f| self.occurs(var_id, f.ty)),
            TypeKind::Effect { base, row } => {
                self.occurs(var_id, base)
                    || row
                        .ops
                        .iter()
                        .any(|op| self.occurs(var_id, op.param) || self.occurs(var_id, op.result))
            }
            TypeKind::ForAll { body, .. } => self.occurs(var_id, body),
            TypeKind::Apply { func, arg } => self.occurs(var_id, func) || self.occurs(var_id, arg),
            _ => false,
        }
    }

    /// Structural unification of two types.  Solves InferVars in `infer_subst`.
    /// Reports a `TypeMismatch` diagnostic for rigid conflicts.
    pub(in crate::lower) fn unify(&mut self, t1: TypeId, t2: TypeId, span: Span) {
        self.unify_inner(t1, t2, span, &mut FxHashSet::default());
    }

    fn unify_inner(
        &mut self,
        t1: TypeId,
        t2: TypeId,
        span: Span,
        seen_alias_pairs: &mut FxHashSet<(BindingId, BindingId)>,
    ) {
        let t1 = self.resolve(t1);
        let t2 = self.resolve(t2);
        if t1 == t2 {
            return;
        }

        let k1 = self.type_arena[t1.0 as usize].kind.clone();
        let k2 = self.type_arena[t2.0 as usize].kind.clone();

        match (k1, k2) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => {}
            (TypeKind::Never, _) | (_, TypeKind::Never) => {}

            (TypeKind::InferVar(v), _) => {
                if self.occurs(v, t2) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::InfiniteType,
                        span,
                    });
                } else {
                    self.infer_subst.insert(v, t2);
                }
            }

            (_, TypeKind::InferVar(v)) => {
                if self.occurs(v, t1) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::InfiniteType,
                        span,
                    });
                } else {
                    self.infer_subst.insert(v, t1);
                }
            }

            (TypeKind::Alias(b1), TypeKind::Alias(b2)) if b1 == b2 => {}

            (TypeKind::Alias(binding), _) => {
                if !seen_alias_pairs.insert((binding, binding)) {
                    return;
                }
                match self.aliases.get(&binding).copied() {
                    Some(body) => self.unify_inner(body, t2, span, seen_alias_pairs),
                    None => self.type_mismatch(t1, t2, span),
                }
            }

            (_, TypeKind::Alias(binding)) => {
                if !seen_alias_pairs.insert((binding, binding)) {
                    return;
                }
                match self.aliases.get(&binding).copied() {
                    Some(body) => self.unify_inner(t1, body, span, seen_alias_pairs),
                    None => self.type_mismatch(t1, t2, span),
                }
            }

            (TypeKind::Function { from: f1, to: r1 }, TypeKind::Function { from: f2, to: r2 }) => {
                self.unify_inner(f1, f2, span, seen_alias_pairs);
                self.unify_inner(r1, r2, span, seen_alias_pairs);
            }

            (TypeKind::Effect { base: b1, row: r1 }, TypeKind::Effect { base: b2, row: r2 }) => {
                self.unify_inner(b1, b2, span, seen_alias_pairs);
                self.effect_rows_unify(&r1, &r2, span);
            }

            (TypeKind::List(e1), TypeKind::List(e2)) => {
                self.unify_inner(e1, e2, span, seen_alias_pairs)
            }

            (TypeKind::Optional(e1), TypeKind::Optional(e2)) => {
                self.unify_inner(e1, e2, span, seen_alias_pairs)
            }
            (TypeKind::Maybe(e1), TypeKind::Maybe(e2)) => {
                self.unify_inner(e1, e2, span, seen_alias_pairs)
            }
            (TypeKind::Code(e1), TypeKind::Code(e2)) => {
                self.unify_inner(e1, e2, span, seen_alias_pairs)
            }
            (
                TypeKind::Patch {
                    target: t1,
                    deep: d1,
                },
                TypeKind::Patch {
                    target: t2,
                    deep: d2,
                },
            ) if d1 == d2 => self.unify_inner(t1, t2, span, seen_alias_pairs),

            (TypeKind::Tuple(items1), TypeKind::Tuple(items2)) => {
                if items1.len() != items2.len() {
                    self.type_mismatch(t1, t2, span);
                    return;
                }
                for (left, right) in items1.iter().zip(items2.iter()) {
                    match (left, right) {
                        (
                            TypeTupleItem::Named {
                                name: n1, ty: t1, ..
                            },
                            TypeTupleItem::Named {
                                name: n2, ty: t2, ..
                            },
                        ) if n1 == n2 => self.unify_inner(*t1, *t2, span, seen_alias_pairs),
                        (TypeTupleItem::Positional(t1), TypeTupleItem::Positional(t2)) => {
                            self.unify_inner(*t1, *t2, span, seen_alias_pairs);
                        }
                        _ => {
                            self.type_mismatch(t1, t2, span);
                            return;
                        }
                    }
                }
            }

            (TypeKind::Record(fields1, tail1), TypeKind::Record(fields2, tail2)) => {
                let (fields1, tail1) = self.flatten_record_row(fields1, tail1);
                let (fields2, tail2) = self.flatten_record_row(fields2, tail2);
                if tail1 != tail2 || fields1.len() != fields2.len() {
                    self.type_mismatch(t1, t2, span);
                    return;
                }
                let fields2_by_name: FxHashMap<&str, &TypeRecordField> = fields2
                    .iter()
                    .map(|field| (field.name.as_str(), field))
                    .collect();
                for field1 in &fields1 {
                    let Some(field2) = fields2_by_name.get(field1.name.as_str()) else {
                        self.type_mismatch(t1, t2, span);
                        return;
                    };
                    if field1.optional != field2.optional {
                        self.type_mismatch(t1, t2, span);
                        return;
                    }
                    self.unify_inner(field1.ty, field2.ty, span, seen_alias_pairs);
                }
            }

            (TypeKind::Union(vars1, tail1), TypeKind::Union(vars2, tail2)) => {
                let (vars1, tail1) = self.flatten_union_row(vars1, tail1);
                let (vars2, tail2) = self.flatten_union_row(vars2, tail2);
                if tail1 != tail2 || vars1.len() != vars2.len() {
                    self.type_mismatch(t1, t2, span);
                    return;
                }
                let vars2_by_name: FxHashMap<&str, &UnionVariant> = vars2
                    .iter()
                    .map(|variant| (variant.name.as_str(), variant))
                    .collect();
                for var1 in &vars1 {
                    let Some(var2) = vars2_by_name.get(var1.name.as_str()) else {
                        self.type_mismatch(t1, t2, span);
                        return;
                    };
                    match (var1.payload, var2.payload) {
                        (Some(left), Some(right)) => {
                            self.unify_inner(left, right, span, seen_alias_pairs);
                        }
                        (None, None) => {}
                        _ => {
                            self.type_mismatch(t1, t2, span);
                            return;
                        }
                    }
                }
            }

            // Equirecursive aliases are transparent, but comparing two distinct
            // recursive aliases by repeatedly expanding both sides can consume the
            // global type-level fuel forever (`ambient Stream A` versus imported
            // `s.Stream A`). Remember alias-head pairs coinductively: once the
            // same pair is seen below itself, the recursive back-edge has matched.
            (
                TypeKind::AliasApply {
                    binding: b1,
                    args: a1,
                },
                TypeKind::AliasApply {
                    binding: b2,
                    args: a2,
                },
            ) => {
                if a1.len() != a2.len() {
                    self.type_mismatch(t1, t2, span);
                    return;
                }
                for (left, right) in a1.iter().zip(a2.iter()) {
                    self.unify_inner(*left, *right, span, seen_alias_pairs);
                }
                if b1 == b2 {
                    return;
                }

                let key = if b1.0 <= b2.0 { (b1, b2) } else { (b2, b1) };
                if !seen_alias_pairs.insert(key) {
                    return;
                }

                match (
                    self.expand_alias_apply_once(b1, &a1, span),
                    self.expand_alias_apply_once(b2, &a2, span),
                ) {
                    (Some(left), Some(right)) => {
                        self.unify_inner(left, right, span, seen_alias_pairs);
                    }
                    _ => self.type_mismatch(t1, t2, span),
                }
            }

            // Higher-kinded application: decompose head and argument. Required so
            // method-level / constraint type params solve when unifying `F A`
            // shapes (`F A ~ F B`, `?f A ~ F A`). Structural `!=` would spuriously
            // mismatch two separately-built but equal `Apply` nodes.
            (TypeKind::Apply { func: f1, arg: a1 }, TypeKind::Apply { func: f2, arg: a2 }) => {
                self.unify_inner(f1, f2, span, seen_alias_pairs);
                self.unify_inner(a1, a2, span, seen_alias_pairs);
            }

            // First-order constructor inference: when an abstract constructor
            // application meets a saturated concrete constructor, solve the
            // abstract head and then compare the application arguments. This is
            // the Miller-pattern case needed by `F A ~ List B`; the kind checker
            // has already constrained `F` to `Type -> Type`, and we intentionally
            // do not synthesize higher-order constructor functions here.
            (TypeKind::Apply { func, arg }, concrete)
                if matches!(
                    self.type_arena[self.resolve(func).0 as usize].kind,
                    TypeKind::InferVar(_)
                ) && self.unify_abstract_constructor_apply(
                    func,
                    arg,
                    t2,
                    &concrete,
                    span,
                    seen_alias_pairs,
                ) => {}

            (concrete, TypeKind::Apply { func, arg })
                if matches!(
                    self.type_arena[self.resolve(func).0 as usize].kind,
                    TypeKind::InferVar(_)
                ) && self.unify_abstract_constructor_apply(
                    func,
                    arg,
                    t1,
                    &concrete,
                    span,
                    seen_alias_pairs,
                ) => {}

            (left, right) => {
                // Cross-form applications: canonicalize via `resolve_alias` (folds
                // builtin `Con` apps and expands saturated named-alias apps) so a
                // saturated `Apply`/`AliasApply` meets its concrete form. Only retry
                // when reduction made progress, else fall through to mismatch.
                let app_like = |k: &TypeKind| {
                    matches!(
                        k,
                        TypeKind::Apply { .. } | TypeKind::AliasApply { .. } | TypeKind::Con(_)
                    )
                };
                if app_like(&left) || app_like(&right) {
                    let r1 = self.resolve_alias(t1, &mut FxHashSet::default(), span);
                    let r2 = self.resolve_alias(t2, &mut FxHashSet::default(), span);
                    if r1 != t1 || r2 != t2 {
                        self.unify_inner(r1, r2, span, seen_alias_pairs);
                        return;
                    }
                }
                // NOTE: an abstract-headed application against a concrete
                // constructor (`Apply{?f, X} ~ List Y`) is intentionally *not*
                // bridged here (would need Miller-pattern `?f := Con(List)` then
                // `unify(X, Y)`). Concrete higher-kinded application is outside the
                // Phase 14 gate and a refused check is the safe direction; the arm
                // belongs to the later concrete-HKT-dispatch milestone.
                if left != right {
                    self.type_mismatch(t1, t2, span);
                }
            }
        }
    }

    /// Solve the first-order constructor metavariable in `Apply(?f, arg)` when
    /// the other side is a saturated unary constructor such as `List item`.
    /// Returns `false` for non-constructor shapes so ordinary mismatch handling
    /// remains authoritative.
    fn unify_abstract_constructor_apply(
        &mut self,
        func: TypeId,
        arg: TypeId,
        concrete_ty: TypeId,
        concrete: &TypeKind,
        span: Span,
        seen_alias_pairs: &mut FxHashSet<(BindingId, BindingId)>,
    ) -> bool {
        let (constructor, concrete_arg) = match concrete {
            TypeKind::List(item) => (self.builtin_constructor("List", span), *item),
            TypeKind::Optional(item) => (self.builtin_constructor("Optional", span), *item),
            TypeKind::Maybe(item) => (self.builtin_constructor("Maybe", span), *item),
            TypeKind::Code(item) => (self.builtin_constructor("Code", span), *item),
            TypeKind::Patch {
                target,
                deep: false,
            } => (self.builtin_constructor("Patch", span), *target),
            TypeKind::Patch { target, deep: true } => {
                (self.builtin_constructor("DeepPatch", span), *target)
            }
            TypeKind::AliasApply { binding, args } if !args.is_empty() => {
                let constructor =
                    self.partial_alias_constructor(*binding, &args[..args.len() - 1], span);
                (Some(constructor), args[args.len() - 1])
            }
            _ => return false,
        };
        let Some(constructor) = constructor else {
            return false;
        };
        self.unify_inner(func, constructor, span, seen_alias_pairs);
        self.unify_inner(arg, concrete_arg, span, seen_alias_pairs);

        // Preserve the concrete type's identity for diagnostics and downstream
        // zonking; the two component unifications above carry the real solving.
        let _ = concrete_ty;
        true
    }

    pub(in crate::lower) fn builtin_constructor(
        &mut self,
        name: &str,
        span: Span,
    ) -> Option<TypeId> {
        let binding = self
            .hir
            .bindings
            .iter()
            .position(|binding| binding.kind == BindingKind::BuiltinType && binding.name == name)
            .map(|index| BindingId(index as u32))?;
        Some(self.alloc_type(Type {
            kind: TypeKind::Con(binding),
            span,
        }))
    }

    pub(in crate::lower) fn partial_alias_constructor(
        &mut self,
        binding: BindingId,
        prefix: &[TypeId],
        span: Span,
    ) -> TypeId {
        let head = self.alloc_type(Type {
            kind: TypeKind::Alias(binding),
            span,
        });
        self.fold_apply(head, prefix, span)
    }

    pub(in crate::lower) fn expand_alias_apply_once(
        &mut self,
        binding: BindingId,
        args: &[TypeId],
        span: Span,
    ) -> Option<TypeId> {
        if self.type_eval_fuel == 0 {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TypeLevelEvalLimitExceeded,
                span,
            });
            return Some(self.error_type);
        }
        self.type_eval_fuel -= 1;
        let params = self.alias_params.get(&binding).cloned()?;
        if params.len() != args.len() {
            return None;
        }
        let body = self.aliases.get(&binding).copied()?;
        let subst: FxHashMap<BindingId, TypeId> =
            params.into_iter().zip(args.iter().copied()).collect();
        Some(self.instantiate_type_vars(body, &subst))
    }

    /// Zonk: for every solved InferVar slot in the type arena, overwrite it
    /// with the kind of its resolved type so callers see concrete types without
    /// having to chase substitution chains.
    pub(in crate::lower) fn zonk_type_arena(&mut self) {
        for i in 0..self.type_arena.len() {
            if matches!(self.type_arena[i].kind, TypeKind::InferVar(_)) {
                let resolved = self.resolve(TypeId(i as u32));
                if resolved.0 as usize != i {
                    let resolved_kind = self.type_arena[resolved.0 as usize].kind.clone();
                    self.type_arena[i].kind = resolved_kind;
                }
            }
        }
        // Flatten solved flexible row tails in record/union types so consumers
        // see the captured fields/members inline with a rigid residual tail.
        for i in 0..self.type_arena.len() {
            match self.type_arena[i].kind.clone() {
                TypeKind::Record(fields, tail @ RowTail::Infer(_)) => {
                    let (fields, tail) = self.flatten_record_row(fields, tail);
                    self.type_arena[i].kind = TypeKind::Record(fields, tail);
                }
                TypeKind::Union(variants, tail @ RowTail::Infer(_)) => {
                    let (variants, tail) = self.flatten_union_row(variants, tail);
                    self.type_arena[i].kind = TypeKind::Union(variants, tail);
                }
                TypeKind::Effect {
                    base,
                    row:
                        EffectRow {
                            ops,
                            tail: tail @ RowTail::Infer(_),
                        },
                } => {
                    let (ops, tail) = self.flatten_effect_row(ops, tail);
                    self.type_arena[i].kind = TypeKind::Effect {
                        base,
                        row: EffectRow { ops, tail },
                    };
                }
                _ => {}
            }
        }
    }
}
