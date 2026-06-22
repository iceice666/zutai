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
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Patch { target: inner, .. } => {
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
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Patch { target: inner, .. } => {
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
}
