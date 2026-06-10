use zutai_hir::{HirExprId, HirImportSource};
use zutai_syntax::Span;

use crate::import::{ImportedTupleItem, ImportedType};
use crate::ir::{
    ThirExpr, ThirExprId, ThirExprKind, Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
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
                let ty = self.intern_imported_type(&desc, span);
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
    fn intern_imported_type(&mut self, desc: &ImportedType, span: Span) -> TypeId {
        match desc {
            ImportedType::Bool => self.bool_type(span),
            ImportedType::Int => self.int_type(span),
            ImportedType::Float => self.float_type(span),
            ImportedType::Text => self.text_type(span),
            ImportedType::Atom(name) => self.alloc_type(Type {
                kind: TypeKind::Atom(name.clone()),
                span,
            }),
            ImportedType::List(inner) => {
                let inner_ty = self.intern_imported_type(inner, span);
                self.alloc_type(Type {
                    kind: TypeKind::List(inner_ty),
                    span,
                })
            }
            ImportedType::Optional(inner) => {
                let inner_ty = self.intern_imported_type(inner, span);
                self.optional_type(inner_ty, span)
            }
            ImportedType::Record(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| {
                        let ty = self.intern_imported_type(&field.ty, span);
                        TypeRecordField {
                            name: field.name.clone(),
                            optional: field.optional,
                            ty,
                            span,
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Record(fields),
                    span,
                })
            }
            ImportedType::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        ImportedTupleItem::Named { name, ty } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.intern_imported_type(ty, span),
                            span,
                        },
                        ImportedTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.intern_imported_type(ty, span))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(items),
                    span,
                })
            }
            ImportedType::Union(items) => {
                let items = items
                    .iter()
                    .map(|item| self.intern_imported_type(item, span))
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Union(items),
                    span,
                })
            }
            ImportedType::Unknown => self.fresh_infer_var(span),
        }
    }
}
