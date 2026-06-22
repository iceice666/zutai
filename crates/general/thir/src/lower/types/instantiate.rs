use super::*;

impl<'hir> Lowerer<'hir> {
    /// Substitute `TypeVar`s appearing in `ty` according to `subst`.
    /// Allocates new `Type` nodes for any structural type that contains
    /// substituted vars; leaf types and unchanged subtrees are reused.
    pub(in crate::lower) fn instantiate_type_vars(
        &mut self,
        ty: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
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
            TypeKind::Patch { target, deep } => {
                let new_target = self.instantiate_type_vars(target, subst);
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
            TypeKind::ForAll {
                params,
                param_bounds,
                body,
            } => {
                let inner_subst: FxHashMap<BindingId, TypeId> = subst
                    .iter()
                    .filter(|(binding, _)| !params.contains(binding))
                    .map(|(&binding, &ty)| (binding, ty))
                    .collect();
                if inner_subst.is_empty() {
                    return ty;
                }
                let new_body = self.instantiate_type_vars(body, &inner_subst);
                self.alloc_type(Type {
                    kind: TypeKind::ForAll {
                        params,
                        param_bounds,
                        body: new_body,
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
        subst: &FxHashMap<BindingId, RowTail>,
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
            TypeKind::Patch { target, deep } => {
                let new_target = self.instantiate_row_params(target, subst);
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
            TypeKind::ForAll {
                params,
                param_bounds,
                body,
            } => {
                let inner_subst: FxHashMap<BindingId, RowTail> = subst
                    .iter()
                    .filter(|(binding, _)| !params.contains(binding))
                    .map(|(&binding, &row)| (binding, row))
                    .collect();
                if inner_subst.is_empty() {
                    return ty;
                }
                let new_body = self.instantiate_row_params(body, &inner_subst);
                self.alloc_type(Type {
                    kind: TypeKind::ForAll {
                        params,
                        param_bounds,
                        body: new_body,
                    },
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
}
