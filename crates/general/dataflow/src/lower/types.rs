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
                if let Some(body) = self.type_aliases.get(&binding).copied() {
                    self.lower_type(body)
                } else {
                    self.types.alloc(DfTy::TyVar(DfTyVar::Named(binding.0)))
                }
            }
            TlcType::TyVar(v, _) => self.types.alloc(DfTy::TyVar(lower_tyvar(v))),

            TlcType::TyApp(f, arg) => {
                let df = self.lower_type(f);
                let darg = self.lower_type(arg);
                self.types.alloc(DfTy::TyApp(df, vec![darg]))
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
}
