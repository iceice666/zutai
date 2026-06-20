use super::*;

impl<'hir> Lowerer<'hir> {
    /// Collect all `TypeVar` binding IDs that appear free in `ty`, in a
    /// deduped stable order (by binding index).
    pub(in crate::lower) fn collect_type_vars(&self, ty: TypeId) -> Vec<BindingId> {
        let mut vars: Vec<BindingId> = Vec::new();
        self.collect_type_vars_into(ty, &mut vars);
        vars.sort_by_key(|b| b.0);
        vars.dedup();
        vars
    }

    pub(in crate::lower) fn collect_type_vars_into(&self, ty: TypeId, out: &mut Vec<BindingId>) {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) => out.push(b),
            TypeKind::Function { from, to } => {
                self.collect_type_vars_into(from, out);
                self.collect_type_vars_into(to, out);
            }
            TypeKind::List(inner) | TypeKind::Optional(inner) | TypeKind::Maybe(inner) => {
                self.collect_type_vars_into(inner, out);
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(payload) = v.payload {
                        self.collect_type_vars_into(payload, out);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => ty,
                        TypeTupleItem::Positional(ty) => ty,
                    };
                    self.collect_type_vars_into(inner, out);
                }
            }
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.collect_type_vars_into(f.ty, out);
                }
            }
            TypeKind::Effect { base, row } => {
                self.collect_type_vars_into(base, out);
                for op in row.ops {
                    self.collect_type_vars_into(op.param, out);
                    self.collect_type_vars_into(op.result, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in args {
                    self.collect_type_vars_into(a, out);
                }
            }
            TypeKind::Apply { func, arg } => {
                self.collect_type_vars_into(func, out);
                self.collect_type_vars_into(arg, out);
            }
            _ => {}
        }
    }

    /// Collect all rigid row variables (`RowTail::Param`) appearing in `ty`, in a
    /// deduped stable order. These `<Rest>` row parameters are instantiated with
    /// fresh flexible row variables at each call site, like type parameters.
    pub(in crate::lower) fn collect_row_params(&self, ty: TypeId) -> Vec<BindingId> {
        let mut vars: Vec<BindingId> = Vec::new();
        self.collect_row_params_into(ty, &mut vars);
        vars.sort_by_key(|b| b.0);
        vars.dedup();
        vars
    }

    pub(in crate::lower) fn collect_row_params_into(&self, ty: TypeId, out: &mut Vec<BindingId>) {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                self.collect_row_params_into(from, out);
                self.collect_row_params_into(to, out);
            }
            TypeKind::List(inner) | TypeKind::Optional(inner) | TypeKind::Maybe(inner) => {
                self.collect_row_params_into(inner, out);
            }
            TypeKind::Record(fields, tail) => {
                for f in &fields {
                    self.collect_row_params_into(f.ty, out);
                }
                if let RowTail::Param(b) = tail {
                    out.push(b);
                }
            }
            TypeKind::Union(variants, tail) => {
                for v in &variants {
                    if let Some(payload) = v.payload {
                        self.collect_row_params_into(payload, out);
                    }
                }
                if let RowTail::Param(b) = tail {
                    out.push(b);
                }
            }
            TypeKind::Effect { base, row } => {
                self.collect_row_params_into(base, out);
                for op in &row.ops {
                    self.collect_row_params_into(op.param, out);
                    self.collect_row_params_into(op.result, out);
                }
                if let RowTail::Param(b) = row.tail {
                    out.push(b);
                }
            }
            TypeKind::Tuple(items) => {
                for item in &items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => *ty,
                        TypeTupleItem::Positional(ty) => *ty,
                    };
                    self.collect_row_params_into(inner, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in &args {
                    self.collect_row_params_into(*a, out);
                }
            }
            TypeKind::Apply { func, arg } => {
                self.collect_row_params_into(func, out);
                self.collect_row_params_into(arg, out);
            }
            _ => {}
        }
    }

    /// Substitute `TypeVar`s appearing in `ty` according to `subst`.
    /// Allocates new `Type` nodes for any structural type that contains
    /// substituted vars; leaf types and unchanged subtrees are reused.
    pub(in crate::lower) fn instantiate_type_vars(
        &mut self,
        ty: TypeId,
        subst: &HashMap<BindingId, TypeId>,
    ) -> TypeId {
        if subst.is_empty() {
            return ty;
        }
        let span = self.type_arena[ty.0 as usize].span;
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) => subst.get(&b).copied().unwrap_or(ty),
            TypeKind::Function { from, to } => {
                let new_from = self.instantiate_type_vars(from, subst);
                let new_to = self.instantiate_type_vars(to, subst);
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
                let new_inner = self.instantiate_type_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::List(new_inner),
                    span,
                })
            }
            TypeKind::Optional(inner) => {
                let new_inner = self.instantiate_type_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Optional(new_inner),
                    span,
                })
            }
            TypeKind::Maybe(inner) => {
                let new_inner = self.instantiate_type_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Maybe(new_inner),
                    span,
                })
            }
            TypeKind::Union(variants, tail) => {
                let new_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|p| self.instantiate_type_vars(p, subst)),
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
                            ty: self.instantiate_type_vars(*inner, subst),
                            span: *s,
                        },
                        TypeTupleItem::Positional(inner) => {
                            TypeTupleItem::Positional(self.instantiate_type_vars(*inner, subst))
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
                        ty: self.instantiate_type_vars(f.ty, subst),
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
                let new_base = self.instantiate_type_vars(base, subst);
                let new_ops: Vec<EffectOp> = row
                    .ops
                    .iter()
                    .map(|op| EffectOp {
                        name: op.name.clone(),
                        param: self.instantiate_type_vars(op.param, subst),
                        result: self.instantiate_type_vars(op.result, subst),
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
            TypeKind::AliasApply { binding, args } => {
                let new_args: Vec<TypeId> = args
                    .iter()
                    .map(|&a| self.instantiate_type_vars(a, subst))
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
            TypeKind::Apply { func, arg } => {
                let new_func = self.instantiate_type_vars(func, subst);
                let new_arg = self.instantiate_type_vars(arg, subst);
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
            _ => ty,
        }
    }

    /// Replace rigid row variables (`RowTail::Param`) in `ty` with the flexible
    /// row variables given by `subst`, rebuilding only structural nodes.
    pub(in crate::lower) fn instantiate_row_params(
        &mut self,
        ty: TypeId,
        subst: &HashMap<BindingId, RowTail>,
    ) -> TypeId {
        if subst.is_empty() {
            return ty;
        }
        let span = self.type_arena[ty.0 as usize].span;
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                let nf = self.instantiate_row_params(from, subst);
                let nt = self.instantiate_row_params(to, subst);
                if nf == from && nt == to {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Function { from: nf, to: nt },
                    span,
                })
            }
            TypeKind::List(inner) => {
                let ni = self.instantiate_row_params(inner, subst);
                if ni == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::List(ni),
                    span,
                })
            }
            TypeKind::Optional(inner) => {
                let ni = self.instantiate_row_params(inner, subst);
                if ni == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Optional(ni),
                    span,
                })
            }
            TypeKind::Maybe(inner) => {
                let ni = self.instantiate_row_params(inner, subst);
                if ni == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Maybe(ni),
                    span,
                })
            }
            TypeKind::Record(fields, tail) => {
                let new_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|f| TypeRecordField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: self.instantiate_row_params(f.ty, subst),
                        span: f.span,
                    })
                    .collect();
                let new_tail = match tail {
                    RowTail::Param(b) => subst.get(&b).copied().unwrap_or(tail),
                    _ => tail,
                };
                if new_fields == fields && new_tail == tail {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Record(new_fields, new_tail),
                    span,
                })
            }
            TypeKind::Union(variants, tail) => {
                let new_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|p| self.instantiate_row_params(p, subst)),
                        span: v.span,
                    })
                    .collect();
                let new_tail = match tail {
                    RowTail::Param(b) => subst.get(&b).copied().unwrap_or(tail),
                    _ => tail,
                };
                self.alloc_type(Type {
                    kind: TypeKind::Union(new_variants, new_tail),
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
                            ty: self.instantiate_row_params(*inner, subst),
                            span: *s,
                        },
                        TypeTupleItem::Positional(inner) => {
                            TypeTupleItem::Positional(self.instantiate_row_params(*inner, subst))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(new_items),
                    span,
                })
            }
            TypeKind::Apply { func, arg } => {
                let nf = self.instantiate_row_params(func, subst);
                let na = self.instantiate_row_params(arg, subst);
                if nf == func && na == arg {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Apply { func: nf, arg: na },
                    span,
                })
            }
            TypeKind::AliasApply { binding, args } => {
                let new_args: Vec<TypeId> = args
                    .iter()
                    .map(|&a| self.instantiate_row_params(a, subst))
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
            TypeKind::Effect { base, row } => {
                let new_base = self.instantiate_row_params(base, subst);
                let new_ops: Vec<EffectOp> = row
                    .ops
                    .iter()
                    .map(|op| EffectOp {
                        name: op.name.clone(),
                        param: self.instantiate_row_params(op.param, subst),
                        result: self.instantiate_row_params(op.result, subst),
                        span: op.span,
                    })
                    .collect();
                let new_tail = match row.tail {
                    RowTail::Param(b) => subst.get(&b).copied().unwrap_or(row.tail),
                    _ => row.tail,
                };
                if new_base == base && new_ops == row.ops && new_tail == row.tail {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Effect {
                        base: new_base,
                        row: EffectRow {
                            ops: new_ops,
                            tail: new_tail,
                        },
                    },
                    span,
                })
            }
            _ => ty,
        }
    }

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
            TypeKind::List(inner) | TypeKind::Optional(inner) | TypeKind::Maybe(inner) => {
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
