use zutai_hir::BindingId;
use zutai_thir::{RowTail, ThirConstraintMethod, ThirDeclKind, TypeId, TypeKind, TypeTupleItem};

use crate::ir::{BuiltinOp, Literal, PrimTy, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcType};

use crate::lower::Lowerer;

#[derive(Clone)]
enum DeriveShape {
    Leaf,
    Record(Vec<(String, TypeId)>),
    Tuple(Vec<(Option<String>, TypeId)>),
    Union(Vec<DeriveVariant>),
}

#[derive(Clone)]
struct DeriveVariant {
    name: String,
    payload_fields: Vec<(String, TypeId)>,
}

#[derive(Clone)]
struct DecodedRecordField {
    name: String,
    optional: bool,
    field_ty: crate::ir::TlcTypeId,
    value_ty: crate::ir::TlcTypeId,
    result_ty: crate::ir::TlcTypeId,
    result: TlcExprId,
}

/// Which generic derive builder a recipe body names. A constraint whose
/// `derive =` recipe reduces to a bare reference to one of the ambient builder
/// builtins routes witness synthesis by builder identity rather than by the
/// legacy method-name coincidence — the "generic recipe API".
#[derive(Clone, Copy)]
enum DeriveBuilder {
    Show,
    OrdLex,
    FromData,
    ToData,
}

