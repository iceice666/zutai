use std::collections::{HashMap, HashSet};
use zutai_hir::BindingId;
use zutai_tlc::{Literal as TlcLit, PrimTy, TlcTupleField, TlcType, TlcTypeId, TlcTypeVar};

use crate::{DfRecordField, DfTupleField, DfTy, DfTyId, DfTyVar, DfUnionVariant};

use super::*;

impl<'m> Lowerer<'m> {
    // ── Type lowering ─────────────────────────────────────────────────────────

    pub(super) fn lower_type(&mut self, id: TlcTypeId) -> DfTyId {
        if let Some(&cached) = self.type_cache.get(&id) {
            return cached;
        }
        // Clone to release the borrow on self.module before calling lower_type recursively.
        let ty = self.module.type_arena[id].clone();
        let result = self.lower_type_owned(ty);
        self.type_cache.insert(id, result);
        result
    }

    pub(super) fn lower_type_owned(&mut self, ty: TlcType) -> DfTyId {
        match ty {
            TlcType::Prim(PrimTy::Int) => self.types.alloc(DfTy::Int),
            TlcType::Prim(PrimTy::Float) => self.types.alloc(DfTy::Float),
            TlcType::Prim(PrimTy::FixedNum(fw)) => {
                let ty = if fw.is_float() {
                    DfTy::Float
                } else {
                    DfTy::Int
                };
                self.types.alloc(ty)
            }
            TlcType::Prim(PrimTy::Posit(spec)) => self.types.alloc(DfTy::Posit(spec)),
            TlcType::Opaque(name) => self.types.alloc(DfTy::Opaque(name)),
            TlcType::Prim(PrimTy::Bool) => self.types.alloc(DfTy::Bool),
            TlcType::Prim(PrimTy::Str) => self.types.alloc(DfTy::Text),
            TlcType::Prim(PrimTy::Atom) => self.types.alloc(DfTy::Atom),
            TlcType::Prim(PrimTy::Nothing) => self.types.alloc(DfTy::Error),

            TlcType::Singleton(TlcLit::Bool(true)) => self.types.alloc(DfTy::True),
            TlcType::Singleton(TlcLit::Bool(false)) => self.types.alloc(DfTy::False),
            // Atom singletons (used for union-arm discrimination) lower to the generic
            // Atom primitive — DC's type system has no singleton-Atom variant.
            TlcType::Singleton(TlcLit::Atom(_)) => self.types.alloc(DfTy::Atom),
            TlcType::Singleton(TlcLit::Posit(literal)) => {
                self.types.alloc(DfTy::Posit(literal.spec))
            }
            TlcType::Singleton(TlcLit::Int(_)) => self.types.alloc(DfTy::Int),
            TlcType::Singleton(TlcLit::Float(_)) => self.types.alloc(DfTy::Float),
            TlcType::Singleton(TlcLit::Str(_)) => self.types.alloc(DfTy::Text),
            // Nothing has no DC runtime type representation.
            TlcType::Singleton(TlcLit::Nothing) => self.types.alloc(DfTy::Error),

            TlcType::Fun(a, b, _eff) => {
                let da = self.lower_type(a);
                let db = self.lower_type(b);
                self.types.alloc(DfTy::Fun(da, db))
            }

            TlcType::ForAll(v, _, body) => {
                let dv = lower_tyvar(v);
                let dbody = self.lower_type(body);
                self.types.alloc(DfTy::TyFun(vec![dv], dbody))
            }

            TlcType::TyVar(TlcTypeVar::Named(binding), _) => {
                let binding = BindingId(binding);
                // Permanent per-binding cache + equirecursive guard.  All
                // `TyVar(Named(binding))` occurrences — regardless of their TlcTypeId —
                // resolve to the same DfTyId so the DfTy arena stays consistent.
                // During body lowering the slot holds a placeholder `DfTy::Error`
                // back-reference; after lowering the slot is overwritten with the real
                // body content, forming a finite self-referential cycle.
                if let Some(&place) = self.alias_binding_type.get(&binding) {
                    return place;
                }
                if let Some(body) = self.type_aliases.get(&binding).copied() {
                    // Reserve a slot.  Recursive back-edges inside `lower_type(body)`
                    // will re-enter this arm, find `place` in the cache, and return it.
                    let place = self.types.alloc(DfTy::Error);
                    self.alias_binding_type.insert(binding, place);
                    let lowered = self.lower_type(body);
                    // Backpatch: overwrite the placeholder with the lowered body so all
                    // back-edges pointing at `place` now see the correct type.  The
                    // entry remains in `alias_binding_type` — it becomes the canonical
                    // DfTyId for this alias for all future lookups.
                    let body_ty = self.types[lowered].clone();
                    self.types[place] = body_ty;
                    place
                } else {
                    self.types.alloc(DfTy::TyVar(DfTyVar::Named(binding.0)))
                }
            }
            TlcType::TyVar(v, _) => self.types.alloc(DfTy::TyVar(lower_tyvar(v))),

            TlcType::TyApp(f, arg) => {
                let df = self.lower_type(f);
                let darg = self.lower_type(arg);
                self.apply_df_type(df, vec![darg])
            }

            TlcType::TyLamK(v, _, body) => {
                let dv = lower_tyvar(v);
                let dbody = self.lower_type(body);
                self.types.alloc(DfTy::TyFun(vec![dv], dbody))
            }

            TlcType::Record(row) => {
                // Collect field data (copy TlcTypeIds out) before calling lower_type.
                let field_data: Vec<(String, bool, TlcTypeId)> = row_to_fields(&row);
                let mut df_fields: Vec<DfRecordField> = field_data
                    .into_iter()
                    .map(|(name, optional, ty_id)| DfRecordField {
                        name,
                        optional,
                        ty: self.lower_type(ty_id),
                    })
                    .collect();
                df_fields.sort_by(|a, b| a.name.cmp(&b.name));
                self.types.alloc(DfTy::Record(df_fields))
            }

            TlcType::VariantT(row) => {
                let variants: Vec<(String, TlcTypeId)> = row
                    .fields()
                    .map(|(tag, ty)| (tag.to_string(), ty))
                    .collect();
                let df_variants: Vec<DfUnionVariant> = variants
                    .into_iter()
                    .map(|(tag, ty)| DfUnionVariant {
                        tag,
                        ty: self.lower_type(ty),
                    })
                    .collect();
                self.types.alloc(DfTy::Union(df_variants))
            }

            TlcType::Tuple(fields) => {
                let df_fields: Vec<DfTupleField> = fields
                    .into_iter()
                    .map(|f| match f {
                        TlcTupleField::Named { name, ty } => DfTupleField::Named {
                            name,
                            ty: self.lower_type(ty),
                        },
                        TlcTupleField::Positional(ty) => {
                            DfTupleField::Positional(self.lower_type(ty))
                        }
                    })
                    .collect();
                self.types.alloc(DfTy::Tuple(df_fields))
            }

            TlcType::List(t) => {
                let dt = self.lower_type(t);
                self.types.alloc(DfTy::List(dt))
            }

            TlcType::Optional(t) => {
                let dt = self.lower_type(t);
                self.types.alloc(DfTy::Optional(dt))
            }

            TlcType::Maybe(t) => {
                let dt = self.lower_type(t);
                self.types.alloc(DfTy::Maybe(dt))
            }
        }
    }

