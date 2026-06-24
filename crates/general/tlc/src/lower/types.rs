use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;
use zutai_thir::{
    EffectRow, Kind as ThirKind, RowTail, ThirDeclKind, TypeId, TypeKind, TypeRecordField,
    TypeTupleItem, UniverseLevel,
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
            TypeKind::Opaque(name) => self.alloc_type(TlcType::Opaque(name)),
            TypeKind::Bool => self.alloc_type(TlcType::Prim(PrimTy::Bool)),
            // Singleton types — preserve discrimination (Phase 0 bug fix).
            TypeKind::True => self.alloc_type(TlcType::Singleton(Literal::Bool(true))),
            TypeKind::False => self.alloc_type(TlcType::Singleton(Literal::Bool(false))),
            // Atom with its symbol payload (Phase 0 bug fix).
            TypeKind::Atom(sym) => self.alloc_type(TlcType::Singleton(Literal::Atom(sym))),
            TypeKind::Text => self.alloc_type(TlcType::Prim(PrimTy::Str)),
            TypeKind::Function { from, to } => {
                let from_tlc = self.lower_type(from);
                match self.lower_effect_type_to_tlc(to, &FxHashMap::default()) {
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
                self.lower_patch_type_with_subst(target, deep, &FxHashMap::default())
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
                let kind = self.kind_for_type_param(binding);
                self.alloc_type(TlcType::TyVar(tyvar, kind))
            }
            TypeKind::InferVar(v) => {
                let tyvar = self.inferred_tyvar(v);
                let kind = self.kind_for_infer_var(v);
                self.alloc_type(TlcType::TyVar(tyvar, kind))
            }
            TypeKind::Alias(binding) => {
                let tyvar = self.named_tyvar(binding);
                let kind = self.alias_head_kind(binding);
                self.alloc_type(TlcType::TyVar(tyvar, kind))
            }
            TypeKind::AliasApply { binding, args } => {
                let tyvar = self.named_tyvar(binding);
                let kind = self.alias_head_kind_for_args(binding, &args);
                let mut spine = self.alloc_type(TlcType::TyVar(tyvar, kind));
                for &arg in &args {
                    let arg_tlc = self.lower_type(arg);
                    spine = self.alloc_type(TlcType::TyApp(spine, arg_tlc));
                }
                spine
            }
            TypeKind::ForAll {
                params,
                param_bounds,
                body,
            } => {
                let mut body_tlc = self.lower_type(body);
                for (index, &param) in params.iter().enumerate().rev() {
                    let tyvar = self.named_tyvar(param);
                    let kind = self.kind_for_type_param(param);
                    for _bound in param_bounds[index].iter().rev() {
                        let dict_ty = self.alloc_type(TlcType::Record(Row::REmpty));
                        body_tlc = self.alloc_type(TlcType::Fun(dict_ty, body_tlc, Row::REmpty));
                    }
                    body_tlc = self.alloc_type(TlcType::ForAll(tyvar, kind, body_tlc));
                }
                return body_tlc;
            }
            TypeKind::Con(binding) => {
                // A bare builtin constructor (`List`, `Optional`) — kind `Type ℓ -> Type ℓ`.
                let tyvar = self.named_tyvar(binding);
                let level = self
                    .thir
                    .type_universes
                    .get(resolved.0 as usize)
                    .copied()
                    .unwrap_or(0);
                let kind = Kind::Arrow(Box::new(Kind::Type(level)), Box::new(Kind::Type(level)));
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
            TypeKind::Type(_) => self.alloc_type(TlcType::Prim(PrimTy::Nothing)),
        };
        self.type_cache.insert(resolved.0, tlc_ty);
        tlc_ty
    }

    fn lower_effect_type_to_tlc(
        &mut self,
        ty: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
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
        subst: &FxHashMap<BindingId, TypeId>,
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
        subst: &FxHashMap<BindingId, TypeId>,
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

    pub(super) fn lower_type_with_subst(
        &mut self,
        ty: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
    ) -> TlcTypeId {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(binding) => match subst.get(&binding).copied() {
                Some(replacement) => self.lower_type_with_subst(replacement, subst),
                None => {
                    let tyvar = self.named_tyvar(binding);
                    let kind = self.kind_for_type_param(binding);
                    self.alloc_type(TlcType::TyVar(tyvar, kind))
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
                let kind = self.alias_head_kind_for_args(binding, &args);
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
    pub(super) fn sig_row_param_bindings(&self, sig: TypeId) -> FxHashSet<u32> {
        let mut out = FxHashSet::default();
        self.collect_sig_row_params(sig, &mut out);
        out
    }

    fn collect_sig_row_params(&self, ty: TypeId, out: &mut FxHashSet<u32>) {
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
        subst: &FxHashMap<BindingId, TypeId>,
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

    pub(super) fn record_shape_with_subst(
        &self,
        target: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
    ) -> Option<(Vec<TypeRecordField>, RowTail, FxHashMap<BindingId, TypeId>)> {
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

    fn lower_thir_kind(&self, kind: &ThirKind) -> Kind {
        match kind {
            ThirKind::Type(UniverseLevel::Known(n)) => Kind::Type(*n),
            ThirKind::Type(other) => {
                panic!("unsolved THIR universe level reached TLC lowering: {other:?}")
            }
            ThirKind::Row(inner) => Kind::Row(Box::new(self.lower_thir_kind(inner))),
            ThirKind::Arrow(from, to) => Kind::Arrow(
                Box::new(self.lower_thir_kind(from)),
                Box::new(self.lower_thir_kind(to)),
            ),
        }
    }

    pub(super) fn kind_for_type_param(&self, binding: BindingId) -> Kind {
        self.thir
            .type_param_kinds
            .get(&binding)
            .map(|kind| self.lower_thir_kind(kind))
            .unwrap_or_else(Kind::ground)
    }

    pub(super) fn kind_for_infer_var(&self, _infer: u32) -> Kind {
        Kind::ground()
    }

    fn kind_for_type_id(&self, ty: TypeId) -> Kind {
        Kind::Type(
            self.thir
                .type_universes
                .get(ty.0 as usize)
                .copied()
                .unwrap_or(0),
        )
    }

    fn alias_head_kind(&self, binding: BindingId) -> Kind {
        let Some((params, ty)) = self.thir.decls.iter().find_map(|&d| {
            let decl = &self.thir.decl_arena[d];
            if decl.binding == binding
                && let ThirDeclKind::TypeAlias { ref params, ty } = decl.kind
            {
                return Some((params.clone(), ty));
            }
            None
        }) else {
            return Kind::ground();
        };
        params
            .into_iter()
            .rev()
            .fold(self.kind_for_type_id(ty), |acc, param| {
                Kind::Arrow(Box::new(self.kind_for_type_param(param)), Box::new(acc))
            })
    }

    fn alias_head_kind_for_args(&mut self, binding: BindingId, args: &[TypeId]) -> Kind {
        let Some((params, ty)) = self.thir.decls.iter().find_map(|&d| {
            let decl = &self.thir.decl_arena[d];
            if decl.binding == binding
                && let ThirDeclKind::TypeAlias { ref params, ty } = decl.kind
            {
                return Some((params.clone(), ty));
            }
            None
        }) else {
            return Kind::ground();
        };
        let result = if params.len() == args.len() {
            let subst: FxHashMap<BindingId, TypeId> =
                params.iter().copied().zip(args.iter().copied()).collect();
            Kind::Type(self.thir_universe_with_subst(ty, &subst))
        } else {
            self.kind_for_type_id(ty)
        };
        params.into_iter().rev().fold(result, |acc, param| {
            Kind::Arrow(Box::new(self.kind_for_type_param(param)), Box::new(acc))
        })
    }

    fn thir_universe_with_subst(&self, ty: TypeId, subst: &FxHashMap<BindingId, TypeId>) -> u32 {
        self.thir_universe_with_subst_seen(ty, subst, &mut FxHashSet::default())
    }

    fn thir_universe_with_subst_seen(
        &self,
        ty: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
        alias_seen: &mut FxHashSet<BindingId>,
    ) -> u32 {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Type(_) => 1,
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
            | TypeKind::Never => 0,
            TypeKind::List(inner)
            | TypeKind::Optional(inner)
            | TypeKind::Maybe(inner)
            | TypeKind::Patch { target: inner, .. } => {
                self.thir_universe_with_subst_seen(inner, subst, alias_seen)
            }
            TypeKind::Record(fields, _) => fields
                .into_iter()
                .map(|field| self.thir_universe_with_subst_seen(field.ty, subst, alias_seen))
                .max()
                .unwrap_or(0),
            TypeKind::Union(variants, _) => variants
                .into_iter()
                .filter_map(|variant| variant.payload)
                .map(|payload| self.thir_universe_with_subst_seen(payload, subst, alias_seen))
                .max()
                .unwrap_or(0),
            TypeKind::Tuple(items) => items
                .into_iter()
                .map(|item| match item {
                    TypeTupleItem::Named { ty, .. } | TypeTupleItem::Positional(ty) => {
                        self.thir_universe_with_subst_seen(ty, subst, alias_seen)
                    }
                })
                .max()
                .unwrap_or(0),
            TypeKind::Function { from, to } => self
                .thir_universe_with_subst_seen(from, subst, alias_seen)
                .max(self.thir_universe_with_subst_seen(to, subst, alias_seen)),
            TypeKind::Effect { base, row } => row.ops.into_iter().fold(
                self.thir_universe_with_subst_seen(base, subst, alias_seen),
                |acc, op| {
                    acc.max(self.thir_universe_with_subst_seen(op.param, subst, alias_seen))
                        .max(self.thir_universe_with_subst_seen(op.result, subst, alias_seen))
                },
            ),
            TypeKind::TypeVar(binding) => {
                let Some(subst_ty) = subst.get(&binding).copied() else {
                    return match self.thir.type_param_kinds.get(&binding) {
                        Some(ThirKind::Type(UniverseLevel::Known(level))) => *level,
                        _ => 0,
                    };
                };
                if matches!(
                    self.thir.type_arena.get(subst_ty.0 as usize).map(|t| &t.kind),
                    Some(TypeKind::TypeVar(b)) if *b == binding
                ) {
                    return self
                        .thir
                        .type_universes
                        .get(subst_ty.0 as usize)
                        .copied()
                        .unwrap_or(0);
                }
                if !alias_seen.insert(binding) {
                    return self
                        .thir
                        .type_universes
                        .get(ty.0 as usize)
                        .copied()
                        .unwrap_or(0);
                }
                let result = self.thir_universe_with_subst_seen(subst_ty, subst, alias_seen);
                alias_seen.remove(&binding);
                result
            }
            TypeKind::InferVar(_) => self
                .thir
                .type_universes
                .get(ty.0 as usize)
                .copied()
                .unwrap_or(0),
            TypeKind::Alias(binding) => {
                if !alias_seen.insert(binding) {
                    return self
                        .thir
                        .type_universes
                        .get(ty.0 as usize)
                        .copied()
                        .unwrap_or(0);
                }
                // O(decls): consider a binding→decl index if this becomes hot.
                let level = self
                    .thir
                    .decls
                    .iter()
                    .find_map(|&decl_id| {
                        let decl = &self.thir.decl_arena[decl_id];
                        if decl.binding == binding
                            && let ThirDeclKind::TypeAlias { params, ty } = &decl.kind
                            && params.is_empty()
                        {
                            Some(*ty)
                        } else {
                            None
                        }
                    })
                    .map(|body| self.thir_universe_with_subst_seen(body, subst, alias_seen))
                    .unwrap_or(0);
                alias_seen.remove(&binding);
                level
            }
            TypeKind::AliasApply { binding, args } => {
                if !alias_seen.insert(binding) {
                    return self
                        .thir
                        .type_universes
                        .get(ty.0 as usize)
                        .copied()
                        .unwrap_or(0);
                }
                // O(decls): consider a binding→decl index if this becomes hot.
                let level = self
                    .thir
                    .decls
                    .iter()
                    .find_map(|&decl_id| {
                        let decl = &self.thir.decl_arena[decl_id];
                        if decl.binding == binding
                            && let ThirDeclKind::TypeAlias { params, ty } = &decl.kind
                        {
                            Some((params.clone(), *ty))
                        } else {
                            None
                        }
                    })
                    .filter(|(params, _)| params.len() == args.len())
                    .map(|(params, body)| {
                        let mut next_subst = subst.clone();
                        for (param, arg) in params.into_iter().zip(args) {
                            next_subst.insert(param, arg);
                        }
                        self.thir_universe_with_subst_seen(body, &next_subst, alias_seen)
                    })
                    .unwrap_or_else(|| {
                        self.thir
                            .type_universes
                            .get(ty.0 as usize)
                            .copied()
                            .unwrap_or(0)
                    });
                alias_seen.remove(&binding);
                level
            }
            TypeKind::ForAll { body, .. } => {
                self.thir_universe_with_subst_seen(body, subst, alias_seen)
            }
            TypeKind::Apply { .. } | TypeKind::Con(_) | TypeKind::Error => self
                .thir
                .type_universes
                .get(ty.0 as usize)
                .copied()
                .unwrap_or(0),
        }
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
        let mut mapping: FxHashMap<u32, TypeId> = FxHashMap::default();
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

    fn match_types(&self, template: TypeId, instance: TypeId, out: &mut FxHashMap<u32, TypeId>) {
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
