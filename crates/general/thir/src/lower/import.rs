use rustc_hash::FxHashMap;
use zutai_hir::{BindingId, BindingKind, HirExprId, HirImportSource};
use zutai_syntax::Span;

use crate::import::{ImportedTupleItem, ImportedType};
use crate::ir::{
    Kind, RowTail, ThirDecl, ThirDeclKind, ThirExpr, ThirExprId, ThirExprKind, Type, TypeId,
    TypeKind, TypeRecordField, TypeTupleItem,
};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
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
                self.import_tyvar_cache.clear();
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
            ImportedType::Posit(spec) => self.posit_type(*spec, span),
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
                                let denotation =
                                    self.intern_imported_type_with_source(inner, source, span);
                                if let Some(src) = source {
                                    self.import_type_denotations
                                        .insert((src.clone(), field.name.clone()), denotation);
                                }
                                self.type_type
                            }
                        }
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
                    .map(|a| self.intern_imported_type_with_source(a, source, span))
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
        let body_ty = self.intern_imported_type_with_source(body, Some(source), span);
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