    fn apply_df_type(&mut self, func: DfTyId, args: Vec<DfTyId>) -> DfTyId {
        let DfTy::TyFun(params, body) = self.types[func].clone() else {
            return self.types.alloc(DfTy::TyApp(func, args));
        };
        if params.len() != args.len() {
            return self.types.alloc(DfTy::TyApp(func, args));
        }

        let key = (func, args.clone());
        if let Some(&cached) = self.type_app_cache.get(&key) {
            return cached;
        }

        const MAX_TYPE_APP_DEPTH: u32 = 128;
        if self.type_app_depth >= MAX_TYPE_APP_DEPTH {
            return self.error_ty;
        }
        self.type_app_depth += 1;

        let place = self.types.alloc(DfTy::Error);
        self.type_app_cache.insert(key, place);
        let subst: HashMap<DfTyVar, DfTyId> = params.into_iter().zip(args).collect();
        let lowered = self.instantiate_df_type(body, &subst, &mut HashMap::new());
        let body_ty = self.types[lowered].clone();
        self.types[place] = body_ty;
        self.type_app_depth -= 1;
        place
    }

    fn instantiate_df_type(
        &mut self,
        ty: DfTyId,
        subst: &HashMap<DfTyVar, DfTyId>,
        memo: &mut HashMap<DfTyId, DfTyId>,
    ) -> DfTyId {
        if !self.df_ty_mentions(ty, subst, &mut HashSet::new()) {
            return ty;
        }
        match self.types[ty].clone() {
            DfTy::TyVar(var) => return subst.get(&var).copied().unwrap_or(ty),
            DfTy::TyApp(func, args) => {
                let func = match self.types[func].clone() {
                    DfTy::TyVar(var) => subst.get(&var).copied().unwrap_or(func),
                    _ => func,
                };
                let args = args
                    .into_iter()
                    .map(|arg| self.instantiate_df_type(arg, subst, memo))
                    .collect();
                return self.apply_df_type(func, args);
            }
            DfTy::Int
            | DfTy::Float
            | DfTy::Posit(_)
            | DfTy::Bool
            | DfTy::Text
            | DfTy::Atom
            | DfTy::True
            | DfTy::False
            | DfTy::Type
            | DfTy::Error => return ty,
            _ => {}
        }

        if let Some(&cached) = memo.get(&ty) {
            return cached;
        }
        let place = self.types.alloc(DfTy::Error);
        memo.insert(ty, place);

        let instantiated = match self.types[ty].clone() {
            DfTy::List(inner) => {
                let inner = self.instantiate_df_type(inner, subst, memo);
                DfTy::List(inner)
            }
            DfTy::Optional(inner) => {
                let inner = self.instantiate_df_type(inner, subst, memo);
                DfTy::Optional(inner)
            }
            DfTy::Maybe(inner) => {
                let inner = self.instantiate_df_type(inner, subst, memo);
                DfTy::Maybe(inner)
            }
            DfTy::Record(fields) => {
                let fields = fields
                    .into_iter()
                    .map(|mut field| {
                        field.ty = self.instantiate_df_type(field.ty, subst, memo);
                        field
                    })
                    .collect();
                DfTy::Record(fields)
            }
            DfTy::Union(variants) => {
                let variants = variants
                    .into_iter()
                    .map(|mut variant| {
                        variant.ty = self.instantiate_df_type(variant.ty, subst, memo);
                        variant
                    })
                    .collect();
                DfTy::Union(variants)
            }
            DfTy::Tuple(fields) => {
                let fields = fields
                    .into_iter()
                    .map(|field| match field {
                        DfTupleField::Named { name, ty } => DfTupleField::Named {
                            name,
                            ty: self.instantiate_df_type(ty, subst, memo),
                        },
                        DfTupleField::Positional(ty) => {
                            DfTupleField::Positional(self.instantiate_df_type(ty, subst, memo))
                        }
                    })
                    .collect();
                DfTy::Tuple(fields)
            }
            DfTy::Fun(from, to) => {
                let from = self.instantiate_df_type(from, subst, memo);
                let to = self.instantiate_df_type(to, subst, memo);
                DfTy::Fun(from, to)
            }
            DfTy::TyFun(params, body) => {
                let mut nested = subst.clone();
                for param in &params {
                    nested.remove(param);
                }
                let body = self.instantiate_df_type(body, &nested, memo);
                DfTy::TyFun(params, body)
            }
            DfTy::TyApp(_, _)
            | DfTy::TyVar(_)
            | DfTy::Int
            | DfTy::Float
            | DfTy::Posit(_)
            | DfTy::Bool
            | DfTy::Text
            | DfTy::Opaque(_)
            | DfTy::Atom
            | DfTy::True
            | DfTy::False
            | DfTy::Type
            | DfTy::Error => unreachable!("handled before memoization"),
        };
        self.types[place] = instantiated;
        place
    }

