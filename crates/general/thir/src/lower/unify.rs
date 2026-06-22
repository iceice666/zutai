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

            (TypeKind::Function { from: f1, to: r1 }, TypeKind::Function { from: f2, to: r2 }) => {
                self.unify(f1, f2, span);
                self.unify(r1, r2, span);
            }

            (TypeKind::Effect { base: b1, row: r1 }, TypeKind::Effect { base: b2, row: r2 }) => {
                self.unify(b1, b2, span);
                self.effect_rows_unify(&r1, &r2, span);
            }

            (TypeKind::List(e1), TypeKind::List(e2)) => self.unify(e1, e2, span),

            (TypeKind::Optional(e1), TypeKind::Optional(e2)) => self.unify(e1, e2, span),
            (TypeKind::Maybe(e1), TypeKind::Maybe(e2)) => self.unify(e1, e2, span),
            (
                TypeKind::Patch {
                    target: t1,
                    deep: d1,
                },
                TypeKind::Patch {
                    target: t2,
                    deep: d2,
                },
            ) if d1 == d2 => self.unify(t1, t2, span),

            // Higher-kinded application: decompose head and argument. Required so
            // method-level / constraint type params solve when unifying `F A`
            // shapes (`F A ~ F B`, `?f A ~ F A`). Structural `!=` would spuriously
            // mismatch two separately-built but equal `Apply` nodes.
            (TypeKind::Apply { func: f1, arg: a1 }, TypeKind::Apply { func: f2, arg: a2 }) => {
                self.unify(f1, f2, span);
                self.unify(a1, a2, span);
            }

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
                        self.unify(r1, r2, span);
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
                _ => {}
            }
        }
    }
}
