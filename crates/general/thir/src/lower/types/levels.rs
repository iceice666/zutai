use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn fresh_level_meta(&mut self) -> UniverseLevel {
        let id = self.next_level_meta;
        self.next_level_meta += 1;
        UniverseLevel::meta(id)
    }

    pub(in crate::lower) fn constrain_level_at_least(
        &mut self,
        level: UniverseLevel,
        lower: u32,
        span: Span,
    ) {
        match self.normalize_level(level) {
            UniverseLevel::Known(n) if n < lower => {
                self.push_universe_level_cycle("<anonymous>", span)
            }
            UniverseLevel::Meta(id) => {
                let entry = self.level_lower_bounds.entry(id).or_insert(0);
                *entry = (*entry).max(lower);
            }
            UniverseLevel::Max(levels) => {
                for level in levels {
                    self.constrain_level_at_least(level, lower, span);
                }
            }
            UniverseLevel::Succ(inner) if lower > 0 => {
                self.constrain_level_at_least(*inner, lower - 1, span);
            }
            _ => {}
        }
    }

    pub(in crate::lower) fn constrain_level_leq(
        &mut self,
        found: UniverseLevel,
        expected: UniverseLevel,
        span: Span,
    ) -> bool {
        let found = self.normalize_level(found);
        let expected = self.normalize_level(expected);
        if self.level_occurs_strictly_in(&expected, &found) {
            self.push_universe_level_cycle("<anonymous>", span);
            return false;
        }
        match (found, expected) {
            (UniverseLevel::Known(f), UniverseLevel::Known(e)) => f <= e,
            (UniverseLevel::Known(f), UniverseLevel::Meta(e)) => {
                self.constrain_level_at_least(UniverseLevel::Meta(e), f, span);
                true
            }
            (UniverseLevel::Meta(f), UniverseLevel::Known(e)) => {
                self.level_equalities.insert(f, UniverseLevel::Known(e));
                true
            }
            (UniverseLevel::Meta(f), rhs) => {
                if self.level_occurs_in(f, &rhs) {
                    self.push_universe_level_cycle("<anonymous>", span);
                    false
                } else {
                    self.level_equalities.insert(f, rhs);
                    true
                }
            }
            (UniverseLevel::Max(levels), rhs) => levels
                .into_iter()
                .all(|level| self.constrain_level_leq(level, rhs.clone(), span)),
            (UniverseLevel::Succ(inner), rhs) => {
                self.constrain_level_leq(*inner, UniverseLevel::succ(rhs), span)
            }
            (lhs, UniverseLevel::Max(levels)) => {
                let solved_lhs = self.default_level(lhs);
                let solved_rhs = levels
                    .into_iter()
                    .map(|level| self.default_level(level))
                    .max()
                    .unwrap_or(0);
                solved_lhs <= solved_rhs
            }
            (lhs, UniverseLevel::Succ(rhs)) => {
                let lhs = self.default_level(lhs);
                let rhs = self.default_level(*rhs);
                lhs <= rhs.saturating_add(1)
            }
        }
    }

    pub(in crate::lower) fn solve_level(
        &mut self,
        level: UniverseLevel,
        _span: Span,
    ) -> Option<u32> {
        Some(self.default_level(level))
    }

    pub(in crate::lower) fn default_level(&mut self, level: UniverseLevel) -> u32 {
        match self.normalize_level(level) {
            UniverseLevel::Known(n) => n,
            UniverseLevel::Meta(id) => *self.level_lower_bounds.get(&id).unwrap_or(&0),
            UniverseLevel::Max(levels) => levels
                .into_iter()
                .map(|level| self.default_level(level))
                .max()
                .unwrap_or(0),
            UniverseLevel::Succ(level) => self.default_level(*level).saturating_add(1),
        }
    }

    pub(in crate::lower) fn kind_compatible(
        &mut self,
        expected: &Kind,
        found: &Kind,
        span: Span,
    ) -> bool {
        match (expected, found) {
            (Kind::Type(expected), Kind::Type(found)) => {
                self.constrain_level_leq(found.clone(), expected.clone(), span)
            }
            (Kind::Row(expected), Kind::Row(found)) => self.kind_compatible(expected, found, span),
            (Kind::Arrow(exp_from, exp_to), Kind::Arrow(found_from, found_to)) => {
                self.kind_compatible(exp_from, found_from, span)
                    && self.kind_compatible(exp_to, found_to, span)
            }
            _ => false,
        }
    }

    pub(in crate::lower) fn type_universe(&mut self, ty: TypeId, span: Span) -> UniverseLevel {
        let ty = self.resolve(ty);
        if let Some(level) = self.type_universe_cache.get(&ty).cloned() {
            return level;
        }
        self.type_universe_cache.insert(ty, UniverseLevel::Known(0));
        let level = match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Type(level) => UniverseLevel::succ(level),
            TypeKind::Bool
            | TypeKind::Text
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::FixedNum(_)
            | TypeKind::Posit(_)
            | TypeKind::Opaque(_)
            | TypeKind::Atom(_)
            | TypeKind::True
            | TypeKind::False
            | TypeKind::Never => UniverseLevel::Known(0),
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Patch { target: inner, .. } => self.type_universe(inner, span),
            TypeKind::Record(fields, _) => UniverseLevel::max(
                fields
                    .into_iter()
                    .map(|field| self.type_universe(field.ty, field.span)),
            ),
            TypeKind::Union(variants, _) => UniverseLevel::max(
                variants
                    .into_iter()
                    .filter_map(|v| v.payload.map(|payload| self.type_universe(payload, v.span))),
            ),
            TypeKind::Tuple(items) => {
                UniverseLevel::max(items.into_iter().map(|item| match item {
                    TypeTupleItem::Named { ty, .. } | TypeTupleItem::Positional(ty) => {
                        self.type_universe(ty, span)
                    }
                }))
            }
            TypeKind::Function { from, to } => {
                UniverseLevel::max([self.type_universe(from, span), self.type_universe(to, span)])
            }
            TypeKind::Effect { base, row } => {
                let mut levels = vec![self.type_universe(base, span)];
                for op in row.ops {
                    levels.push(self.type_universe(op.param, op.span));
                    levels.push(self.type_universe(op.result, op.span));
                }
                UniverseLevel::max(levels)
            }
            TypeKind::TypeVar(binding) => self
                .type_param_kinds
                .get(&binding)
                .cloned()
                .and_then(|kind| match kind {
                    Kind::Type(level) => Some(level),
                    _ => None,
                })
                .unwrap_or_else(|| self.fresh_level_meta()),
            TypeKind::InferVar(v) => self
                .infer_subst
                .get(&v)
                .copied()
                .map(|solved| self.type_universe(solved, span))
                .unwrap_or(UniverseLevel::Known(0)),
            TypeKind::Alias(binding) => {
                if self
                    .alias_params
                    .get(&binding)
                    .is_some_and(|params| !params.is_empty())
                {
                    UniverseLevel::Known(0)
                } else if let Some(body) = self.aliases.get(&binding).copied() {
                    self.type_universe(body, span)
                } else {
                    UniverseLevel::Known(0)
                }
            }
            TypeKind::AliasApply { binding, args } => {
                self.alias_apply_universe(binding, &args, span)
            }
            TypeKind::ForAll { body, .. } => self.type_universe(body, span),
            TypeKind::Apply { .. } => self.apply_universe(ty, span),
            TypeKind::Con(_) | TypeKind::Error => UniverseLevel::Known(0),
        };
        self.type_universe_cache.insert(ty, level.clone());
        level
    }

    pub(in crate::lower) fn finalized_type_universes(&mut self) -> Vec<u32> {
        // Snapshot the arena length *before* iterating. Generic recursive aliases
        // (e.g. `Tree :: <A> type { #node : { left : Tree A; ... } }`) may call
        // `alias_apply_universe` → `instantiate_type_vars`, which allocates fresh
        // TypeId nodes.  Processing those fresh nodes would trigger further expansion,
        // making the `while i < self.type_arena.len()` loop infinite.  Nodes added
        // during universe computation are internal expansion artefacts; they are never
        // stored in user-visible IR positions, so their level is 0 (ground).
        let initial_len = self.type_arena.len();
        let mut universes = Vec::with_capacity(initial_len);
        let mut i = 0;
        while i < initial_len {
            let span = self.type_arena[i].span;
            let level = self.type_universe(TypeId(i as u32), span);
            universes.push(self.solve_level(level, span).unwrap_or(0));
            i += 1;
        }
        // Fill entries for any freshly-allocated nodes so
        // `type_universes.len() == type_arena.len()` holds.
        let extra = self.type_arena.len().saturating_sub(initial_len);
        universes.extend(std::iter::repeat_n(0u32, extra));
        universes
    }

    fn alias_apply_universe(
        &mut self,
        binding: BindingId,
        args: &[TypeId],
        span: Span,
    ) -> UniverseLevel {
        // Recursive guard: generic recursive aliases re-instantiate their bodies on
        // every call, minting fresh TypeIds that defeat the `type_universe_cache`
        // per-TypeId cycle break. On re-entry, use the applied args as a
        // conservative universe upper bound.
        if !self.alias_universe_in_progress.insert(binding) {
            // Conservative upper bound: max universe of the applied args.
            // Keep the binding-keyed guard (do NOT re-key by `(binding, args)`:
            // instantiate_type_vars mints fresh TypeIds per level, so an
            // `(binding, args)` key never repeats and would not terminate).
            // Collect first — a lazy `.map(|a| self.type_universe(..))` closure
            // plus a `.fold` closure would both borrow `&mut self` at once.
            let arg_levels: Vec<UniverseLevel> =
                args.iter().map(|&a| self.type_universe(a, span)).collect();
            return UniverseLevel::max(arg_levels);
        }
        let Some(params) = self.alias_params.get(&binding).cloned() else {
            self.alias_universe_in_progress.remove(&binding);
            return UniverseLevel::Known(0);
        };
        if params.len() != args.len() {
            self.alias_universe_in_progress.remove(&binding);
            return UniverseLevel::Known(0);
        }
        let Some(body) = self.aliases.get(&binding).copied() else {
            self.alias_universe_in_progress.remove(&binding);
            return UniverseLevel::Known(0);
        };
        let subst: FxHashMap<BindingId, TypeId> =
            params.into_iter().zip(args.iter().copied()).collect();
        let expanded = self.instantiate_type_vars(body, &subst);
        let level = self.type_universe(expanded, span);
        self.alias_universe_in_progress.remove(&binding);
        level
    }

    fn apply_universe(&mut self, ty: TypeId, span: Span) -> UniverseLevel {
        let (head, args) = self.app_spine(ty);
        let head = self.resolve(head);
        match self.type_arena[head.0 as usize].kind.clone() {
            TypeKind::Con(b) => {
                let name = self.hir.bindings[b.0 as usize].name.as_str();
                if args.len() == 1
                    && matches!(name, "List" | "Optional" | "Maybe" | "Patch" | "DeepPatch")
                {
                    self.type_universe(args[0], span)
                } else {
                    UniverseLevel::Known(0)
                }
            }
            TypeKind::Alias(binding) => self.alias_apply_universe(binding, &args, span),
            _ => match self.kind_of(ty, span) {
                Kind::Type(level) => level,
                _ => UniverseLevel::Known(0),
            },
        }
    }

    fn normalize_level(&self, level: UniverseLevel) -> UniverseLevel {
        match level {
            UniverseLevel::Meta(id) => self
                .level_equalities
                .get(&id)
                .cloned()
                .map(|level| self.normalize_level(level))
                .unwrap_or(UniverseLevel::Meta(id)),
            UniverseLevel::Max(levels) => {
                UniverseLevel::max(levels.into_iter().map(|level| self.normalize_level(level)))
            }
            UniverseLevel::Succ(level) => UniverseLevel::succ(self.normalize_level(*level)),
            other => other,
        }
    }

    fn level_occurs_in(&self, needle: u32, level: &UniverseLevel) -> bool {
        match level {
            UniverseLevel::Meta(id) => *id == needle,
            UniverseLevel::Max(levels) => levels
                .iter()
                .any(|level| self.level_occurs_in(needle, level)),
            UniverseLevel::Succ(level) => self.level_occurs_in(needle, level),
            UniverseLevel::Known(_) => false,
        }
    }

    fn level_occurs_strictly_in(&self, expected: &UniverseLevel, found: &UniverseLevel) -> bool {
        match (expected, found) {
            (UniverseLevel::Meta(e), UniverseLevel::Succ(inner)) => self.level_occurs_in(*e, inner),
            (UniverseLevel::Meta(e), level) => self.level_occurs_in(*e, level),
            (_, UniverseLevel::Max(levels)) => levels
                .iter()
                .any(|level| self.level_occurs_strictly_in(expected, level)),
            (_, UniverseLevel::Succ(inner)) => self.level_occurs_strictly_in(expected, inner),
            _ => false,
        }
    }

    pub(in crate::lower) fn finalized_kind(&mut self, kind: Kind) -> Kind {
        match kind {
            Kind::Type(level) => Kind::Type(UniverseLevel::Known(self.default_level(level))),
            Kind::Row(inner) => Kind::Row(Box::new(self.finalized_kind(*inner))),
            Kind::Arrow(from, to) => Kind::Arrow(
                Box::new(self.finalized_kind(*from)),
                Box::new(self.finalized_kind(*to)),
            ),
        }
    }

    /// Enforce that a type-value's universe (`found`) fits within an annotated
    /// universe (`expected`), with cumulativity. Registers the level constraint
    /// (so inferred bare `Type` levels default correctly and nothing well-founded
    /// is newly rejected) and, when both sides are fully concrete, emits
    /// `ExplicitLevelTooLow` on a definitive violation (`$0 = $0`: `1 ≤ 0`).
    pub(in crate::lower) fn check_universe_fits(
        &mut self,
        found: UniverseLevel,
        expected: UniverseLevel,
        span: Span,
    ) {
        // Skip the solver when the bound is provably satisfied (notably the same
        // shared meta from a `<$l>` binder used as `$l`/`$(l + n)`/`$(max …)`).
        // `constrain_level_leq`'s occurs-check would otherwise mis-flag `L ≤ L`
        // and `L ≤ Succ(L)` as cycles — both are trivially true by monotonicity.
        if self.level_le_trivial(&found, &expected) {
            return;
        }
        self.constrain_level_leq(found.clone(), expected.clone(), span);
        // The solver's Succ/Max arms are deliberately permissive on ground levels;
        // a concrete too-low universe must be caught by comparing solved values.
        if self.level_is_ground(&found) && self.level_is_ground(&expected) {
            let found_level = self.default_level(found);
            let required = self.default_level(expected);
            if found_level > required {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::ExplicitLevelTooLow {
                        required,
                        found: found_level,
                    },
                    span,
                });
            }
        }
    }

    /// `found ≤ expected` is provable without solving: either the levels are
    /// equal, or `found` is a metavariable that appears inside `expected` (which
    /// can only make it larger via `Succ`/`Max`).
    fn level_le_trivial(&self, found: &UniverseLevel, expected: &UniverseLevel) -> bool {
        let found = self.normalize_level(found.clone());
        let expected = self.normalize_level(expected.clone());
        if found == expected {
            return true;
        }
        matches!(found, UniverseLevel::Meta(id) if self.level_occurs_in(id, &expected))
    }

    /// A level is ground when, after normalization, it contains no metavariable.
    fn level_is_ground(&self, level: &UniverseLevel) -> bool {
        match self.normalize_level(level.clone()) {
            UniverseLevel::Known(_) => true,
            UniverseLevel::Meta(_) => false,
            UniverseLevel::Succ(inner) => self.level_is_ground(&inner),
            UniverseLevel::Max(levels) => levels.iter().all(|l| self.level_is_ground(l)),
        }
    }

    pub(in crate::lower) fn push_universe_level_cycle(&mut self, name: &str, span: Span) {
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::UniverseLevelCycle {
                name: name.to_string(),
            },
            span,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn universe_circular_definition_reports_kind_diagnostic() {
        let parsed = zutai_syntax::parse("1");
        let hir = zutai_hir::lower_file_with_preludes(
            parsed.ast().expect("parse should produce AST"),
            zutai_hir::HirLowerOptions::default(),
            zutai_hir::SourcePreludes {
                stream: Some(include_str!(concat!(
                    env!("ZUTAI_STDLIB_ROOT"),
                    "/modules/stream.zt"
                ))),
                prelude: Some(include_str!(concat!(
                    env!("ZUTAI_STDLIB_ROOT"),
                    "/modules/prelude.zt"
                ))),
            },
        );
        let mut lowerer = Lowerer::new(&hir.file, FxHashMap::default());
        let level = lowerer.fresh_level_meta();

        assert!(!lowerer.constrain_level_leq(
            UniverseLevel::succ(level.clone()),
            level,
            Span::default(),
        ));
        assert!(lowerer.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.kind,
            ThirDiagnosticKind::UniverseLevelCycle { .. }
        )));
    }
}
