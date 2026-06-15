use std::collections::HashMap;

use zutai_hir::BindingId;
use zutai_thir::{ThirDeclKind, TypeId, TypeKind, TypeTupleItem};

use crate::ir::{PrimTy, TlcRecordField, TlcTupleField, TlcType, TlcTypeId, TlcTypeVar};

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
            TypeKind::Bool | TypeKind::True | TypeKind::False => {
                self.alloc_type(TlcType::Prim(PrimTy::Bool))
            }
            TypeKind::Text => self.alloc_type(TlcType::Prim(PrimTy::Str)),
            TypeKind::Atom(_) => self.alloc_type(TlcType::Prim(PrimTy::Atom)),
            TypeKind::Function { from, to } => {
                let from_tlc = self.lower_type(from);
                let to_tlc = self.lower_type(to);
                self.alloc_type(TlcType::Fun(from_tlc, to_tlc))
            }
            TypeKind::List(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::List(inner_tlc))
            }
            TypeKind::Optional(inner) => {
                let inner_tlc = self.lower_type(inner);
                self.alloc_type(TlcType::Optional(inner_tlc))
            }
            TypeKind::Record(fields) => {
                let tlc_fields: Vec<TlcRecordField> = fields
                    .iter()
                    .map(|f| TlcRecordField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: self.lower_type(f.ty),
                    })
                    .collect();
                self.alloc_type(TlcType::Record(tlc_fields))
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
            TypeKind::Union(_) => self.alloc_type(TlcType::Record(Vec::new())),
            TypeKind::TypeVar(binding) => {
                let tyvar = self.named_tyvar(binding);
                self.alloc_type(TlcType::TyVar(tyvar))
            }
            TypeKind::InferVar(v) => {
                let tyvar = self.inferred_tyvar(v);
                self.alloc_type(TlcType::TyVar(tyvar))
            }
            TypeKind::Alias(binding) => match self.thir_alias_body(binding) {
                Some(body) => self.lower_type(body),
                None => self.alloc_type(TlcType::Record(Vec::new())),
            },
            TypeKind::AliasApply { binding, args } => {
                match self.expand_alias_apply(binding, &args) {
                    Some(expanded) => self.lower_type(expanded),
                    None => self.alloc_type(TlcType::Record(Vec::new())),
                }
            }
            TypeKind::Type | TypeKind::Error => self.alloc_type(TlcType::Record(Vec::new())),
        };
        self.type_cache.insert(resolved.0, tlc_ty);
        tlc_ty
    }

    fn resolve_thir(&self, ty: TypeId) -> TypeId {
        ty
    }

    fn thir_alias_body(&self, binding: BindingId) -> Option<TypeId> {
        for &decl_id in &self.thir.decls {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == binding {
                if let ThirDeclKind::TypeAlias { ty, .. } = decl.kind {
                    return Some(ty);
                }
            }
        }
        None
    }

    fn expand_alias_apply(&self, binding: BindingId, args: &[TypeId]) -> Option<TypeId> {
        for &decl_id in &self.thir.decls {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == binding {
                if let ThirDeclKind::TypeAlias { ref params, ty } = decl.kind {
                    if params.len() != args.len() {
                        return None;
                    }
                    return Some(self.substitute_type_vars(ty, params, args));
                }
            }
        }
        None
    }

    fn substitute_type_vars(&self, ty: TypeId, params: &[BindingId], args: &[TypeId]) -> TypeId {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) => {
                if let Some(pos) = params.iter().position(|&p| p == b) {
                    args[pos]
                } else {
                    ty
                }
            }
            _ => ty,
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
        let mut mapping: HashMap<u32, TypeId> = HashMap::new();
        self.match_types(scheme_ty, ref_ty, &mut mapping);
        scheme_vars
            .iter()
            .map(|&v| {
                let tlc_ty = if let Some(&concrete) = mapping.get(&v) {
                    self.lower_type(concrete)
                } else {
                    let tyvar = self.inferred_tyvar(v);
                    self.alloc_type(TlcType::TyVar(tyvar))
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
