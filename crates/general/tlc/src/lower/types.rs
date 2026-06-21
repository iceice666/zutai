use std::collections::{HashMap, HashSet};

use zutai_hir::BindingId;
use zutai_thir::{
    EffectRow, RowTail, ThirDeclKind, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
};

use crate::ir::{Kind, Literal, PrimTy, Row, TlcTupleField, TlcType, TlcTypeId, TlcTypeVar};

use super::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_type(&mut self, thir_ty: TypeId) -> TlcTypeId {
        let resolved = self.resolve_thir(thir_ty);
        if let Some(&cached) = self.type_cache.get(&resolved.0) {
            return cached;
        }
        let kind = self.thir.type_arena[resolved.0 as usize].kind.clone();
        let tlc_ty = match kind {
            TypeKind::Int => self.alloc_type(TlcType::Prim(PrimTy::Int)),
            TypeKind::Float => self.alloc_type(TlcType::Prim(PrimTy::Float)),
            TypeKind::FixedNum(fw) => self.alloc_type(TlcType::Prim(PrimTy::FixedNum(fw))),
            TypeKind::Posit(spec) => self.alloc_type(TlcType::Prim(PrimTy::Posit(spec))),
            TypeKind::Bool => self.alloc_type(TlcType::Prim(PrimTy::Bool)),
            // Singleton types — preserve discrimination (Phase 0 bug fix).
            TypeKind::True => self.alloc_type(TlcType::Singleton(Literal::Bool(true))),
            TypeKind::False => self.alloc_type(TlcType::Singleton(Literal::Bool(false))),
            // Atom with its symbol payload (Phase 0 bug fix).
            TypeKind::Atom(sym) => self.alloc_type(TlcType::Singleton(Literal::Atom(sym))),
            TypeKind::Text => self.alloc_type(TlcType::Prim(PrimTy::Str)),
            TypeKind::Function { from, to } => {
                let from_tlc = self.lower_type(from);
                match self.lower_effect_type_to_tlc(to, &HashMap::new()) {
                    Some((base_tlc, eff_row)) => {
                        self.alloc_type(TlcType::Fun(from_tlc, base_tlc, eff_row))
                    }
                    None => {
                        let to_tlc = self.lower_type(to);
                        self.alloc_type(TlcType::Fun(from_tlc, to_tlc, Row::REmpty))
                    }
                }
            }
            TypeKind::Effect { base, .. } => self.lower_type(base),
            TypeKind::Never => self.alloc_type(TlcType::Prim(PrimTy::Nothing)),
            TypeKind::List(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::List(inner_tlc))
            }
            TypeKind::Optional(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::Optional(inner_tlc))
            }
            TypeKind::Maybe(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::Maybe(inner_tlc))
            }
            TypeKind::Patch { target, deep } => {
                self.lower_patch_type_with_subst(target, deep, &HashMap::new())
            }
            TypeKind::Record(fields, tail) => {
                let row_fields: Vec<(String, TlcTypeId, bool)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_type(f.ty), f.optional))
                    .collect();
                let row_tail = self.thir_row_tail(tail);
                self.alloc_type(TlcType::Record(Row::from_record_fields_with_tail(
                    row_fields, row_tail,
                )))
            }
            TypeKind::Tuple(items) => {
                let tlc_items: Vec<TlcTupleField> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => TlcTupleField::Named {
                            name: name.clone(),
                            ty: self.lower_type(*ty),
                        },
                        TypeTupleItem::Positional(ty) => {
                            TlcTupleField::Positional(self.lower_type(*ty))
                        }
                    })
                    .collect();
                self.alloc_type(TlcType::Tuple(tlc_items))
            }
            // Union / sum type — build a VariantT row (Phase 0 bug fix).
            TypeKind::Union(variants, tail) => {
                let fields: Vec<(String, TlcTypeId)> = variants
                    .iter()
                    .map(|v| {
                        let arm_ty = match v.payload {
                            // Bare atom arm: type is `Singleton(Atom(name))`.
                            None => {
                                self.alloc_type(TlcType::Singleton(Literal::Atom(v.name.clone())))
                            }
                            // Tagged-payload arm: lower the payload type directly.
                            Some(payload_ty) => self.lower_type(payload_ty),
                        };
                        (v.name.clone(), arm_ty)
                    })
                    .collect();
                let row_tail = self.thir_row_tail(tail);
                self.alloc_type(TlcType::VariantT(Row::from_fields_with_tail(
                    fields, row_tail,
                )))
            }
            TypeKind::TypeVar(binding) => {
                let tyvar = self.named_tyvar(binding);
                self.alloc_type(TlcType::TyVar(tyvar, Kind::ground()))
            }
            TypeKind::InferVar(v) => {
                let tyvar = self.inferred_tyvar(v);
                self.alloc_type(TlcType::TyVar(tyvar, Kind::ground()))
            }
            TypeKind::Alias(binding) => {
                let tyvar = self.named_tyvar(binding);
                let kind = self.alias_head_kind(binding);
                self.alloc_type(TlcType::TyVar(tyvar, kind))
            }
            TypeKind::AliasApply { binding, args } => {
                let tyvar = self.named_tyvar(binding);
                let kind = self.alias_head_kind(binding);
                let mut spine = self.alloc_type(TlcType::TyVar(tyvar, kind));
                for &arg in &args {
                    let arg_tlc = self.lower_type(arg);
                    spine = self.alloc_type(TlcType::TyApp(spine, arg_tlc));
                }
                spine
            }
            TypeKind::Con(binding) => {
                // A bare builtin constructor (`List`, `Optional`) — kind `Type -> Type`.
                let tyvar = self.named_tyvar(binding);
                let kind = Kind::Arrow(Box::new(Kind::ground()), Box::new(Kind::ground()));
                self.alloc_type(TlcType::TyVar(tyvar, kind))
            }
            TypeKind::Apply { func, arg } => {
                // Curried higher-kinded / partial application maps 1:1 to TyApp.
                let func_tlc = self.lower_type(func);
                let arg_tlc = self.lower_type(arg);
                self.alloc_type(TlcType::TyApp(func_tlc, arg_tlc))
            }
            // TLC is only produced when THIR is complete — Error cannot appear.
            TypeKind::Error => unreachable!(
                "TypeKind::Error must not reach TLC lowering; only call lower_thir when is_thir_complete()"
            ),
            // Type-valued expressions are erased to Lit(Nothing) in the expr lowerer;
            // their type is mapped to a Nothing placeholder here.
            TypeKind::Type => self.alloc_type(TlcType::Prim(PrimTy::Nothing)),
        };
        self.type_cache.insert(resolved.0, tlc_ty);
        tlc_ty
    }

    fn lower_effect_type_to_tlc(
        &mut self,
        ty: TypeId,
        subst: &HashMap<BindingId, TypeId>,
    ) -> Option<(TlcTypeId, Row)> {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(binding) => subst
                .get(&binding)
                .and_then(|&replacement| self.lower_effect_type_to_tlc(replacement, subst)),
            TypeKind::Effect { base, row } => {
                let base = self.lower_type_with_subst(base, subst);
                let row = self.lower_effect_row_to_tlc_with_subst(&row, subst);
                Some((base, row))
            }
            TypeKind::Alias(binding) => self
                .type_alias_body(binding)
                .and_then(|body| self.lower_effect_type_to_tlc(body, subst)),
            TypeKind::AliasApply { binding, args } => {
                self.lower_effect_alias_apply_to_tlc(binding, &args, subst)
            }
            TypeKind::Apply { .. } => {
                let (head, args) = self.thir_app_spine(ty);
                match self.thir.type_arena[head.0 as usize].kind {
                    TypeKind::Alias(binding) => {
                        self.lower_effect_alias_apply_to_tlc(binding, &args, subst)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn lower_effect_alias_apply_to_tlc(
        &mut self,
        binding: BindingId,
        args: &[TypeId],
        subst: &HashMap<BindingId, TypeId>,
    ) -> Option<(TlcTypeId, Row)> {
        let (params, body) = self.type_alias_params_body(binding)?;
        if params.len() != args.len() {
            return None;
        }
        let mut child = subst.clone();
        for (param, &arg) in params.iter().zip(args) {
            child.insert(*param, arg);
        }
        self.lower_effect_type_to_tlc(body, &child)
    }

    fn lower_effect_row_to_tlc_with_subst(
        &mut self,
        row: &EffectRow,
        subst: &HashMap<BindingId, TypeId>,
    ) -> Row {
        let fields: Vec<_> = row
            .ops
            .iter()
            .map(|op| {
                let param = self.lower_type_with_subst(op.param, subst);
                let result = self.lower_type_with_subst(op.result, subst);
                let sig = self.alloc_type(TlcType::Fun(param, result, Row::REmpty));
                (op.name.clone(), sig)
            })
            .collect();
        let tail = self.thir_row_tail(row.tail);
        Row::from_fields_with_tail(fields, tail)
    }

    fn lower_type_with_subst(
        &mut self,
        ty: TypeId,
        subst: &HashMap<BindingId, TypeId>,
    ) -> TlcTypeId {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(binding) => match subst.get(&binding).copied() {
                Some(replacement) => self.lower_type_with_subst(replacement, subst),
                None => {
                    let tyvar = self.named_tyvar(binding);
                    self.alloc_type(TlcType::TyVar(tyvar, Kind::ground()))
                }
            },
            TypeKind::Function { from, to } => {
                let from_tlc = self.lower_type_with_subst(from, subst);
                match self.lower_effect_type_to_tlc(to, subst) {
                    Some((base_tlc, row)) => self.alloc_type(TlcType::Fun(from_tlc, base_tlc, row)),
                    None => {
                        let to_tlc = self.lower_type_with_subst(to, subst);
                        self.alloc_type(TlcType::Fun(from_tlc, to_tlc, Row::REmpty))
                    }
                }
            }
            TypeKind::Effect { base, .. } => self.lower_type_with_subst(base, subst),
            TypeKind::List(inner) => {
                let inner = self.lower_type_with_subst(inner, subst);
                self.alloc_type(TlcType::List(inner))
            }
            TypeKind::Optional(inner) => {
                let inner = self.lower_type_with_subst(inner, subst);
                self.alloc_type(TlcType::Optional(inner))
            }
            TypeKind::Maybe(inner) => {
                let inner = self.lower_type_with_subst(inner, subst);
                self.alloc_type(TlcType::Maybe(inner))
            }
            TypeKind::Patch { target, deep } => {
                self.lower_patch_type_with_subst(target, deep, subst)
            }
            TypeKind::Record(fields, tail) => {
                let row_fields: Vec<(String, TlcTypeId, bool)> = fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            self.lower_type_with_subst(field.ty, subst),
                            field.optional,
                        )
                    })
                    .collect();
                let row_tail = self.thir_row_tail(tail);
                self.alloc_type(TlcType::Record(Row::from_record_fields_with_tail(
                    row_fields, row_tail,
                )))
            }
            TypeKind::Union(variants, tail) => {
                let fields: Vec<(String, TlcTypeId)> = variants
                    .iter()
                    .map(|variant| {
                        let ty = match variant.payload {
                            Some(payload) => self.lower_type_with_subst(payload, subst),
                            None => self.alloc_type(TlcType::Singleton(Literal::Atom(
                                variant.name.clone(),
                            ))),
                        };
                        (variant.name.clone(), ty)
                    })
                    .collect();
                let row_tail = self.thir_row_tail(tail);
                self.alloc_type(TlcType::VariantT(Row::from_fields_with_tail(
                    fields, row_tail,
                )))
            }
            TypeKind::Tuple(items) => {
                let items: Vec<TlcTupleField> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => TlcTupleField::Named {
                            name: name.clone(),
                            ty: self.lower_type_with_subst(*ty, subst),
                        },
                        TypeTupleItem::Positional(ty) => {
                            TlcTupleField::Positional(self.lower_type_with_subst(*ty, subst))
                        }
                    })
                    .collect();
                self.alloc_type(TlcType::Tuple(items))
            }
            TypeKind::AliasApply { binding, args } => {
                let tyvar = self.named_tyvar(binding);
                let kind = self.alias_head_kind(binding);
                let mut spine = self.alloc_type(TlcType::TyVar(tyvar, kind));
                for arg in args {
                    let arg_tlc = self.lower_type_with_subst(arg, subst);
                    spine = self.alloc_type(TlcType::TyApp(spine, arg_tlc));
                }
                spine
            }
            TypeKind::Apply { func, arg } => {
                let func = self.lower_type_with_subst(func, subst);
                let arg = self.lower_type_with_subst(arg, subst);
                self.alloc_type(TlcType::TyApp(func, arg))
            }
            _ => self.lower_type(ty),
        }
    }

    fn resolve_thir(&self, ty: TypeId) -> TypeId {
        ty
    }

    /// Convert a THIR row tail into a TLC row tail: a closed row ends in `REmpty`;
    /// every open form ends in an `RVar` — anonymous `...` gets a fresh variable,
    /// a `<Rest>` parameter maps to `Named`, and a flexible tail to `Inferred`.
    fn thir_row_tail(&mut self, tail: RowTail) -> Row {
        match tail {
            RowTail::Closed => Row::REmpty,
            RowTail::Open => Row::RVar(self.fresh_row_var()),
            RowTail::Param(binding) => Row::RVar(TlcTypeVar::Named(binding.0)),
            RowTail::Infer(v) => Row::RVar(TlcTypeVar::Inferred(v)),
        }
    }

    /// Collect the bindings of every `<Rest>` row parameter used as a row tail in
    /// `sig`. These quantify with `Kind::Row`, unlike ordinary type parameters.
    pub(super) fn sig_row_param_bindings(&self, sig: TypeId) -> HashSet<u32> {
        let mut out = HashSet::new();
        self.collect_sig_row_params(sig, &mut out);
        out
    }

    fn collect_sig_row_params(&self, ty: TypeId, out: &mut HashSet<u32>) {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                self.collect_sig_row_params(from, out);
                self.collect_sig_row_params(to, out);
            }
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Patch { target: inner, .. } => {
                self.collect_sig_row_params(inner, out);
            }
            TypeKind::Record(fields, tail) => {
                for f in &fields {
                    self.collect_sig_row_params(f.ty, out);
                }
                if let RowTail::Param(b) = tail {
                    out.insert(b.0);
                }
            }
            TypeKind::Union(variants, tail) => {
                for v in &variants {
                    if let Some(p) = v.payload {
                        self.collect_sig_row_params(p, out);
                    }
                }
                if let RowTail::Param(b) = tail {
                    out.insert(b.0);
                }
            }
            TypeKind::Tuple(items) => {
                for item in &items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => *ty,
                        TypeTupleItem::Positional(ty) => *ty,
                    };
                    self.collect_sig_row_params(inner, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in &args {
                    self.collect_sig_row_params(*a, out);
                }
            }
            _ => {}
        }
    }

    fn lower_patch_type_with_subst(
        &mut self,
        target: TypeId,
        deep: bool,
        subst: &HashMap<BindingId, TypeId>,
    ) -> TlcTypeId {
        let Some((fields, tail, env)) = self.record_shape_with_subst(target, subst) else {
            return self.alloc_type(TlcType::Record(Row::REmpty));
        };
        let row_fields: Vec<(String, TlcTypeId, bool)> = fields
            .iter()
            .map(|field| {
                let field_ty = if deep && self.record_shape_with_subst(field.ty, &env).is_some() {
                    self.lower_patch_type_with_subst(field.ty, true, &env)
                } else {
                    self.lower_type_with_subst(field.ty, &env)
                };
                (field.name.clone(), field_ty, true)
            })
            .collect();
        let row_tail = self.thir_row_tail(tail);
        self.alloc_type(TlcType::Record(Row::from_record_fields_with_tail(
            row_fields, row_tail,
        )))
    }

    fn record_shape_with_subst(
        &self,
        target: TypeId,
        subst: &HashMap<BindingId, TypeId>,
    ) -> Option<(Vec<TypeRecordField>, RowTail, HashMap<BindingId, TypeId>)> {
        let mut ty = target;
        let mut env = subst.clone();
        let mut fuel = 64u32;
        while fuel > 0 {
            fuel -= 1;
            match self.thir.type_arena[ty.0 as usize].kind.clone() {
                TypeKind::TypeVar(binding) => {
                    ty = *env.get(&binding)?;
                }
                TypeKind::Alias(binding) => {
                    ty = self.type_alias_body(binding)?;
                }
                TypeKind::AliasApply { binding, args } => {
                    let (params, body) = self.type_alias_params_body(binding)?;
                    if params.len() != args.len() {
                        return None;
                    }
                    for (param, arg) in params.into_iter().zip(args) {
                        env.insert(param, arg);
                    }
                    ty = body;
                }
                TypeKind::Record(fields, tail) => return Some((fields, tail, env)),
                _ => return None,
            }
        }
        None
    }

    /// Returns the kind for an alias head: `Type(0)` for a 0-ary alias;
    /// `Arrow(Type(0), … Arrow(Type(0), Type(0)))` for an n-ary one.
    /// Phase 2 first constructs `Kind::Arrow` — the dormant Phase-1 variant goes live here.
    fn alias_head_kind(&self, binding: BindingId) -> Kind {
        let arity = self
            .thir
            .decls
            .iter()
            .find_map(|&d| {
                let decl = &self.thir.decl_arena[d];
                if decl.binding == binding
                    && let ThirDeclKind::TypeAlias { ref params, .. } = decl.kind
                {
                    return Some(params.len());
                }
                None
            })
            .unwrap_or(0);
        (0..arity).fold(Kind::ground(), |acc, _| {
            Kind::Arrow(Box::new(Kind::ground()), Box::new(acc))
        })
    }

    pub(super) fn named_tyvar(&mut self, binding: BindingId) -> TlcTypeVar {
        *self
            .named_to_tyvar
            .entry(binding.0)
            .or_insert(TlcTypeVar::Named(binding.0))
    }

    pub(super) fn inferred_tyvar(&mut self, v: u32) -> TlcTypeVar {
        *self
            .infer_to_tyvar
            .entry(v)
            .or_insert(TlcTypeVar::Inferred(v))
    }

    pub(super) fn extract_instantiation(
        &mut self,
        scheme_vars: &[u32],
        scheme_ty: TypeId,
        ref_ty: TypeId,
    ) -> Vec<(TlcTypeVar, TlcTypeId)> {
        let mut mapping: HashMap<u32, TypeId> = HashMap::new();
        self.match_types(scheme_ty, ref_ty, &mut mapping);
        scheme_vars
            .iter()
            .map(|&v| {
                let tlc_ty = if let Some(&concrete) = mapping.get(&v) {
                    self.lower_type(concrete)
                } else {
                    let tyvar = self.inferred_tyvar(v);
                    self.alloc_type(TlcType::TyVar(tyvar, Kind::ground()))
                };
                (self.inferred_tyvar(v), tlc_ty)
            })
            .collect()
    }

    fn match_types(&self, template: TypeId, instance: TypeId, out: &mut HashMap<u32, TypeId>) {
        use TypeKind::*;
        match self.thir.type_arena[template.0 as usize].kind.clone() {
            InferVar(v) => {
                out.entry(v).or_insert(instance);
            }
            Function { from: tf, to: tt } => {
                if let Function { from: iif, to: it } =
                    self.thir.type_arena[instance.0 as usize].kind.clone()
                {
                    self.match_types(tf, iif, out);
                    self.match_types(tt, it, out);
                }
            }
            List(ti) => {
                if let List(ii) = self.thir.type_arena[instance.0 as usize].kind.clone() {
                    self.match_types(ti, ii, out);
                }
            }
            Optional(ti) => {
                if let Optional(ii) = self.thir.type_arena[instance.0 as usize].kind.clone() {
                    self.match_types(ti, ii, out);
                }
            }
            Patch {
                target: tt,
                deep: td,
            } => {
                if let Patch {
                    target: it,
                    deep: id,
                } = self.thir.type_arena[instance.0 as usize].kind.clone()
                    && td == id
                {
                    self.match_types(tt, it, out);
                }
            }
            _ => {}
        }
    }
}
