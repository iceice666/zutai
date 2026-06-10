use std::collections::{HashMap, HashSet};

use zutai_hir::{
    BindingId, BindingKind, HirTypeId, HirTypeKind, HirTypeRecordField, HirTypeTupleItem,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn lower_type(&mut self, id: HirTypeId) -> TypeId {
        let ty = self.hir_type(id);
        match &ty.kind {
            HirTypeKind::BindingRef(binding) => self.alias_or_builtin_type(*binding, ty.span),
            HirTypeKind::Record(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| self.lower_type_record_field(field))
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Record(fields),
                    span: ty.span,
                })
            }
            HirTypeKind::Union(items) => {
                let items = items.iter().map(|item| self.lower_type(*item)).collect();
                self.alloc_type(Type {
                    kind: TypeKind::Union(items),
                    span: ty.span,
                })
            }
            HirTypeKind::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        HirTypeTupleItem::Named { name, ty, span } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.lower_type(*ty),
                            span: *span,
                        },
                        HirTypeTupleItem::Positional(ty) => {
                            TypeTupleItem::Positional(self.lower_type(*ty))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(items),
                    span: ty.span,
                })
            }
            HirTypeKind::Optional(inner) => {
                let inner = self.lower_type(*inner);
                self.optional_type(inner, ty.span)
            }
            HirTypeKind::Arrow { from, to } => {
                let from = self.lower_type(*from);
                let to = self.lower_type(*to);
                self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span: ty.span,
                })
            }
            HirTypeKind::Apply { func, arg } => self.lower_type_apply(*func, *arg, ty.span),
            HirTypeKind::Atom(name) => self.alloc_type(Type {
                kind: TypeKind::Atom(name.clone()),
                span: ty.span,
            }),
            HirTypeKind::True => self.alloc_type(Type {
                kind: TypeKind::True,
                span: ty.span,
            }),
            HirTypeKind::False => self.alloc_type(Type {
                kind: TypeKind::False,
                span: ty.span,
            }),
            HirTypeKind::UnresolvedIdent(_) => {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::InvalidTypeExpression {
                        reason: "unresolved type identifier",
                    },
                    span: ty.span,
                });
                self.error_type
            }
            HirTypeKind::Access { .. } => {
                self.invalid_type("type field access is not supported yet", ty.span)
            }
            HirTypeKind::ExprEscape(_) => {
                self.invalid_type("type expression escapes are not supported yet", ty.span)
            }
        }
    }

    fn lower_type_record_field(&mut self, field: &HirTypeRecordField) -> TypeRecordField {
        TypeRecordField {
            name: field.name.clone(),
            optional: field.optional,
            ty: self.lower_type(field.ty),
            span: field.span,
        }
    }

    fn lower_type_apply(&mut self, func: HirTypeId, arg: HirTypeId, span: Span) -> TypeId {
        let func_ty = self.hir_type(func);
        let arg = self.lower_type(arg);
        let HirTypeKind::BindingRef(binding) = func_ty.kind else {
            return self.invalid_type("only built-in type constructors are supported yet", span);
        };
        let name = &self.hir.bindings[binding.0 as usize].name;
        match name.as_str() {
            "List" => self.alloc_type(Type {
                kind: TypeKind::List(arg),
                span,
            }),
            "Optional" => self.optional_type(arg, span),
            _ => self.invalid_type("generic type application is not supported yet", span),
        }
    }

    fn alias_or_builtin_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        let binding_info = &self.hir.bindings[binding.0 as usize];
        match binding_info.kind {
            BindingKind::BuiltinType => self
                .builtin_type_by_name(&binding_info.name, span)
                .unwrap_or_else(|| self.invalid_type("unknown built-in type", span)),
            BindingKind::TopType => self.alias_type(binding, span),
            BindingKind::TypeParam => self.alloc_type(Type {
                kind: TypeKind::TypeVar(binding),
                span,
            }),
            _ => self.invalid_type("value binding used as a type", span),
        }
    }

    pub(super) fn builtin_type_by_name(&mut self, name: &str, span: Span) -> Option<TypeId> {
        let kind = match name {
            "Type" => TypeKind::Type,
            "Text" => TypeKind::Text,
            "Bool" => TypeKind::Bool,
            "Int" => TypeKind::Int,
            "Float" => TypeKind::Float,
            _ => return None,
        };
        Some(self.alloc_type(Type { kind, span }))
    }

    pub(super) fn alias_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Alias(binding),
            span,
        })
    }

    pub(super) fn bool_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Bool,
            span,
        })
    }

    pub(super) fn int_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Int,
            span,
        })
    }

    pub(super) fn float_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Float,
            span,
        })
    }

    pub(super) fn text_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Text,
            span,
        })
    }

    pub(super) fn optional_type(&mut self, inner: TypeId, span: Span) -> TypeId {
        let normalized = self.resolve_alias(inner, &mut HashSet::new(), span);
        if matches!(self.ty(normalized).kind, TypeKind::Optional(_)) {
            return normalized;
        }
        self.alloc_type(Type {
            kind: TypeKind::Optional(inner),
            span,
        })
    }

    pub(super) fn record_fields(&mut self, ty: TypeId, span: Span) -> Option<Vec<TypeRecordField>> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match &self.ty(resolved).kind {
            TypeKind::Record(fields) => Some(fields.clone()),
            _ => None,
        }
    }

    pub(super) fn list_item_type(&mut self, ty: TypeId, span: Span) -> Option<TypeId> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind {
            TypeKind::List(item) => Some(item),
            _ => None,
        }
    }

    pub(super) fn optional_inner_type(&mut self, ty: TypeId, span: Span) -> Option<TypeId> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind {
            TypeKind::Optional(inner) => Some(inner),
            _ => None,
        }
    }

    pub(super) fn function_input_output(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<(TypeId, TypeId)> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind {
            TypeKind::Function { from, to } => Some((from, to)),
            _ => None,
        }
    }

    pub(super) fn function_parts(&mut self, ty: TypeId, span: Span) -> (Vec<TypeId>, TypeId) {
        let mut params = Vec::new();
        let mut current = ty;
        loop {
            let resolved = self.resolve_alias(current, &mut HashSet::new(), span);
            match self.ty(resolved).kind {
                TypeKind::Function { from, to } => {
                    params.push(from);
                    current = to;
                }
                _ => return (params, resolved),
            }
        }
    }

    pub(super) fn type_matches(&mut self, expected: TypeId, found: TypeId) -> bool {
        let expected = self.resolve_alias(expected, &mut HashSet::new(), self.ty(expected).span);
        let found = self.resolve_alias(found, &mut HashSet::new(), self.ty(found).span);
        if expected == found {
            return true;
        }

        match (self.ty(expected).kind.clone(), self.ty(found).kind.clone()) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => true,
            (TypeKind::Bool, TypeKind::True | TypeKind::False) => true,
            (TypeKind::Union(items), _) => items
                .iter()
                .copied()
                .any(|item| self.type_matches(item, found)),
            // #none is always a valid value of Optional(T)
            (TypeKind::Optional(_), TypeKind::Atom(ref name)) if name == "none" => true,
            (TypeKind::List(expected), TypeKind::List(found))
            | (TypeKind::Optional(expected), TypeKind::Optional(found)) => {
                self.type_matches(expected, found)
            }
            (TypeKind::Record(expected_fields), TypeKind::Record(found_fields)) => {
                self.record_types_match(&expected_fields, &found_fields)
            }
            (TypeKind::Tuple(expected_items), TypeKind::Tuple(found_items)) => {
                self.tuple_types_match(&expected_items, &found_items)
            }
            (
                TypeKind::Function {
                    from: expected_from,
                    to: expected_to,
                },
                TypeKind::Function {
                    from: found_from,
                    to: found_to,
                },
            ) => {
                self.type_matches(expected_from, found_from)
                    && self.type_matches(expected_to, found_to)
            }
            (left, right) => left == right,
        }
    }

    fn record_types_match(
        &mut self,
        expected_fields: &[TypeRecordField],
        found_fields: &[TypeRecordField],
    ) -> bool {
        let found_by_name: HashMap<_, _> = found_fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        for expected in expected_fields {
            let Some(found) = found_by_name.get(expected.name.as_str()) else {
                if expected.optional {
                    continue;
                }
                return false;
            };
            if !self.type_matches(expected.ty, found.ty) {
                return false;
            }
        }
        found_fields
            .iter()
            .all(|found| expected_fields.iter().any(|field| field.name == found.name))
    }

    fn tuple_types_match(
        &mut self,
        expected_items: &[TypeTupleItem],
        found_items: &[TypeTupleItem],
    ) -> bool {
        if expected_items.len() != found_items.len() {
            return false;
        }
        expected_items
            .iter()
            .zip(found_items)
            .all(|(expected, found)| match (expected, found) {
                (TypeTupleItem::Positional(expected), TypeTupleItem::Positional(found)) => {
                    self.type_matches(*expected, *found)
                }
                (
                    TypeTupleItem::Named {
                        name: expected_name,
                        ty: expected,
                        ..
                    },
                    TypeTupleItem::Named {
                        name: found_name,
                        ty: found,
                        ..
                    },
                ) if expected_name == found_name => self.type_matches(*expected, *found),
                _ => false,
            })
    }

    pub(super) fn resolve_alias(
        &mut self,
        ty: TypeId,
        seen: &mut HashSet<BindingId>,
        span: Span,
    ) -> TypeId {
        let TypeKind::Alias(binding) = self.ty(ty).kind else {
            return ty;
        };
        if !seen.insert(binding) {
            let name = self.hir.bindings[binding.0 as usize].name.clone();
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::AliasCycle { name },
                span,
            });
            return self.error_type;
        }
        match self.aliases.get(&binding).copied() {
            Some(alias) => self.resolve_alias(alias, seen, span),
            None => ty,
        }
    }

    pub(super) fn type_name(&mut self, ty: TypeId) -> String {
        let ty = self.resolve_alias(ty, &mut HashSet::new(), self.ty(ty).span);
        match self.ty(ty).kind.clone() {
            TypeKind::Type => "Type".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Text => "Text".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Atom(name) => format!("#{name}"),
            TypeKind::True => "true".to_string(),
            TypeKind::False => "false".to_string(),
            TypeKind::List(inner) => format!("List {}", self.type_name(inner)),
            TypeKind::Optional(inner) => format!("{}?", self.type_name(inner)),
            TypeKind::Record(_) => "record".to_string(),
            TypeKind::Union(_) => "union".to_string(),
            TypeKind::Tuple(_) => "tuple".to_string(),
            TypeKind::Function { .. } => "function".to_string(),
            TypeKind::TypeVar(binding) | TypeKind::Alias(binding) => {
                self.hir.bindings[binding.0 as usize].name.clone()
            }
            TypeKind::Error => "<error>".to_string(),
        }
    }

    pub(super) fn type_mismatch(&mut self, expected: TypeId, found: TypeId, span: Span) {
        let expected = self.type_name(expected);
        let found = self.type_name(found);
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::TypeMismatch { expected, found },
            span,
        });
    }
}
