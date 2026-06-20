use std::collections::{HashMap, HashSet};

use zutai_hir::BindingId;
use zutai_thir::{RowTail, ThirDeclKind, TypeId, TypeKind, TypeTupleItem};

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
            TypeKind::Bool => self.alloc_type(TlcType::Prim(PrimTy::Bool)),
            // Singleton types — preserve discrimination (Phase 0 bug fix).
            TypeKind::True => self.alloc_type(TlcType::Singleton(Literal::Bool(true))),
            TypeKind::False => self.alloc_type(TlcType::Singleton(Literal::Bool(false))),
            // Atom with its symbol payload (Phase 0 bug fix).
            TypeKind::Atom(sym) => self.alloc_type(TlcType::Singleton(Literal::Atom(sym))),
            TypeKind::Text => self.alloc_type(TlcType::Prim(PrimTy::Str)),
            TypeKind::Function { from, to } => {
                let from_tlc = self.lower_type(from);
                let to_tlc = self.lower_type(to);
                // v0: every function is pure — effect row defaults to REmpty (spec §4 line 171).
                self.alloc_type(TlcType::Fun(from_tlc, to_tlc, Row::REmpty))
            }
            TypeKind::List(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::List(inner_tlc))
            }
            TypeKind::Optional(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::Optional(inner_tlc))
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
            TypeKind::List(inner) | TypeKind::Optional(inner) => {
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
            _ => {}
        }
    }
}