mod from_data;
mod ord;
mod show;
mod to_data;
mod validation;

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_derive_fields(
        &mut self,
        constraint: BindingId,
        target: TypeId,
        span: zutai_syntax::Span,
    ) -> Vec<(String, TlcExprId)> {
        let Some((constraint_param, methods)) = self.constraint_info(constraint) else {
            return Vec::new();
        };

        // Generic recipe API: a recipe body naming a builder builtin
        // (`<T> => deriveShow`) routes witness synthesis by builder identity,
        // superseding the FromData name-hack and method-name paths below.
        if let Some(marker) = self.derive_builder_marker(constraint) {
            return self.synthesize_builder_fields(
                marker,
                &methods,
                constraint,
                constraint_param,
                target,
            );
        }

        if self.constraint_has_recipe(constraint) {
            if let Some(fields) = self.lower_quoted_recipe_record(constraint, span) {
                return fields;
            }
            return methods
                .iter()
                .filter_map(|method| match derive_recipe_kind(&method.name) {
                    Some(DeriveRecipeKind::Show) => {
                        let value = self.synthesize_show_method(
                            constraint,
                            &method.name,
                            method.sig,
                            constraint_param,
                            target,
                        )?;
                        Some((method.name.clone(), value))
                    }
                    Some(DeriveRecipeKind::Ord) => {
                        let value = self.synthesize_ord_method(
                            constraint,
                            &method.name,
                            method.sig,
                            constraint_param,
                            target,
                        )?;
                        Some((method.name.clone(), value))
                    }
                    None => None,
                })
                .collect();
        }

        let Some(eq_method_name) = methods
            .iter()
            .find(|method| matches!(derive_equality_kind(&method.name), Some(EqualityKind::Eq)))
            .map(|method| method.name.clone())
        else {
            // No positive `eq`/`==` recipe to build on; THIR has already rejected
            // this derive, so emit no fields rather than risk a wrong recipe.
            return Vec::new();
        };

        methods
            .iter()
            .filter_map(|method| {
                let kind = derive_equality_kind(&method.name)?;
                let (arg_ty, result_ty) =
                    self.binary_bool_method_parts(method.sig, constraint_param, target)?;
                let value = self.synthesize_equality_method(
                    constraint,
                    kind,
                    &eq_method_name,
                    target,
                    arg_ty,
                    result_ty,
                );
                Some((method.name.clone(), value))
            })
            .collect()
    }

    /// Detects a recipe body of the form `<T> => deriveShow` — the type-lambda
    /// params live in `recipe.params`, so the body is a bare `BindingRef` to a
    /// builder builtin. Returns which builder, or `None` for any other recipe
    /// shape (quoted `Code`, method-name lambda, or a user binding).
    fn derive_builder_marker(&self, constraint: BindingId) -> Option<DeriveBuilder> {
        let body = self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint {
                    recipe: Some(recipe),
                    ..
                } = &decl.kind
            {
                Some(recipe.body)
            } else {
                None
            }
        })?;
        let zutai_thir::ThirExprKind::BindingRef { binding, .. } = &self.thir.expr_arena[body].kind
        else {
            return None;
        };
        self.builder_marker_name(*binding)
    }

    /// Resolves a `BindingId` to its builder marker, but only when it is the
    /// seeded builtin (first occurrence of the name, `BuiltinValue` kind) — the
    /// same poison-free guard `builtin_overlay_name` uses, so a user binding
    /// that shadows the name never drives synthesis.
    fn builder_marker_name(&self, binding: BindingId) -> Option<DeriveBuilder> {
        if !self
            .thir
            .binding_kinds
            .get(binding.0 as usize)
            .is_some_and(|kind| *kind == zutai_hir::BindingKind::BuiltinValue)
        {
            return None;
        }
        let name = self.thir.binding_names.get(binding.0 as usize)?.as_str();
        let marker = match name {
            "deriveShow" => DeriveBuilder::Show,
            "deriveOrdLex" => DeriveBuilder::OrdLex,
            "deriveFromData" => DeriveBuilder::FromData,
            "deriveToData" => DeriveBuilder::ToData,
            _ => return None,
        };
        let first = self.thir.binding_names.iter().position(|c| c == name)?;
        (first == binding.0 as usize).then_some(marker)
    }

    /// Dispatches every constraint method through the named builder's existing
    /// structural synthesizer at `target`. A method whose signature does not fit
    /// the builder (e.g. a non-`A -> Text` method under `deriveShow`) yields no
    /// field; the synthesizer's own shape validation is the safety net.
    fn synthesize_builder_fields(
        &mut self,
        marker: DeriveBuilder,
        methods: &[ThirConstraintMethod],
        constraint: BindingId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Vec<(String, TlcExprId)> {
        methods
            .iter()
            .filter_map(|method| {
                let value = match marker {
                    DeriveBuilder::Show => self.synthesize_show_method(
                        constraint,
                        &method.name,
                        method.sig,
                        constraint_param,
                        target,
                    )?,
                    DeriveBuilder::OrdLex => self.synthesize_ord_method(
                        constraint,
                        &method.name,
                        method.sig,
                        constraint_param,
                        target,
                    )?,
                    DeriveBuilder::FromData => {
                        self.synthesize_from_data_method(method.sig, constraint_param, target)?
                    }
                    DeriveBuilder::ToData => self.synthesize_to_data_method(
                        constraint,
                        &method.name,
                        method.sig,
                        constraint_param,
                        target,
                    )?,
                };
                Some((method.name.clone(), value))
            })
            .collect()
    }

    fn tlc_row_field_type(
        &self,
        ty: crate::ir::TlcTypeId,
        label: &str,
    ) -> Option<crate::ir::TlcTypeId> {
        let row = match &self.type_arena[ty] {
            TlcType::Record(row) | TlcType::VariantT(row) => row,
            _ => return None,
        };
        row.fields()
            .find_map(|(name, field_ty)| (name == label).then_some(field_ty))
    }

    fn constraint_has_recipe(&self, constraint: BindingId) -> bool {
        self.thir.decls.iter().any(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            decl.binding == constraint
                && matches!(
                    &decl.kind,
                    ThirDeclKind::Constraint {
                        recipe: Some(_),
                        ..
                    }
                )
        })
    }
    fn constraint_info(
        &self,
        constraint: BindingId,
    ) -> Option<(BindingId, Vec<ThirConstraintMethod>)> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint {
                    params, methods, ..
                } = &decl.kind
            {
                return params
                    .first()
                    .copied()
                    .map(|param| (param, methods.clone()));
            }
            None
        })
    }

    fn binary_bool_method_parts(
        &self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<(TypeId, TypeId)> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        let TypeKind::Function {
            from: second,
            to: result,
        } = self.thir.type_arena[to.0 as usize].kind
        else {
            return None;
        };
        if !matches!(self.thir.type_arena[result.0 as usize].kind, TypeKind::Bool) {
            return None;
        }

        let first = self.substitute_constraint_arg(from, constraint_param, target);
        let second = self.substitute_constraint_arg(second, constraint_param, target);
        (first == target && second == target).then_some((target, result))
    }

    fn substitute_constraint_arg(
        &self,
        ty: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> TypeId {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(binding) if binding == constraint_param => target,
            _ => ty,
        }
    }

    fn unary_text_method_parts(
        &self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<(TypeId, TypeId)> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        if !matches!(self.thir.type_arena[to.0 as usize].kind, TypeKind::Text) {
            return None;
        }
        let arg = self.substitute_constraint_arg(from, constraint_param, target);
        (arg == target).then_some((target, to))
    }

    fn binary_ord_method_parts(
        &self,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<(TypeId, TypeId)> {
        let TypeKind::Function { from, to } = self.thir.type_arena[sig.0 as usize].kind else {
            return None;
        };
        let TypeKind::Function {
            from: second,
            to: result,
        } = self.thir.type_arena[to.0 as usize].kind
        else {
            return None;
        };
        let first = self.substitute_constraint_arg(from, constraint_param, target);
        let second = self.substitute_constraint_arg(second, constraint_param, target);
        (first == target && second == target).then_some((target, result))
    }

    fn str_lit(&mut self, text: &str) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let text_ty = self.alloc_type(TlcType::Prim(PrimTy::Str));
        self.alloc_expr(TlcExpr::Lit(Literal::Str(text.to_string())), text_ty, span)
    }

    /// Concatenate rendered pieces with the always-seeded `__textJoin` builtin
    /// (`Text -> List Text -> Text`), joined by the empty separator. There is no
    /// string-concat `BuiltinOp`, so the derive routes through the builtin by its
    /// seeded `BindingId`, exactly as the interpreter and dataflow lowering
    /// resolve it.
    fn text_join(&mut self, pieces: Vec<TlcExprId>) -> TlcExprId {
        if pieces.len() == 1 {
            return pieces[0];
        }
        let span = zutai_syntax::Span::default();
        let text_ty = self.alloc_type(TlcType::Prim(PrimTy::Str));
        let list_ty = self.alloc_type(TlcType::List(text_ty));
        let list = self.alloc_expr(TlcExpr::List(pieces), list_ty, span);
        let sep = self.alloc_expr(TlcExpr::Lit(Literal::Str(String::new())), text_ty, span);
        let join_binding = self.text_join_binding();
        let after_sep_ty = self.alloc_type(TlcType::Fun(list_ty, text_ty, Row::REmpty));
        let join_ty = self.alloc_type(TlcType::Fun(text_ty, after_sep_ty, Row::REmpty));
        let join_fn = self.alloc_expr(TlcExpr::Var(join_binding), join_ty, span);
        let partial = self.alloc_expr(TlcExpr::App(join_fn, sep), after_sep_ty, span);
        self.alloc_expr(TlcExpr::App(partial, list), text_ty, span)
    }

    fn text_join_binding(&self) -> BindingId {
        let index = self
            .thir
            .binding_names
            .iter()
            .position(|name| name == "__textJoin")
            .expect("__textJoin builtin binding is always seeded");
        BindingId(index as u32)
    }

    fn variant_wildcard_pat(&self, variant: &DeriveVariant) -> TlcPat {
        if variant.payload_fields.is_empty() {
            TlcPat::Atom(variant.name.clone())
        } else {
            TlcPat::Variant(variant.name.clone(), Box::new(TlcPat::Wildcard))
        }
    }

    fn if_ordering_eq(
        &mut self,
        cmp: TlcExprId,
        then_expr: TlcExprId,
        else_expr: TlcExprId,
        result_ty: TypeId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
        let result_tlc_ty = self.lower_type(result_ty);
        let eq_atom = self.ordering_atom("eq", result_ty);
        let is_eq = self.alloc_expr(TlcExpr::Builtin(BuiltinOp::Eq, cmp, eq_atom), bool_ty, span);
        self.alloc_expr(
            TlcExpr::Case(
                is_eq,
                vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: then_expr,
                    },
                    TlcAlt {
                        pat: TlcPat::Wildcard,
                        guard: None,
                        body: else_expr,
                    },
                ],
            ),
            result_tlc_ty,
            span,
        )
    }

    fn ordering_atom(&mut self, name: &str, result_ty: TypeId) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let ty = self.lower_type(result_ty);
        self.alloc_expr(TlcExpr::Lit(Literal::Atom(name.to_string())), ty, span)
    }

    fn type_label_for_derive(&self, ty: TypeId) -> String {
        match self.resolve_alias_shape(ty) {
            TypeKind::Bool | TypeKind::True | TypeKind::False => "Bool".to_string(),
            TypeKind::Text => "Text".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Posit(spec) => spec.type_name(),
            TypeKind::Atom(name) => format!("#{name}"),
            TypeKind::Opaque(name) => name,
            _ => "value".to_string(),
        }
    }

    fn derive_witness_call(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        ty: TypeId,
        lhs: TlcExprId,
        rhs: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let component_ty = self.lower_type(ty);
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        let after_first_ty = self.alloc_type(TlcType::Fun(component_ty, bool_ty, Row::REmpty));
        let method_ty = self.alloc_type(TlcType::Fun(component_ty, after_first_ty, Row::REmpty));
        let dict = self.get_dict_expr(constraint, ty, span);
        let method = self.alloc_expr(
            TlcExpr::GetField(dict, method_name.to_string()),
            method_ty,
            span,
        );
        self.register_dict_field_slot(method, constraint, method_name);
        let first = self.alloc_expr(TlcExpr::App(method, lhs), after_first_ty, span);
        self.alloc_expr(TlcExpr::App(first, rhs), bool_ty, span)
    }

    fn derive_get_field(&mut self, base: TlcExprId, field: &str, field_ty: TypeId) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let ty = self.lower_type(field_ty);
        self.alloc_expr(TlcExpr::GetField(base, field.to_string()), ty, span)
    }

    fn fold_bool_parts(&mut self, parts: impl IntoIterator<Item = TlcExprId>) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let bool_ty = self.alloc_type(TlcType::Prim(crate::ir::PrimTy::Bool));
        let mut iter = parts.into_iter();
        let Some(first) = iter.next() else {
            return self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), bool_ty, span);
        };
        iter.fold(first, |lhs, rhs| {
            self.alloc_expr(TlcExpr::Builtin(BuiltinOp::And, lhs, rhs), bool_ty, span)
        })
    }

    fn derive_shape(&self, ty: TypeId) -> DeriveShape {
        match self.resolve_alias_shape(ty) {
            TypeKind::Record(fields, RowTail::Closed) => DeriveShape::Record(
                fields
                    .into_iter()
                    .map(|field| (field.name, field.ty))
                    .collect(),
            ),
            TypeKind::Tuple(items) => DeriveShape::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => (Some(name), ty),
                        TypeTupleItem::Positional(ty) => (None, ty),
                    })
                    .collect(),
            ),
            TypeKind::Union(variants, RowTail::Closed) => DeriveShape::Union(
                variants
                    .into_iter()
                    .map(|variant| {
                        let payload_fields = variant
                            .payload
                            .and_then(|payload| match self.resolve_alias_shape(payload) {
                                TypeKind::Record(fields, RowTail::Closed) => Some(
                                    fields
                                        .into_iter()
                                        .map(|field| (field.name, field.ty))
                                        .collect(),
                                ),
                                _ => None,
                            })
                            .unwrap_or_default();
                        DeriveVariant {
                            name: variant.name,
                            payload_fields,
                        }
                    })
                    .collect(),
            ),
            _ => DeriveShape::Leaf,
        }
    }

    fn resolve_alias_shape(&self, ty: TypeId) -> TypeKind {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Alias(binding) => self
                .type_alias_body(binding)
                .map(|body| self.resolve_alias_shape(body))
                .unwrap_or(TypeKind::Alias(binding)),
            TypeKind::AliasApply { binding, args } => {
                let Some((params, body)) = self.type_alias_params_body(binding) else {
                    return TypeKind::AliasApply { binding, args };
                };
                let subst: rustc_hash::FxHashMap<BindingId, TypeId> =
                    params.into_iter().zip(args).collect();
                self.substitute_alias_shape(body, &subst)
            }
            kind => kind,
        }
    }

    fn substitute_alias_shape(
        &self,
        ty: TypeId,
        subst: &rustc_hash::FxHashMap<BindingId, TypeId>,
    ) -> TypeKind {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(binding) => subst
                .get(&binding)
                .map(|&replacement| self.resolve_alias_shape(replacement))
                .unwrap_or(TypeKind::TypeVar(binding)),
            TypeKind::Record(fields, tail) => TypeKind::Record(
                fields
                    .into_iter()
                    .map(|mut field| {
                        field.ty = self.substitute_component_type(field.ty, subst);
                        field
                    })
                    .collect(),
                tail,
            ),
            TypeKind::Tuple(items) => TypeKind::Tuple(
                items
                    .into_iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, span } => TypeTupleItem::Named {
                            name,
                            ty: self.substitute_component_type(ty, subst),
                            span,
                        },
                        TypeTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.substitute_component_type(ty, subst))
                        }
                    })
                    .collect(),
            ),
            TypeKind::Union(variants, tail) => TypeKind::Union(
                variants
                    .into_iter()
                    .map(|mut variant| {
                        variant.payload = variant
                            .payload
                            .map(|payload| self.substitute_component_type(payload, subst));
                        variant
                    })
                    .collect(),
                tail,
            ),
            kind => kind,
        }
    }

    fn substitute_component_type(
        &self,
        ty: TypeId,
        subst: &rustc_hash::FxHashMap<BindingId, TypeId>,
    ) -> TypeId {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(binding) => subst.get(&binding).copied().unwrap_or(ty),
            _ => ty,
        }
    }

    fn has_witness_binding(&self, constraint: BindingId, ty: TypeId) -> bool {
        self.thir_type_to_witness_key(ty)
            .is_some_and(|key| self.witness_bindings.contains_key(&(constraint.0, key)))
    }

    fn derive_builtin_leaf(&self, ty: TypeId) -> bool {
        matches!(
            self.resolve_alias_shape(ty),
            TypeKind::Bool
                | TypeKind::True
                | TypeKind::False
                | TypeKind::Text
                | TypeKind::Int
                | TypeKind::Float
                | TypeKind::Posit(_)
                | TypeKind::Atom(_)
        )
    }
}

#[derive(Clone, Copy)]
enum EqualityKind {
    Eq,
    Ne,
}

fn derive_equality_kind(method_name: &str) -> Option<EqualityKind> {
    match method_name {
        "eq" | "==" => Some(EqualityKind::Eq),
        "neq" | "!=" => Some(EqualityKind::Ne),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeriveRecipeKind {
    Show,
    Ord,
}

fn derive_recipe_kind(method_name: &str) -> Option<DeriveRecipeKind> {
    match method_name {
        "show" => Some(DeriveRecipeKind::Show),
        "compare" => Some(DeriveRecipeKind::Ord),
        _ => None,
    }
}
