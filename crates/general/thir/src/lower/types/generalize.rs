use super::*;

impl<'hir> Lowerer<'hir> {
    // ── HM let-generalization ────────────────────────────────────────────────

    /// Collect every unresolved `InferVar` id that appears free in `ty`, deduped
    /// in stable order. Resolves chains at entry so partially-solved variables
    /// (e.g. `?1` pointing at `?0`) are reported by their canonical id.
    pub(in crate::lower) fn free_infer_vars_in(&self, ty: TypeId) -> Vec<u32> {
        let mut vars: Vec<u32> = Vec::new();
        self.free_infer_vars_into(ty, &mut vars);
        vars.sort_unstable();
        vars.dedup();
        vars
    }

    pub(in crate::lower) fn free_infer_vars_into(&self, ty: TypeId, out: &mut Vec<u32>) {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::InferVar(v) => out.push(v),
            TypeKind::Function { from, to } => {
                self.free_infer_vars_into(from, out);
                self.free_infer_vars_into(to, out);
            }
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Patch { target: inner, .. } => {
                self.free_infer_vars_into(inner, out);
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(payload) = v.payload {
                        self.free_infer_vars_into(payload, out);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => ty,
                        TypeTupleItem::Positional(ty) => ty,
                    };
                    self.free_infer_vars_into(inner, out);
                }
            }
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.free_infer_vars_into(f.ty, out);
                }
            }
            TypeKind::Effect { base, row } => {
                self.free_infer_vars_into(base, out);
                for op in row.ops {
                    self.free_infer_vars_into(op.param, out);
                    self.free_infer_vars_into(op.result, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in args {
                    self.free_infer_vars_into(a, out);
                }
            }
            TypeKind::Apply { func, arg } => {
                self.free_infer_vars_into(func, out);
                self.free_infer_vars_into(arg, out);
            }
            _ => {}
        }
    }

    /// All `InferVar` ids free in the stored type of any binding other than
    /// `exclude`. These are "in the environment": generalizing them would be
    /// unsound, so they stay monomorphic.
    pub(in crate::lower) fn env_infer_vars(&self, exclude: BindingId) -> HashSet<u32> {
        let mut set = HashSet::new();
        for (&binding, &ty) in &self.value_types {
            if binding == exclude {
                continue;
            }
            for v in self.free_infer_vars_in(ty) {
                set.insert(v);
            }
        }
        set
    }

    /// HM "gen" rule: generalize `binding`'s free inference variables that are not
    /// shared with the surrounding environment. Call AFTER the binding's body is
    /// fully lowered and its type is in `value_types`.
    ///
    /// Source-order / define-before-use: only references that appear textually
    /// after this point observe the scheme. Polymorphic recursion is not inferred.
    pub(in crate::lower) fn generalize_if_polymorphic(&mut self, binding: BindingId, ty: TypeId) {
        let env = self.env_infer_vars(binding);
        let scheme: Vec<u32> = self
            .free_infer_vars_in(ty)
            .into_iter()
            .filter(|v| !env.contains(v))
            .collect();
        if !scheme.is_empty() {
            self.poly_schemes.insert(binding, scheme);
        }
    }

    /// Substitute `InferVar`s appearing in `ty` according to `subst`, allocating
    /// new nodes only where a substitution occurs. Unlike `instantiate_type_vars`,
    /// this resolves chains at entry because stored signatures contain
    /// partially-solved `InferVar`s.
    pub(in crate::lower) fn instantiate_infer_vars(
        &mut self,
        ty: TypeId,
        subst: &HashMap<u32, TypeId>,
    ) -> TypeId {
        let ty = self.resolve(ty);
        if subst.is_empty() {
            return ty;
        }
        let span = self.type_arena[ty.0 as usize].span;
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::InferVar(v) => subst.get(&v).copied().unwrap_or(ty),
            TypeKind::Function { from, to } => {
                let new_from = self.instantiate_infer_vars(from, subst);
                let new_to = self.instantiate_infer_vars(to, subst);
                if new_from == from && new_to == to {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Function {
                        from: new_from,
                        to: new_to,
                    },
                    span,
                })
            }
            TypeKind::List(inner) => {
                let new_inner = self.instantiate_infer_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::List(new_inner),
                    span,
                })
            }
            TypeKind::Optional(inner) => {
                let new_inner = self.instantiate_infer_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Optional(new_inner),
                    span,
                })
            }
            TypeKind::Maybe(inner) => {
                let new_inner = self.instantiate_infer_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Maybe(new_inner),
                    span,
                })
            }
            TypeKind::Patch { target, deep } => {
                let new_target = self.instantiate_infer_vars(target, subst);
                if new_target == target {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Patch {
                        target: new_target,
                        deep,
                    },
                    span,
                })
            }
            TypeKind::Union(variants, tail) => {
                let new_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|p| self.instantiate_infer_vars(p, subst)),
                        span: v.span,
                    })
                    .collect();
                if new_variants
                    .iter()
                    .zip(variants.iter())
                    .all(|(n, o)| n.payload == o.payload)
                {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Union(new_variants, tail),
                    span,
                })
            }
            TypeKind::Tuple(items) => {
                let new_items: Vec<TypeTupleItem> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named {
                            name,
                            ty: inner,
                            span: s,
                        } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.instantiate_infer_vars(*inner, subst),
                            span: *s,
                        },
                        TypeTupleItem::Positional(inner) => {
                            TypeTupleItem::Positional(self.instantiate_infer_vars(*inner, subst))
                        }
                    })
                    .collect();
                if new_items == items {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(new_items),
                    span,
                })
            }
            TypeKind::Record(fields, tail) => {
                let new_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|f| TypeRecordField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: self.instantiate_infer_vars(f.ty, subst),
                        span: f.span,
                    })
                    .collect();
                if new_fields == fields {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Record(new_fields, tail),
                    span,
                })
            }
            TypeKind::Effect { base, row } => {
                let new_base = self.instantiate_infer_vars(base, subst);
                let new_ops: Vec<EffectOp> = row
                    .ops
                    .iter()
                    .map(|op| EffectOp {
                        name: op.name.clone(),
                        param: self.instantiate_infer_vars(op.param, subst),
                        result: self.instantiate_infer_vars(op.result, subst),
                        span: op.span,
                    })
                    .collect();
                if new_base == base && new_ops == row.ops {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Effect {
                        base: new_base,
                        row: EffectRow {
                            ops: new_ops,
                            tail: row.tail,
                        },
                    },
                    span,
                })
            }
            TypeKind::Apply { func, arg } => {
                let new_func = self.instantiate_infer_vars(func, subst);
                let new_arg = self.instantiate_infer_vars(arg, subst);
                if new_func == func && new_arg == arg {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Apply {
                        func: new_func,
                        arg: new_arg,
                    },
                    span,
                })
            }
            TypeKind::AliasApply { binding, args } => {
                let new_args: Vec<TypeId> = args
                    .iter()
                    .map(|&a| self.instantiate_infer_vars(a, subst))
                    .collect();
                if new_args == args {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::AliasApply {
                        binding,
                        args: new_args,
                    },
                    span,
                })
            }
            _ => ty,
        }
    }

    pub(in crate::lower) fn type_mismatch(&mut self, expected: TypeId, found: TypeId, span: Span) {
        let expected = self.type_name(expected);
        let found = self.type_name(found);
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::TypeMismatch { expected, found },
            span,
        });
    }
}
