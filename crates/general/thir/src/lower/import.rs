use zutai_hir::{HirExprId, HirImportSource};
use zutai_syntax::Span;

use crate::import::{ImportedTupleItem, ImportedType};
use crate::ir::{
    RowTail, ThirExpr, ThirExprId, ThirExprKind, Type, TypeId, TypeKind, TypeRecordField,
    TypeTupleItem,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    /// Lower an `import` expression by looking up its pre-resolved type.
    ///
    /// Resolution (filesystem read + `.zti` parse + type derivation) happens in
    /// the semantic layer; here we only intern the resolved descriptor.  An
    /// import the resolver could not handle (missing file, `.zt` import, no base
    /// directory, …) is absent from the map and becomes an `Error` node, so the
    /// eval gate refuses to run it.
    pub(super) fn lower_import_expr(
        &mut self,
        id: HirExprId,
        source: &HirImportSource,
        span: Span,
    ) -> ThirExprId {
        match self.imports.get(source).cloned() {
            Some(desc) => {
                let ty = self.intern_imported_type_with_source(&desc, Some(source), span);
                self.alloc_expr(ThirExpr {
                    source: id,
                    ty,
                    kind: ThirExprKind::Import(source.clone()),
                    span,
                })
            }
            None => self.unsupported_expr(id, "imports", span),
        }
    }

    /// Intern a neutral [`ImportedType`] descriptor into the THIR type arena.
    ///
    /// `source` is the import key of the module being interned; it is `Some`
    /// only when called from `lower_import_expr` (not from recursive calls on
    /// nested non-import types).  It is used to register denotations for
    /// `ImportedType::Type` fields so that annotation-position access works.
    pub(super) fn intern_imported_type_with_source(
        &mut self,
        desc: &ImportedType,
        source: Option<&HirImportSource>,
        span: Span,
    ) -> TypeId {
        match desc {
            ImportedType::Bool => self.bool_type(span),
            ImportedType::Int => self.int_type(span),
            ImportedType::Float => self.float_type(span),
            ImportedType::FixedNum(fw) => self.fixed_num_type(*fw, span),
            ImportedType::Text => self.text_type(span),
            ImportedType::Atom(name) => self.alloc_type(Type {
                kind: TypeKind::Atom(name.clone()),
                span,
            }),
            ImportedType::List(inner) => {
                let inner_ty = self.intern_imported_type_with_source(inner, source, span);
                self.alloc_type(Type {
                    kind: TypeKind::List(inner_ty),
                    span,
                })
            }
            ImportedType::Optional(inner) => {
                let inner_ty = self.intern_imported_type_with_source(inner, source, span);
                self.optional_type(inner_ty, span)
            }
            ImportedType::Maybe(inner) => {
                let inner_ty = self.intern_imported_type_with_source(inner, source, span);
                self.maybe_type(inner_ty, span)
            }
            ImportedType::Record(fields) => {
                let mut thir_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    let ty = if let ImportedType::Type(inner) = &field.ty {
                        // This field carries a type-value.  Intern the denotation
                        // separately and register it so annotation-position access
                        // (`x : moduleLib.SomeType`) can recover the concrete type.
                        let denotation = self.intern_imported_type_with_source(inner, source, span);
                        if let Some(src) = source {
                            self.import_type_denotations
                                .insert((src.clone(), field.name.clone()), denotation);
                        }
                        self.type_type
                    } else {
                        self.intern_imported_type_with_source(&field.ty, source, span)
                    };
                    thir_fields.push(TypeRecordField {
                        name: field.name.clone(),
                        optional: field.optional,
                        ty,
                        span,
                    });
                }
                self.alloc_type(Type {
                    kind: TypeKind::Record(thir_fields, RowTail::Closed),
                    span,
                })
            }
            ImportedType::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        ImportedTupleItem::Named { name, ty } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.intern_imported_type_with_source(ty, source, span),
                            span,
                        },
                        ImportedTupleItem::Positional(ty) => TypeTupleItem::Positional(
                            self.intern_imported_type_with_source(ty, source, span),
                        ),
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(items),
                    span,
                })
            }
            ImportedType::Union(variants) => {
                let variants = variants
                    .iter()
                    .map(|v| crate::ir::UnionVariant {
                        name: v.name.clone(),
                        payload: v
                            .payload
                            .as_deref()
                            .map(|p| self.intern_imported_type_with_source(p, source, span)),
                        span,
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Union(variants, RowTail::Closed),
                    span,
                })
            }
            ImportedType::Function { from, to } => {
                let from = self.intern_imported_type_with_source(from, source, span);
                let to = self.intern_imported_type_with_source(to, source, span);
                self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span,
                })
            }
            ImportedType::Type(_) => {
                // A `Type`-kinded value at top-level (not in a record field).
                // No denotation registration here — the field-name context is
                // unavailable.  Just return `type_type`.
                self.type_type
            }
            ImportedType::Unknown => self.fresh_infer_var(span),
        }
    }
}
