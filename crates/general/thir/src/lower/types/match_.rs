use super::alias::row_tail_key;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::lower) enum WrapperKind {
    Optional,
    Maybe,
}

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn record_fields(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<Vec<TypeRecordField>> {
        // Flatten any solved flexible tail so fields captured by a named row tail
        // (e.g. the result of a row-polymorphic call) are visible before zonking.
        self.record_row(ty, span).map(|(fields, _)| fields)
    }

    /// Like `record_fields` but also returns the row tail, with any solved
    /// flexible tail flattened in. Used by record checking to honour open rows.
    pub(in crate::lower) fn record_row(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<(Vec<TypeRecordField>, RowTail)> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind.clone() {
            TypeKind::Record(fields, tail) => Some(self.flatten_record_row(fields, tail)),
            TypeKind::Patch { target, deep } => self.expand_patch_type(target, deep, span),
            _ => None,
        }
    }

    pub(in crate::lower) fn list_item_type(&mut self, ty: TypeId, span: Span) -> Option<TypeId> {
        let alias_resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        let resolved = self.resolve(alias_resolved);
        match self.ty(resolved).kind {
            TypeKind::List(item) => Some(item),
            // For an unsolved InferVar, mint a fresh `List` and unify to bind it,
            // so a list literal checked against an as-yet-unknown type (e.g. a
            // constraint method's instantiated parameter) infers `List <item>`
            // instead of failing with `ExpectedList`.
            TypeKind::InferVar(_) => {
                let item = self.fresh_infer_var(span);
                let list = self.alloc_type(Type {
                    kind: TypeKind::List(item),
                    span,
                });
                self.unify(resolved, list, span);
                Some(item)
            }
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(in crate::lower) fn optional_inner_type(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<TypeId> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind {
            TypeKind::Optional(inner) => Some(inner),
            _ => None,
        }
    }

    pub(in crate::lower) fn optional_or_maybe_inner_type(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<(WrapperKind, TypeId)> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind {
            TypeKind::Optional(inner) => Some((WrapperKind::Optional, inner)),
            TypeKind::Maybe(inner) => Some((WrapperKind::Maybe, inner)),
            _ => None,
        }
    }

    pub(in crate::lower) fn function_input_output(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<(TypeId, TypeId)> {
        // First resolve named aliases, then chase any InferVar substitutions.
        let alias_resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        let resolved = self.resolve(alias_resolved);
        match self.ty(resolved).kind {
            TypeKind::Function { from, to } => Some((from, to)),
            // For an unsolved InferVar, mint a fresh arrow and unify to bind it.
            TypeKind::InferVar(_) => {
                let from = self.fresh_infer_var(span);
                let to = self.fresh_infer_var(span);
                let arrow = self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span,
                });
                self.unify(resolved, arrow, span);
                Some((from, to))
            }
            _ => None,
        }
    }

    pub(in crate::lower) fn function_parts(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> (Vec<TypeId>, TypeId) {
        let mut params = Vec::new();
        let mut current = ty;
        loop {
            let resolved = self.resolve_alias(current, &mut HashSet::new(), span);
            match self.ty(resolved).kind {
                TypeKind::Function { from, to } => {
                    params.push(from);
                    current = to;
                }
                _ => return (params, resolved),
            }
        }
    }

    pub(in crate::lower) fn type_matches(&mut self, expected: TypeId, found: TypeId) -> bool {
        let e_span = self.type_arena[expected.0 as usize].span;
        let f_span = self.type_arena[found.0 as usize].span;
        let expected_head = self.resolve(expected);
        let found_head = self.resolve(found);
        let head_kinds = (
            self.type_arena[expected_head.0 as usize].kind.clone(),
            self.type_arena[found_head.0 as usize].kind.clone(),
        );
        if expected_head == found_head
            && !matches!(
                head_kinds.0,
                TypeKind::Alias(_) | TypeKind::AliasApply { .. }
            )
        {
            return true;
        }
        let guard_key = if matches!(
            head_kinds.0,
            TypeKind::Alias(_) | TypeKind::AliasApply { .. }
        ) && matches!(
            head_kinds.1,
            TypeKind::Alias(_) | TypeKind::AliasApply { .. }
        ) {
            let key = (expected_head, found_head);
            if !self.type_match_in_progress.insert(key) {
                return true;
            }
            Some(key)
        } else {
            None
        };
        match head_kinds {
            (
                TypeKind::AliasApply {
                    binding: eb,
                    args: ea,
                },
                TypeKind::AliasApply {
                    binding: fb,
                    args: fa,
                },
            ) if eb == fb && ea.len() == fa.len() && self.alias_is_recursive(eb) => {
                let result = ea
                    .iter()
                    .zip(fa.iter())
                    .all(|(&ea, &fa)| self.type_matches(ea, fa) && self.type_matches(fa, ea));
                if let Some(key) = guard_key {
                    self.type_match_in_progress.remove(&key);
                }
                return result;
            }
            _ => {}
        }
        let expected = self.resolve_alias(expected, &mut HashSet::new(), e_span);
        let found = self.resolve_alias(found, &mut HashSet::new(), f_span);
        if expected == found {
            if let Some(key) = guard_key {
                self.type_match_in_progress.remove(&key);
            }
            return true;
        }

        let ek = self.type_arena[expected.0 as usize].kind.clone();
        let fk = self.type_arena[found.0 as usize].kind.clone();

        let result = match (ek, fk) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => true,
            (_, TypeKind::Never) => true,

            // Solve InferVars: if either side is an unsolved InferVar, unify
            // and treat as matching (errors emitted inside unify on conflicts).
            (TypeKind::InferVar(v), _) => {
                if self.occurs(v, found) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::InfiniteType,
                        span: e_span,
                    });
                } else {
                    self.infer_subst.insert(v, found);
                }
                true
            }
            (_, TypeKind::InferVar(v)) => {
                if self.occurs(v, expected) {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::InfiniteType,
                        span: f_span,
                    });
                } else {
                    self.infer_subst.insert(v, expected);
                }
                true
            }

            (TypeKind::Bool, TypeKind::True | TypeKind::False) => true,
            (TypeKind::Union(ev, et), TypeKind::Atom(ref name)) => {
                // Treat the atom as a singleton closed union so the row logic
                // decides membership: an explicit nullary member matches, and an
                // open/flexible tail absorbs (and captures) an extra member.
                let found = [UnionVariant {
                    name: name.clone(),
                    payload: None,
                    span: Span::default(),
                }];
                self.union_rows_match(&ev, et, &found, RowTail::Closed)
            }
            (TypeKind::Union(ev, et), TypeKind::Union(fv, ft)) => {
                if et == RowTail::Closed && ft == RowTail::Closed {
                    // Closed v0 unions match exactly (same members, same order).
                    ev.len() == fv.len()
                        && ev.iter().zip(fv.iter()).all(|(a, b)| {
                            a.name == b.name
                                && match (a.payload, b.payload) {
                                    (Some(pa), Some(pb)) => self.type_matches(pa, pb),
                                    (None, None) => true,
                                    _ => false,
                                }
                        })
                } else {
                    self.union_rows_match(&ev, et, &fv, ft)
                }
            }
            // #none is always a valid value of Optional(T); #absent is valid for Maybe(T).
            (TypeKind::Optional(_), TypeKind::Atom(ref name)) if name == "none" => true,
            (TypeKind::Maybe(_), TypeKind::Atom(ref name)) if name == "absent" => true,
            (TypeKind::List(e), TypeKind::List(f))
            | (TypeKind::Optional(e), TypeKind::Optional(f))
            | (TypeKind::Maybe(e), TypeKind::Maybe(f)) => self.type_matches(e, f),
            (
                TypeKind::Patch {
                    target: e,
                    deep: ed,
                },
                TypeKind::Patch {
                    target: f,
                    deep: fd,
                },
            ) => ed == fd && self.type_matches(e, f),
            (TypeKind::Patch { target, deep }, TypeKind::Record(ff, ft)) => self
                .expand_patch_type(target, deep, e_span)
                .is_some_and(|(ef, et)| self.record_rows_match(&ef, et, &ff, ft)),
            (TypeKind::Record(ef, et), TypeKind::Record(ff, ft)) => {
                self.record_rows_match(&ef, et, &ff, ft)
            }
            (TypeKind::Tuple(ei), TypeKind::Tuple(fi)) => self.tuple_types_match(&ei, &fi),
            (TypeKind::Function { from: ef, to: et }, TypeKind::Function { from: ff, to: ft }) => {
                // Parameters are contravariant, results covariant. Contravariance
                // is required for soundness now that records have width subtyping:
                // a function accepting an open record may stand in for one that
                // takes a wider closed record, but never the reverse.
                self.type_matches(ff, ef) && self.type_matches(et, ft)
            }
            (TypeKind::Effect { base: eb, row: er }, TypeKind::Effect { base: fb, row: fr }) => {
                self.effect_rows_match(&er, &fr) && self.type_matches(eb, fb)
            }
            (_, TypeKind::Effect { base, row }) => {
                self.discharge_row(&row, f_span);
                self.type_matches(expected, base)
            }
            // Higher-kinded application: match head and argument structurally,
            // solving infer vars on either side (both already alias-resolved).
            (TypeKind::Apply { func: ef, arg: ea }, TypeKind::Apply { func: ff, arg: fa }) => {
                self.type_matches(ef, ff) && self.type_matches(ea, fa)
            }
            (left, right) => left == right,
        };
        if let Some(key) = guard_key {
            self.type_match_in_progress.remove(&key);
        }
        result
    }

    fn alias_is_recursive(&mut self, binding: BindingId) -> bool {
        if let Some(&cached) = self.alias_recursive_cache.get(&binding) {
            return cached;
        }
        // Insert `false` first so a self-reference encountered during the walk
        // does not recurse back into this method (the walk uses its own visited
        // set, but this also guards re-entry through the cache).
        self.alias_recursive_cache.insert(binding, false);
        let result = match self.aliases.get(&binding).copied() {
            Some(body) => {
                let mut visited = HashSet::new();
                self.type_references_alias(body, binding, &mut visited)
            }
            None => false,
        };
        self.alias_recursive_cache.insert(binding, result);
        result
    }

    /// Does `ty` transitively reference alias `target` through alias/apply edges?
    /// Walks the alias-reference graph (following referenced bodies via
    /// `self.aliases`) with a binding `visited` set so mutual cycles terminate.
    fn type_references_alias(
        &self,
        ty: TypeId,
        target: BindingId,
        visited: &mut HashSet<BindingId>,
    ) -> bool {
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Alias(b) => {
                if b == target {
                    return true;
                }
                if visited.insert(b)
                    && let Some(body) = self.aliases.get(&b).copied()
                {
                    return self.type_references_alias(body, target, visited);
                }
                false
            }
            TypeKind::AliasApply { binding: b, args } => {
                if b == target {
                    return true;
                }
                if args
                    .iter()
                    .any(|&a| self.type_references_alias(a, target, visited))
                {
                    return true;
                }
                if visited.insert(b)
                    && let Some(body) = self.aliases.get(&b).copied()
                {
                    return self.type_references_alias(body, target, visited);
                }
                false
            }
            TypeKind::List(t)
            | TypeKind::Optional(t)
            | TypeKind::Maybe(t)
            | TypeKind::Patch { target: t, .. } => self.type_references_alias(t, target, visited),
            TypeKind::Function { from, to } => {
                self.type_references_alias(from, target, visited)
                    || self.type_references_alias(to, target, visited)
            }
            TypeKind::Apply { func, arg } => {
                self.type_references_alias(func, target, visited)
                    || self.type_references_alias(arg, target, visited)
            }
            TypeKind::Record(fields, _) => fields
                .iter()
                .any(|f| self.type_references_alias(f.ty, target, visited)),
            TypeKind::Union(variants, _) => variants
                .iter()
                .filter_map(|v| v.payload)
                .any(|p| self.type_references_alias(p, target, visited)),
            TypeKind::Tuple(items) => items.iter().any(|it| match it {
                TypeTupleItem::Named { ty, .. } | TypeTupleItem::Positional(ty) => {
                    self.type_references_alias(*ty, target, visited)
                }
            }),
            TypeKind::Effect { base, row } => {
                self.type_references_alias(base, target, visited)
                    || row.ops.iter().any(|op| {
                        self.type_references_alias(op.param, target, visited)
                            || self.type_references_alias(op.result, target, visited)
                    })
            }
            _ => false,
        }
    }

    /// Row-aware record assignability: `found` is assignable to `expected` when
    /// it provides every required field of `expected` (with matching types).
    /// Extra found fields are accepted only if `expected`'s tail is open: an
    /// anonymous tail discards them, a flexible row variable captures them, and a
    /// rigid tail requires the same variable with no extras.
    pub(in crate::lower) fn record_rows_match(
        &mut self,
        ef: &[TypeRecordField],
        et: RowTail,
        ff: &[TypeRecordField],
        ft: RowTail,
    ) -> bool {
        let (ef, et) = self.flatten_record_row(ef.to_vec(), et);
        let (ff, ft) = self.flatten_record_row(ff.to_vec(), ft);
        let found_by_name: HashMap<&str, &TypeRecordField> =
            ff.iter().map(|f| (f.name.as_str(), f)).collect();
        for e in &ef {
            match found_by_name.get(e.name.as_str()) {
                Some(f) => {
                    if !self.type_matches(e.ty, f.ty) {
                        return false;
                    }
                }
                None => {
                    if !e.optional {
                        return false;
                    }
                }
            }
        }
        let expected_names: HashSet<&str> = ef.iter().map(|f| f.name.as_str()).collect();
        let extras: Vec<TypeRecordField> = ff
            .iter()
            .filter(|f| !expected_names.contains(f.name.as_str()))
            .cloned()
            .collect();
        match et {
            RowTail::Closed => extras.is_empty() && ft == RowTail::Closed,
            RowTail::Open => true,
            RowTail::Param(p) => extras.is_empty() && ft == RowTail::Param(p),
            RowTail::Infer(r) => {
                if ft == RowTail::Infer(r) {
                    extras.is_empty()
                } else {
                    self.row_subst.insert(
                        r,
                        RowSolution::Record {
                            fields: extras,
                            tail: ft,
                        },
                    );
                    true
                }
            }
        }
    }

    /// Row-aware union assignability — the dual of `record_rows_match`. A value
    /// of union type `found` is assignable to `expected` when every member
    /// `found` may be is accounted for by `expected`: it either matches an
    /// explicit member (with matching payload) or is absorbed by `expected`'s
    /// tail (discarded by an anonymous tail, captured by a flexible row variable,
    /// rejected by a closed or rigid tail). Explicit `expected` members absent
    /// from `found` are fine — a handler may cover cases the value never takes.
    pub(in crate::lower) fn union_rows_match(
        &mut self,
        ev: &[UnionVariant],
        et: RowTail,
        fv: &[UnionVariant],
        ft: RowTail,
    ) -> bool {
        let (ev, et) = self.flatten_union_row(ev.to_vec(), et);
        let (fv, ft) = self.flatten_union_row(fv.to_vec(), ft);
        let expected_by_name: HashMap<&str, &UnionVariant> =
            ev.iter().map(|v| (v.name.as_str(), v)).collect();
        let mut extras: Vec<UnionVariant> = Vec::new();
        for f in &fv {
            match expected_by_name.get(f.name.as_str()) {
                Some(e) => match (e.payload, f.payload) {
                    (Some(pe), Some(pf)) => {
                        if !self.type_matches(pe, pf) {
                            return false;
                        }
                    }
                    (None, None) => {}
                    _ => return false,
                },
                None => extras.push(f.clone()),
            }
        }
        match et {
            RowTail::Closed => extras.is_empty() && ft == RowTail::Closed,
            RowTail::Open => true,
            RowTail::Param(p) => extras.is_empty() && ft == RowTail::Param(p),
            RowTail::Infer(r) => {
                if ft == RowTail::Infer(r) {
                    extras.is_empty()
                } else {
                    self.row_subst.insert(
                        r,
                        RowSolution::Union {
                            variants: extras,
                            tail: ft,
                        },
                    );
                    true
                }
            }
        }
    }

    pub(in crate::lower) fn effect_rows_unify(
        &mut self,
        expected: &EffectRow,
        found: &EffectRow,
        span: Span,
    ) {
        for op in &expected.ops {
            match found.find(&op.name) {
                Some(found_op) => {
                    self.unify(op.param, found_op.param, span);
                    self.unify(op.result, found_op.result, span);
                }
                None => {
                    self.type_mismatch_effect(expected, found, span);
                    return;
                }
            }
        }
        if found
            .ops
            .iter()
            .any(|found_op| expected.find(&found_op.name).is_none())
            || expected.tail != found.tail
        {
            self.type_mismatch_effect(expected, found, span);
        }
    }

    pub(in crate::lower) fn effect_rows_match(
        &mut self,
        expected: &EffectRow,
        found: &EffectRow,
    ) -> bool {
        if expected.tail != found.tail {
            return false;
        }
        for found_op in &found.ops {
            let Some(expected_op) = expected.find(&found_op.name) else {
                return false;
            };
            if !self.type_matches(found_op.param, expected_op.param)
                || !self.type_matches(expected_op.result, found_op.result)
            {
                return false;
            }
        }
        true
    }

    pub(in crate::lower) fn type_mismatch_effect(
        &mut self,
        expected: &EffectRow,
        found: &EffectRow,
        span: Span,
    ) {
        let expected = self.effect_row_name(expected);
        let found = self.effect_row_name(found);
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::TypeMismatch { expected, found },
            span,
        });
    }

    pub(in crate::lower) fn effect_row_name(&mut self, row: &EffectRow) -> String {
        if row.ops.is_empty() && row.tail == RowTail::Closed {
            return "{}".to_string();
        }
        let mut parts: Vec<String> = row
            .ops
            .iter()
            .map(|op| {
                format!(
                    "{}: {} -> {}",
                    op.name,
                    self.type_name(op.param),
                    self.type_name(op.result)
                )
            })
            .collect();
        if row.tail != RowTail::Closed {
            parts.push(row_tail_key(row.tail));
        }
        format!("{{{}}}", parts.join(", "))
    }

    pub(in crate::lower) fn tuple_types_match(
        &mut self,
        expected_items: &[TypeTupleItem],
        found_items: &[TypeTupleItem],
    ) -> bool {
        if expected_items.len() != found_items.len() {
            return false;
        }
        expected_items
            .iter()
            .zip(found_items)
            .all(|(expected, found)| match (expected, found) {
                (TypeTupleItem::Positional(expected), TypeTupleItem::Positional(found)) => {
                    self.type_matches(*expected, *found)
                }
                (
                    TypeTupleItem::Named {
                        name: expected_name,
                        ty: expected,
                        ..
                    },
                    TypeTupleItem::Named {
                        name: found_name,
                        ty: found,
                        ..
                    },
                ) if expected_name == found_name => self.type_matches(*expected, *found),
                _ => false,
            })
    }

    /// Flatten a curried `Apply` chain into its head and left-to-right argument
    /// list. Does not resolve aliases or infer vars or fold builtins — it is a
    /// pure structural walk. `F A B` → `(F, [A, B])`; a non-application → `(ty, [])`.
    pub(in crate::lower) fn app_spine(&self, ty: TypeId) -> (TypeId, Vec<TypeId>) {
        let mut args: Vec<TypeId> = Vec::new();
        let mut cur = ty;
        while let TypeKind::Apply { func, arg } = self.type_arena[cur.0 as usize].kind {
            args.push(arg);
            cur = func;
        }
        args.reverse();
        (cur, args)
    }
}
