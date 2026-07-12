use rustc_hash::{FxHashMap, FxHashSet};
use zutai_hir::{BindingId, BindingKind, HirExprId, HirImportSource};
use zutai_syntax::Span;

use crate::import::{
    ImportedProvenance, ImportedProvenanceChildren, ImportedRowTail, ImportedTupleItem,
    ImportedType, ImportedTypeOrigin,
};
use crate::ir::{
    EffectOp, EffectRow, Kind, RowTail, ThirDecl, ThirDeclKind, ThirExpr, ThirExprId, ThirExprKind,
    Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn check_heterogeneous_import_lists(
        &mut self,
        source: &HirImportSource,
        expected: TypeId,
        boundary_span: Span,
    ) {
        let Some(provenance) = self.import_provenance.get(source).cloned() else {
            return;
        };
        self.check_heterogeneous_lists_in_provenance(source, expected, &provenance, boundary_span);
    }

    fn check_heterogeneous_lists_in_provenance(
        &mut self,
        source: &HirImportSource,
        expected: TypeId,
        provenance: &ImportedProvenance,
        boundary_span: Span,
    ) {
        let expected_span = self.type_arena[expected.0 as usize].span;
        let expected = self.resolve_alias(expected, &mut FxHashSet::default(), expected_span);
        let expected_kind = self.type_arena[expected.0 as usize].kind.clone();
        match (&provenance.children, expected_kind) {
            (ImportedProvenanceChildren::Record(fields), TypeKind::Record(expected_fields, _)) => {
                for field in fields {
                    if let Some(expected_field) = expected_fields
                        .iter()
                        .find(|candidate| candidate.name == field.name)
                    {
                        self.check_heterogeneous_lists_in_provenance(
                            source,
                            expected_field.ty,
                            &field.value,
                            boundary_span,
                        );
                    }
                }
            }
            (ImportedProvenanceChildren::List(items), TypeKind::List(expected_item)) => {
                if matches!(provenance.ty, ImportedType::List(ref item) if matches!(**item, ImportedType::Unknown))
                {
                    for item in items {
                        let found = self.intern_imported_type_with_source(
                            &item.ty,
                            Some(source),
                            boundary_span,
                            Some(item),
                        );
                        if !self.type_matches(expected_item, found) {
                            self.type_mismatch(expected_item, found, boundary_span);
                        }
                    }
                }
                for item in items {
                    self.check_heterogeneous_lists_in_provenance(
                        source,
                        expected_item,
                        item,
                        boundary_span,
                    );
                }
            }
            _ => {}
        }
    }

    /// Lower an internal import node by looking up its pre-resolved type.
    ///
    /// Resolution (filesystem read + `.zti` parse + type derivation) happens in
    /// the semantic layer; here we only intern the resolved descriptor. An
    /// import the resolver could not handle (missing file, no base directory,
    /// unsupported path form, …) is absent from the map and becomes an `Error`
    /// node, so the eval gate refuses to run it.
    pub(super) fn lower_import_expr(
        &mut self,
        id: HirExprId,
        source: &HirImportSource,
        span: Span,
    ) -> ThirExprId {
        match self.imports.get(source).cloned() {
            Some(desc) => {
                let provenance = self.import_provenance.get(source).cloned();
                self.import_tyvar_cache.clear();
                self.import_rowvar_cache = self
                    .import_rowvar_caches
                    .get(source)
                    .cloned()
                    .unwrap_or_default();
                let ty = self.intern_imported_type_with_source(
                    &desc,
                    Some(source),
                    span,
                    provenance.as_ref(),
                );
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
        provenance: Option<&ImportedProvenance>,
    ) -> TypeId {
        let ty = match desc {
            ImportedType::Bool => self.bool_type(span),
            ImportedType::Int => self.int_type(span),
            ImportedType::Float => self.float_type(span),
            ImportedType::FixedNum(fw) => self.fixed_num_type(*fw, span),
            ImportedType::Posit(spec) => self.posit_type(*spec, span),
            ImportedType::Text => self.text_type(span),
            ImportedType::Opaque(name) => self.alloc_type(Type {
                kind: TypeKind::Opaque(name.clone()),
                span,
            }),
            ImportedType::Atom(name) => self.alloc_type(Type {
                kind: TypeKind::Atom(name.clone()),
                span,
            }),
            ImportedType::List(inner) => {
                let item_provenance =
                    provenance.and_then(|provenance| match &provenance.children {
                        ImportedProvenanceChildren::List(items) => items.first(),
                        _ => None,
                    });
                let inner_ty =
                    self.intern_imported_type_with_source(inner, source, span, item_provenance);
                self.alloc_type(Type {
                    kind: TypeKind::List(inner_ty),
                    span,
                })
            }
            ImportedType::Optional(inner) => {
                let inner_ty = self.intern_imported_type_with_source(inner, source, span, None);
                self.optional_type(inner_ty, span)
            }
            ImportedType::Maybe(inner) => {
                let inner_ty = self.intern_imported_type_with_source(inner, source, span, None);
                self.maybe_type(inner_ty, span)
            }
            ImportedType::Record(fields) => {
                // Pass A: predeclare every imported parametric type constructor in
                // this record so a sibling/recursive `ConApply` resolves while
                // interning bodies in pass B. Done only while predeclaring an
                // import decl (so each constructor is defined exactly once, with a
                // real HIR source for its materialized alias decl).
                let mut pending_ctors: FxHashMap<String, FxHashMap<u32, BindingId>> =
                    FxHashMap::default();
                if let Some(src) = source
                    && self.current_import_decl.is_some()
                {
                    for field in fields {
                        if let ImportedType::Type(inner) = &field.ty
                            && let ImportedType::TypeCon { params, .. } = &**inner
                            && !self
                                .import_type_constructors
                                .contains_key(&(src.clone(), field.name.clone()))
                        {
                            let param_map =
                                self.predeclare_imported_constructor(src, &field.name, params);
                            pending_ctors.insert(field.name.clone(), param_map);
                        }
                    }
                }

                // Pass B: intern field types, finalizing constructor bodies now
                // that every constructor in the record is declared.
                let mut thir_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    let field_provenance = provenance.and_then(|provenance| {
                        let ImportedProvenanceChildren::Record(fields) = &provenance.children
                        else {
                            return None;
                        };
                        fields
                            .iter()
                            .find(|candidate| candidate.name == field.name)
                            .map(|field| &field.value)
                    });
                    let ty = if let ImportedType::Type(inner) = &field.ty {
                        match &**inner {
                            ImportedType::TypeCon { body, .. } => {
                                if let Some(param_map) = pending_ctors.remove(&field.name) {
                                    let src = source.expect("ctor predeclared only with a source");
                                    self.finalize_imported_constructor(
                                        &field.name,
                                        param_map,
                                        body,
                                        src,
                                        span,
                                    );
                                }
                                self.type_type
                            }
                            // A non-parametric type-alias denotation (the
                            // `serverLib.Server` path). Intern it and register it so
                            // annotation-position access recovers the concrete type.
                            _ => {
                                let denotation = self.intern_imported_type_with_source(
                                    inner,
                                    source,
                                    span,
                                    field_provenance,
                                );
                                if let Some(src) = source {
                                    self.import_type_denotations
                                        .insert((src.clone(), field.name.clone()), denotation);
                                }
                                self.type_type
                            }
                        }
                    } else {
                        self.intern_imported_type_with_source(
                            &field.ty,
                            source,
                            span,
                            field_provenance,
                        )
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
            ImportedType::WithTypeExports { value, types } => {
                if let Some(src) = source {
                    self.register_imported_type_exports(src, types, span);
                }
                self.intern_imported_type_with_source(value, source, span, provenance)
            }
            ImportedType::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        ImportedTupleItem::Named { name, ty } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.intern_imported_type_with_source(ty, source, span, None),
                            span,
                        },
                        ImportedTupleItem::Positional(ty) => TypeTupleItem::Positional(
                            self.intern_imported_type_with_source(ty, source, span, None),
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
                            .map(|p| self.intern_imported_type_with_source(p, source, span, None)),
                        span,
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Union(variants, RowTail::Closed),
                    span,
                })
            }
            ImportedType::Function { from, to } => {
                let from = self.intern_imported_type_with_source(from, source, span, None);
                let to = self.intern_imported_type_with_source(to, source, span, None);
                self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span,
                })
            }
            ImportedType::Effect { base, ops, tail } => {
                let base = self.intern_imported_type_with_source(base, source, span, None);
                let ops = ops
                    .iter()
                    .map(|op| EffectOp {
                        name: op.name.clone(),
                        param: self.intern_imported_type_with_source(&op.param, source, span, None),
                        result: self
                            .intern_imported_type_with_source(&op.result, source, span, None),
                        span,
                    })
                    .collect();
                let tail = self.intern_imported_row_tail(*tail);
                self.alloc_type(Type {
                    kind: TypeKind::Effect {
                        base,
                        row: EffectRow { ops, tail },
                    },
                    span,
                })
            }
            ImportedType::Type(_) => {
                // A `Type`-kinded value at top-level (not in a record field).
                // No denotation registration here — the field-name context is
                // unavailable.  Just return `type_type`.
                self.type_type
            }
            ImportedType::TyVar(id) => {
                // Inside an imported constructor's body a `TyVar` is one of the
                // constructor's formal parameters: intern it to the rigid
                // `TypeVar` synthetic binding so `instantiate_type_vars`
                // substitutes it on application.
                if let Some(&binding) = self.ctor_param_map.get(id) {
                    self.alloc_type(Type {
                        kind: TypeKind::TypeVar(binding),
                        span,
                    })
                } else if let Some(&ty) = self.import_tyvar_cache.get(id) {
                    // Otherwise it is a value-level Hindley–Milner type parameter:
                    // each exported variable interns to one fresh inference
                    // variable, shared across its occurrences (so `∀A. A -> A`
                    // becomes `?a -> ?a`), then quantified by the import decl pass.
                    ty
                } else {
                    let ty = self.fresh_infer_var(span);
                    self.import_tyvar_cache.insert(*id, ty);
                    ty
                }
            }
            ImportedType::ConApply { ctor, args } => {
                let arg_tys: Vec<TypeId> = args
                    .iter()
                    .map(|a| self.intern_imported_type_with_source(a, source, span, None))
                    .collect();
                let Some(src) = source else {
                    return self.fresh_infer_var(span);
                };
                let Some(&ctor_binding) = self
                    .import_type_constructors
                    .get(&(src.clone(), ctor.clone()))
                else {
                    // The referenced constructor is not exported by this module as
                    // an importable `TypeCon` field (e.g. a value whose type
                    // mentions a private generic, or a cross-module reference).
                    // Degrade to an unconstrained position — exactly as an
                    // un-exportable type already does — so the value stays a safe
                    // opaque pass-through rather than a hard error. Applying such a
                    // constructor is still refused at the use site (`apply.rs`).
                    return self.fresh_infer_var(span);
                };
                let arity = self
                    .alias_params
                    .get(&ctor_binding)
                    .map_or(0, |params| params.len());
                if arg_tys.len() == arity {
                    self.alloc_type(Type {
                        kind: TypeKind::AliasApply {
                            binding: ctor_binding,
                            args: arg_tys,
                        },
                        span,
                    })
                } else {
                    // Partial application: a curried `Apply` spine, left inert by
                    // `resolve_alias` until saturated.
                    let head = self.alias_type(ctor_binding, span);
                    self.fold_apply(head, &arg_tys, span)
                }
            }
            // A bare `TypeCon` outside a record field's denotation flow (e.g. a
            // module whose final expression is a single parametric constructor)
            // is not rebuilt as an applicable constructor in v1; intern like
            // `Unknown` so any application is refused rather than mistyped.
            ImportedType::TypeCon { .. } => self.fresh_infer_var(span),
            ImportedType::Unknown => self.fresh_infer_var(span),
        };
        if let (Some(source), Some(provenance)) = (source, provenance) {
            self.imported_type_origins.insert(
                ty,
                ImportedTypeOrigin {
                    source: source.clone(),
                    span: provenance.span,
                    name_span: provenance.name_span,
                },
            );
        }
        ty
    }

    fn intern_imported_row_tail(&mut self, tail: ImportedRowTail) -> RowTail {
        match tail {
            ImportedRowTail::Closed => RowTail::Closed,
            ImportedRowTail::Open => RowTail::Open,
            ImportedRowTail::Param(id) => {
                let binding = if let Some(&binding) = self.import_rowvar_cache.get(&id) {
                    binding
                } else {
                    let binding = self.alloc_synthetic_binding(
                        format!("__import_row_{id}"),
                        BindingKind::TypeParam,
                    );
                    self.import_rowvar_cache.insert(id, binding);
                    binding
                };
                RowTail::Param(binding)
            }
        }
    }

    fn register_imported_type_exports(
        &mut self,
        source: &HirImportSource,
        fields: &[crate::import::ImportedField],
        span: Span,
    ) {
        let mut pending_ctors: FxHashMap<String, FxHashMap<u32, BindingId>> = FxHashMap::default();
        if self.current_import_decl.is_some() {
            for field in fields {
                if let ImportedType::Type(inner) = &field.ty
                    && let ImportedType::TypeCon { params, .. } = &**inner
                    && !self
                        .import_type_constructors
                        .contains_key(&(source.clone(), field.name.clone()))
                {
                    let param_map =
                        self.predeclare_imported_constructor(source, &field.name, params);
                    pending_ctors.insert(field.name.clone(), param_map);
                }
            }
        }

        for field in fields {
            let ImportedType::Type(inner) = &field.ty else {
                continue;
            };
            match &**inner {
                ImportedType::TypeCon { body, .. } => {
                    if let Some(param_map) = pending_ctors.remove(&field.name) {
                        self.finalize_imported_constructor(
                            &field.name,
                            param_map,
                            body,
                            source,
                            span,
                        );
                    }
                }
                _ => {
                    let denotation =
                        self.intern_imported_type_with_source(inner, Some(source), span, None);
                    self.import_type_denotations
                        .insert((source.clone(), field.name.clone()), denotation);
                }
            }
        }
    }

    /// Pass A of importing a parametric type constructor: mint the synthetic
    /// constructor binding and one synthetic `TypeParam` binding per parameter,
    /// register the constructor's arity (`alias_params`) and its lookup key
    /// (`import_type_constructors`). Returns the export-id → param-binding map
    /// used while interning the body so its `TyVar`s map to these params.
    fn predeclare_imported_constructor(
        &mut self,
        source: &HirImportSource,
        field_name: &str,
        params: &[u32],
    ) -> FxHashMap<u32, BindingId> {
        let ctor_binding =
            self.alloc_synthetic_binding(field_name.to_string(), BindingKind::TopType);
        let mut param_bindings = Vec::with_capacity(params.len());
        let mut param_map = FxHashMap::default();
        for (i, &export_id) in params.iter().enumerate() {
            let pb =
                self.alloc_synthetic_binding(format!("{field_name}${i}"), BindingKind::TypeParam);
            // Ground kind: higher-kinded parameters are refused at export, so an
            // imported constructor's parameters are all of kind `Type`.
            self.type_param_kinds.insert(pb, Kind::ground());
            param_bindings.push(pb);
            param_map.insert(export_id, pb);
        }
        self.alias_params.insert(ctor_binding, param_bindings);
        self.import_type_constructors
            .insert((source.clone(), field_name.to_string()), ctor_binding);
        param_map
    }

    /// Pass B of importing a parametric type constructor: intern the body with
    /// the parameter map active (so `TyVar`s become rigid `TypeVar`s and a
    /// recursive `ConApply` resolves to this constructor), register the alias
    /// body, and materialize a `TypeAlias` decl so TLC and the evaluators treat
    /// it as an ordinary local parametric alias.
    fn finalize_imported_constructor(
        &mut self,
        field_name: &str,
        param_map: FxHashMap<u32, BindingId>,
        body: &ImportedType,
        source: &HirImportSource,
        span: Span,
    ) {
        let ctor_binding = self.import_type_constructors[&(source.clone(), field_name.to_string())];
        let params = self
            .alias_params
            .get(&ctor_binding)
            .cloned()
            .unwrap_or_default();
        let saved = std::mem::replace(&mut self.ctor_param_map, param_map);
        let body_ty = self.intern_imported_type_with_source(body, Some(source), span, None);
        self.ctor_param_map = saved;
        self.aliases.insert(ctor_binding, body_ty);
        let source_decl = self
            .current_import_decl
            .expect("imported constructor is finalized only during import predeclare");
        let decl_id = self.decl_arena.alloc(ThirDecl {
            source: source_decl,
            binding: ctor_binding,
            kind: ThirDeclKind::TypeAlias {
                params,
                ty: body_ty,
            },
            span,
        });
        self.synthetic_decls.push(decl_id);
    }
}