    /// Does `ty`'s (possibly cyclic) subtree mention any variable bound by `subst`?
    /// Conservative: ignores `TyFun` param shadowing (over-reporting only costs a
    /// redundant instantiation, which still substitutes correctly). `visited`
    /// guards the equirecursive cycles in the DfTy arena.
    fn df_ty_mentions(
        &self,
        ty: DfTyId,
        subst: &HashMap<DfTyVar, DfTyId>,
        visited: &mut HashSet<DfTyId>,
    ) -> bool {
        if !visited.insert(ty) {
            return false;
        }
        match self.types[ty].clone() {
            DfTy::TyVar(v) => subst.contains_key(&v),
            DfTy::List(t) | DfTy::Optional(t) | DfTy::Maybe(t) => {
                self.df_ty_mentions(t, subst, visited)
            }
            DfTy::Record(fields) => fields
                .iter()
                .any(|f| self.df_ty_mentions(f.ty, subst, visited)),
            DfTy::Union(variants) => variants
                .iter()
                .any(|v| self.df_ty_mentions(v.ty, subst, visited)),
            DfTy::Tuple(fields) => fields.iter().any(|f| {
                let t = match f {
                    DfTupleField::Named { ty, .. } | DfTupleField::Positional(ty) => *ty,
                };
                self.df_ty_mentions(t, subst, visited)
            }),
            DfTy::Fun(a, b) => {
                self.df_ty_mentions(a, subst, visited) || self.df_ty_mentions(b, subst, visited)
            }
            DfTy::TyApp(f, args) => {
                self.df_ty_mentions(f, subst, visited)
                    || args.iter().any(|&a| self.df_ty_mentions(a, subst, visited))
            }
            DfTy::TyFun(_, body) => self.df_ty_mentions(body, subst, visited),
            _ => false,
        }
    }
}
