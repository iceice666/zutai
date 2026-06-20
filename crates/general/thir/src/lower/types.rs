use std::collections::{HashMap, HashSet};

use zutai_hir::{
    BindingId, BindingKind, HirRowTail, HirRowTailKind, HirTypeId, HirTypeKind, HirTypeRecordField,
    HirTypeTupleItem, HirUnionVariant,
};
use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{RowTail, Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem, UnionVariant};

use super::{Lowerer, RowSolution};

impl<'hir> Lowerer<'hir> {
    pub(super) fn lower_type(&mut self, id: HirTypeId) -> TypeId {
        let ty = self.hir_type(id);
        match &ty.kind {
            HirTypeKind::BindingRef(binding) => self.alias_or_builtin_type(*binding, ty.span),
            HirTypeKind::Record { fields, tail } => {
                let mut thir_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|field| self.lower_type_record_field(field))
                    .collect();
                let row_tail = self.lower_record_tail(tail.as_ref(), &mut thir_fields);
                self.alloc_type(Type {
                    kind: TypeKind::Record(thir_fields, row_tail),
                    span: ty.span,
                })
            }
            HirTypeKind::Union { variants, tail } => {
                let mut thir_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v: &HirUnionVariant| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.as_ref().map(|fields| {
                            let field_types: Vec<TypeRecordField> = fields
                                .iter()
                                .map(|f| self.lower_type_record_field(f))
                                .collect();
                            self.alloc_type(Type {
                                kind: TypeKind::Record(field_types, RowTail::Closed),
                                span: v.span,
                            })
                        }),
                        span: v.span,
                    })
                    .collect();
                let row_tail = self.lower_union_tail(tail.as_ref(), &mut thir_variants);
                self.alloc_type(Type {
                    kind: TypeKind::Union(thir_variants, row_tail),
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
            HirTypeKind::Effect { .. } => {
                self.unsupported_type("effect rows in function types", ty.span)
            }
            HirTypeKind::Select { receiver, fields } => {
                let receiver_ty = self.lower_type(*receiver);
                let resolved = self.resolve_alias(receiver_ty, &mut HashSet::new(), ty.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Record(rec_fields, _) => {
                        let mut selected = Vec::with_capacity(fields.len());
                        for sf in fields {
                            match rec_fields.iter().find(|f| f.name == sf.name) {
                                Some(rf) => selected.push(rf.clone()),
                                None => self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::UnknownField {
                                        name: sf.name.clone(),
                                    },
                                    span: sf.span,
                                }),
                            }
                        }
                        self.alloc_type(Type {
                            kind: TypeKind::Record(selected, RowTail::Closed),
                            span: ty.span,
                        })
                    }
                    TypeKind::Error => self.error_type,
                    _ => self.invalid_type("type-level select requires a record type", ty.span),
                }
            }
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
            HirTypeKind::Access { receiver, field } => {
                // Resolve `moduleLib.SomeType` in annotation position.
                // Only simple `BindingRef` receivers are supported (e.g. `serverLib`);
                // chained access (`a.b.C`) is not yet implemented.
                let access_span = ty.span;
                let receiver_hir = self.hir_type(*receiver);
                let binding = match &receiver_hir.kind {
                    HirTypeKind::BindingRef(b) => *b,
                    _ => {
                        return self.invalid_type(
                            "type field access receiver must be a simple name",
                            access_span,
                        );
                    }
                };
                // Look up the record type of the receiver (e.g. the inferred
                // record type of `serverLib := import "server.zt"`).
                let receiver_ty = match self.value_types.get(&binding).copied() {
                    Some(t) => t,
                    None => {
                        return self
                            .invalid_type("type field access on unknown binding", access_span);
                    }
                };
                // Walk to the record fields of that type.
                let fields = match self.record_fields(receiver_ty, access_span) {
                    Some(f) => f,
                    None => {
                        return self
                            .invalid_type("type field access on non-record type", access_span);
                    }
                };
                let Some(record_field) = fields.iter().find(|f| f.name == *field).cloned() else {
                    return self.invalid_type("unknown type field", access_span);
                };
                // If this binding is a known import and the field carries a
                // registered type denotation, return the concrete type so that
                // annotation-position use (`x : serverLib.Server`) type-checks.
                if let Some(import_source) = self.binding_import_key.get(&binding).cloned()
                    && let Some(&denotation) = self
                        .import_type_denotations
                        .get(&(import_source, field.clone()))
                {
                    return denotation;
                }
                record_field.ty
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

    /// Lower a record row tail, expanding `...Shape` spreads into `fields` and
    /// returning the resulting `RowTail`. Anonymous `...` becomes `Open`; a
    /// `<Rest>` row variable becomes a rigid `Param`.
    fn lower_record_tail(
        &mut self,
        tail: Option<&HirRowTail>,
        fields: &mut Vec<TypeRecordField>,
    ) -> RowTail {
        let Some(tail) = tail else {
            return RowTail::Closed;
        };
        match &tail.kind {
            HirRowTailKind::Anonymous | HirRowTailKind::Unresolved(_) => RowTail::Open,
            HirRowTailKind::Var(binding) => RowTail::Param(*binding),
            HirRowTailKind::Spread(binding) => {
                let spread = self.alias_or_builtin_type(*binding, tail.span);
                let resolved = self.resolve_alias(spread, &mut HashSet::new(), tail.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Record(spread_fields, spread_tail) => {
                        for sf in spread_fields {
                            if fields.iter().any(|f| f.name == sf.name) {
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::OverlappingRowField {
                                        name: sf.name.clone(),
                                    },
                                    span: tail.span,
                                });
                            } else {
                                fields.push(sf);
                            }
                        }
                        spread_tail
                    }
                    TypeKind::Error => RowTail::Closed,
                    _ => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::InvalidTypeExpression {
                                reason: "record spread requires a record type",
                            },
                            span: tail.span,
                        });
                        RowTail::Closed
                    }
                }
            }
        }
    }

    /// Lower a union row tail, expanding `...Shape` spreads into `variants`.
    fn lower_union_tail(
        &mut self,
        tail: Option<&HirRowTail>,
        variants: &mut Vec<UnionVariant>,
    ) -> RowTail {
        let Some(tail) = tail else {
            return RowTail::Closed;
        };
        match &tail.kind {
            HirRowTailKind::Anonymous | HirRowTailKind::Unresolved(_) => RowTail::Open,
            HirRowTailKind::Var(binding) => RowTail::Param(*binding),
            HirRowTailKind::Spread(binding) => {
                let spread = self.alias_or_builtin_type(*binding, tail.span);
                let resolved = self.resolve_alias(spread, &mut HashSet::new(), tail.span);
                match self.ty(resolved).kind.clone() {
                    TypeKind::Union(spread_variants, spread_tail) => {
                        for sv in spread_variants {
                            if variants.iter().any(|v| v.name == sv.name) {
                                self.diagnostics.push(ThirDiagnostic {
                                    kind: ThirDiagnosticKind::OverlappingRowField {
                                        name: sv.name.clone(),
                                    },
                                    span: tail.span,
                                });
                            } else {
                                variants.push(sv);
                            }
                        }
                        spread_tail
                    }
                    TypeKind::Error => RowTail::Closed,
                    _ => {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::InvalidTypeExpression {
                                reason: "union spread requires a union type",
                            },
                            span: tail.span,
                        });
                        RowTail::Closed
                    }
                }
            }
        }
    }

    fn lower_type_apply(&mut self, func: HirTypeId, arg: HirTypeId, span: Span) -> TypeId {
        // Walk the left-nested Apply spine to collect head + all args left-to-right.
        let mut args = vec![self.lower_type(arg)];
        let mut head = func;
        loop {
            let head_kind = self.hir_type(head).kind.clone();
            match head_kind {
                HirTypeKind::Apply { func: f, arg: a } => {
                    args.push(self.lower_type(a));
                    head = f;
                }
                _ => break,
            }
        }
        args.reverse();

        let HirTypeKind::BindingRef(binding) = self.hir_type(head).kind else {
            return self.invalid_type("only named type constructors can be applied", span);
        };

        // Built-in single-arg constructors keep existing handling.
        let name = self.hir.bindings[binding.0 as usize].name.clone();
        match name.as_str() {
            "List" if args.len() == 1 => {
                return self.alloc_type(Type {
                    kind: TypeKind::List(args[0]),
                    span,
                });
            }
            "Optional" if args.len() == 1 => return self.optional_type(args[0], span),
            _ => {}
        }

        // Generic alias or type-level function: arity-check and build a lazy
        // AliasApply node. Expansion happens on demand in resolve_alias.
        if let Some(params) = self.alias_params.get(&binding).cloned() {
            if params.len() != args.len() {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: params.len(),
                        found: args.len(),
                    },
                    span,
                });
                return self.error_type;
            }
            return self.alloc_type(Type {
                kind: TypeKind::AliasApply { binding, args },
                span,
            });
        }

        self.invalid_type("type is not a parametric constructor", span)
    }

    fn alias_or_builtin_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        // A bare reference to a parametric constructor (without application) is
        // a zero-argument arity error. Check before the binding-kind match so
        // both TopType and TopFunction aliases can be caught here.
        if let Some(params) = self.alias_params.get(&binding).cloned() {
            let name = self.hir.bindings[binding.0 as usize].name.clone();
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                    name,
                    expected: params.len(),
                    found: 0,
                },
                span,
            });
            return self.error_type;
        }
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
            BindingKind::Param | BindingKind::Local if self.type_param_scope.contains(&binding) => {
                // A `Param` or `Local` binding that was registered in
                // `type_param_scope` during type-level function body lowering
                // acts as a substitutable type variable.
                self.alloc_type(Type {
                    kind: TypeKind::TypeVar(binding),
                    span,
                })
            }
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
        // Flatten any solved flexible tail so fields captured by a named row tail
        // (e.g. the result of a row-polymorphic call) are visible before zonking.
        self.record_row(ty, span).map(|(fields, _)| fields)
    }

    /// Like `record_fields` but also returns the row tail, with any solved
    /// flexible tail flattened in. Used by record checking to honour open rows.
    pub(super) fn record_row(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Option<(Vec<TypeRecordField>, RowTail)> {
        let resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.ty(resolved).kind.clone() {
            TypeKind::Record(fields, tail) => Some(self.flatten_record_row(fields, tail)),
            _ => None,
        }
    }

    pub(super) fn list_item_type(&mut self, ty: TypeId, span: Span) -> Option<TypeId> {
        let alias_resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        let resolved = self.resolve(alias_resolved);
        match self.ty(resolved).kind {
            TypeKind::List(item) => Some(item),
            // For an unsolved InferVar, mint a fresh `List` and unify to bind it,
            // so a list literal checked against an as-yet-unknown type (e.g. a
            // constraint method's instantiated parameter) infers `List <item>`
            // instead of failing with `ExpectedList`.
            TypeKind::InferVar(_) => {
                let item = self.fresh_infer_var(span);
                let list = self.alloc_type(Type {
                    kind: TypeKind::List(item),
                    span,
                });
                self.unify(resolved, list, span);
                Some(item)
            }
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
        // First resolve named aliases, then chase any InferVar substitutions.
        let alias_resolved = self.resolve_alias(ty, &mut HashSet::new(), span);
        let resolved = self.resolve(alias_resolved);
        match self.ty(resolved).kind {
            TypeKind::Function { from, to } => Some((from, to)),
            // For an unsolved InferVar, mint a fresh arrow and unify to bind it.
            TypeKind::InferVar(_) => {
                let from = self.fresh_infer_var(span);
                let to = self.fresh_infer_var(span);
                let arrow = self.alloc_type(Type {
                    kind: TypeKind::Function { from, to },
                    span,
                });
                self.unify(resolved, arrow, span);
                Some((from, to))
            }
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
        let e_span = self.type_arena[expected.0 as usize].span;
        let f_span = self.type_arena[found.0 as usize].span;
        let expected = self.resolve_alias(expected, &mut HashSet::new(), e_span);
        let found = self.resolve_alias(found, &mut HashSet::new(), f_span);
        if expected == found {
            return true;
        }

        let ek = self.type_arena[expected.0 as usize].kind.clone();
        let fk = self.type_arena[found.0 as usize].kind.clone();

        match (ek, fk) {
            (TypeKind::Error, _) | (_, TypeKind::Error) => true,

            // Solve InferVars: if either side is an unsolved InferVar, unify
            // and treat as matching (errors emitted inside unify on conflicts).
            (TypeKind::InferVar(v), _) => {
                if !self.occurs(v, found) {
                    self.infer_subst.insert(v, found);
                }
                true
            }
            (_, TypeKind::InferVar(v)) => {
                if !self.occurs(v, expected) {
                    self.infer_subst.insert(v, expected);
                }
                true
            }

            (TypeKind::Bool, TypeKind::True | TypeKind::False) => true,
            (TypeKind::Union(ev, et), TypeKind::Atom(ref name)) => {
                // Treat the atom as a singleton closed union so the row logic
                // decides membership: an explicit nullary member matches, and an
                // open/flexible tail absorbs (and captures) an extra member.
                let found = [UnionVariant {
                    name: name.clone(),
                    payload: None,
                    span: Span::default(),
                }];
                self.union_rows_match(&ev, et, &found, RowTail::Closed)
            }
            (TypeKind::Union(ev, et), TypeKind::Union(fv, ft)) => {
                if et == RowTail::Closed && ft == RowTail::Closed {
                    // Closed v0 unions match exactly (same members, same order).
                    ev.len() == fv.len()
                        && ev.iter().zip(fv.iter()).all(|(a, b)| {
                            a.name == b.name
                                && match (a.payload, b.payload) {
                                    (Some(pa), Some(pb)) => self.type_matches(pa, pb),
                                    (None, None) => true,
                                    _ => false,
                                }
                        })
                } else {
                    self.union_rows_match(&ev, et, &fv, ft)
                }
            }
            // #none is always a valid value of Optional(T)
            (TypeKind::Optional(_), TypeKind::Atom(ref name)) if name == "none" => true,
            (TypeKind::List(e), TypeKind::List(f))
            | (TypeKind::Optional(e), TypeKind::Optional(f)) => self.type_matches(e, f),
            (TypeKind::Record(ef, et), TypeKind::Record(ff, ft)) => {
                self.record_rows_match(&ef, et, &ff, ft)
            }
            (TypeKind::Tuple(ei), TypeKind::Tuple(fi)) => self.tuple_types_match(&ei, &fi),
            (TypeKind::Function { from: ef, to: et }, TypeKind::Function { from: ff, to: ft }) => {
                // Parameters are contravariant, results covariant. Contravariance
                // is required for soundness now that records have width subtyping:
                // a function accepting an open record may stand in for one that
                // takes a wider closed record, but never the reverse.
                self.type_matches(ff, ef) && self.type_matches(et, ft)
            }
            (left, right) => left == right,
        }
    }

    /// Row-aware record assignability: `found` is assignable to `expected` when
    /// it provides every required field of `expected` (with matching types).
    /// Extra found fields are accepted only if `expected`'s tail is open: an
    /// anonymous tail discards them, a flexible row variable captures them, and a
    /// rigid tail requires the same variable with no extras.
    fn record_rows_match(
        &mut self,
        ef: &[TypeRecordField],
        et: RowTail,
        ff: &[TypeRecordField],
        ft: RowTail,
    ) -> bool {
        let (ef, et) = self.flatten_record_row(ef.to_vec(), et);
        let (ff, ft) = self.flatten_record_row(ff.to_vec(), ft);
        let found_by_name: HashMap<&str, &TypeRecordField> =
            ff.iter().map(|f| (f.name.as_str(), f)).collect();
        for e in &ef {
            match found_by_name.get(e.name.as_str()) {
                Some(f) => {
                    if !self.type_matches(e.ty, f.ty) {
                        return false;
                    }
                }
                None => {
                    if !e.optional {
                        return false;
                    }
                }
            }
        }
        let expected_names: HashSet<&str> = ef.iter().map(|f| f.name.as_str()).collect();
        let extras: Vec<TypeRecordField> = ff
            .iter()
            .filter(|f| !expected_names.contains(f.name.as_str()))
            .cloned()
            .collect();
        match et {
            RowTail::Closed => extras.is_empty() && ft == RowTail::Closed,
            RowTail::Open => true,
            RowTail::Param(p) => extras.is_empty() && ft == RowTail::Param(p),
            RowTail::Infer(r) => {
                if ft == RowTail::Infer(r) {
                    extras.is_empty()
                } else {
                    self.row_subst.insert(
                        r,
                        RowSolution::Record {
                            fields: extras,
                            tail: ft,
                        },
                    );
                    true
                }
            }
        }
    }

    /// Row-aware union assignability — the dual of `record_rows_match`. A value
    /// of union type `found` is assignable to `expected` when every member
    /// `found` may be is accounted for by `expected`: it either matches an
    /// explicit member (with matching payload) or is absorbed by `expected`'s
    /// tail (discarded by an anonymous tail, captured by a flexible row variable,
    /// rejected by a closed or rigid tail). Explicit `expected` members absent
    /// from `found` are fine — a handler may cover cases the value never takes.
    fn union_rows_match(
        &mut self,
        ev: &[UnionVariant],
        et: RowTail,
        fv: &[UnionVariant],
        ft: RowTail,
    ) -> bool {
        let (ev, et) = self.flatten_union_row(ev.to_vec(), et);
        let (fv, ft) = self.flatten_union_row(fv.to_vec(), ft);
        let expected_by_name: HashMap<&str, &UnionVariant> =
            ev.iter().map(|v| (v.name.as_str(), v)).collect();
        let mut extras: Vec<UnionVariant> = Vec::new();
        for f in &fv {
            match expected_by_name.get(f.name.as_str()) {
                Some(e) => match (e.payload, f.payload) {
                    (Some(pe), Some(pf)) => {
                        if !self.type_matches(pe, pf) {
                            return false;
                        }
                    }
                    (None, None) => {}
                    _ => return false,
                },
                None => extras.push(f.clone()),
            }
        }
        match et {
            RowTail::Closed => extras.is_empty() && ft == RowTail::Closed,
            RowTail::Open => true,
            RowTail::Param(p) => extras.is_empty() && ft == RowTail::Param(p),
            RowTail::Infer(r) => {
                if ft == RowTail::Infer(r) {
                    extras.is_empty()
                } else {
                    self.row_subst.insert(
                        r,
                        RowSolution::Union {
                            variants: extras,
                            tail: ft,
                        },
                    );
                    true
                }
            }
        }
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
        // Resolve InferVar chains first so alias resolution sees the concrete type.
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Alias(binding) => {
                if !seen.insert(binding) {
                    self.push_alias_cycle(binding, span);
                    return self.error_type;
                }
                match self.aliases.get(&binding).copied() {
                    Some(alias) => self.resolve_alias(alias, seen, span),
                    None => ty,
                }
            }
            TypeKind::AliasApply { binding, args } => {
                if !seen.insert(binding) {
                    self.push_alias_cycle(binding, span);
                    return self.error_type;
                }
                if self.type_eval_fuel == 0 {
                    self.diagnostics.push(ThirDiagnostic {
                        kind: ThirDiagnosticKind::TypeLevelEvalLimitExceeded,
                        span,
                    });
                    return self.error_type;
                }
                self.type_eval_fuel -= 1;
                let Some(params) = self.alias_params.get(&binding).cloned() else {
                    return ty; // not registered → leave inert (arity already diagnosed)
                };
                let Some(body) = self.aliases.get(&binding).copied() else {
                    return ty;
                };
                let subst: HashMap<BindingId, TypeId> = params.into_iter().zip(args).collect();
                let expanded = self.instantiate_type_vars(body, &subst);
                self.resolve_alias(expanded, seen, span)
            }
            _ => ty,
        }
    }

    fn push_alias_cycle(&mut self, binding: BindingId, span: Span) {
        let name = self.hir.bindings[binding.0 as usize].name.clone();
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::AliasCycle { name },
            span,
        });
    }

    pub(super) fn type_name(&mut self, ty: TypeId) -> String {
        let span = self.type_arena[ty.0 as usize].span;
        let ty = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.type_arena[ty.0 as usize].kind.clone() {
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
            TypeKind::Record(_, _) => "record".to_string(),
            TypeKind::Union(_, _) => "union".to_string(),
            TypeKind::Tuple(_) => "tuple".to_string(),
            TypeKind::Function { .. } => "function".to_string(),
            TypeKind::TypeVar(binding) | TypeKind::Alias(binding) => {
                self.hir.bindings[binding.0 as usize].name.clone()
            }
            TypeKind::AliasApply { binding, args } => {
                let head = self.hir.bindings[binding.0 as usize].name.clone();
                let parts: Vec<String> = args.iter().map(|&a| self.type_name(a)).collect();
                format!("{head} {}", parts.join(" "))
            }
            TypeKind::InferVar(v) => format!("?{v}"),
            TypeKind::Error => "<error>".to_string(),
        }
    }

    /// Structural coherence key for a witness target type.
    ///
    /// Unlike `type_name`, this function recurses into compound types
    /// (`Record`, `Union`, `Tuple`, `Function`) so that distinct types
    /// always produce distinct keys. This is used as the second half of
    /// the coherence-check map key `(constraint BindingId, target key)`.
    pub(super) fn witness_target_key(&mut self, ty: TypeId) -> String {
        self.witness_target_key_with(ty, &HashMap::new())
    }

    /// Like `witness_target_key`, but each binding in `norm` (a witness's own
    /// type params) keys to its positional `#index` instead of `@<binding>`, so
    /// two conditional witnesses that differ only in param identity — e.g. two
    /// `Eq @(List A)` — produce the same key and are flagged as conflicting.
    pub(super) fn witness_target_key_with(
        &mut self,
        ty: TypeId,
        norm: &std::collections::HashMap<BindingId, usize>,
    ) -> String {
        let span = self.type_arena[ty.0 as usize].span;
        let ty = self.resolve_alias(ty, &mut HashSet::new(), span);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Type => "Type".to_string(),
            TypeKind::Bool => "Bool".to_string(),
            TypeKind::Text => "Text".to_string(),
            TypeKind::Int => "Int".to_string(),
            TypeKind::Float => "Float".to_string(),
            TypeKind::Atom(name) => format!("#{name}"),
            TypeKind::True => "true".to_string(),
            TypeKind::False => "false".to_string(),
            TypeKind::List(inner) => format!("[{}]", self.witness_target_key_with(inner, norm)),
            TypeKind::Optional(inner) => {
                format!("{}?", self.witness_target_key_with(inner, norm))
            }
            TypeKind::Record(fields, tail) => {
                // Sort by name — records are order-independent.
                let mut parts: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let k = self.witness_target_key_with(f.ty, norm);
                        if f.optional {
                            format!("{}?:{}", f.name, k)
                        } else {
                            format!("{}:{}", f.name, k)
                        }
                    })
                    .collect();
                parts.sort();
                format!("{{{}{}}}", parts.join(","), row_tail_key(tail))
            }
            TypeKind::Union(variants, tail) => {
                let parts: Vec<String> = variants
                    .iter()
                    .map(|v| match v.payload {
                        Some(p) => format!("{}({})", v.name, self.witness_target_key_with(p, norm)),
                        None => v.name.clone(),
                    })
                    .collect();
                format!("<{}{}>", parts.join("|"), row_tail_key(tail))
            }
            TypeKind::Tuple(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => {
                            format!("{}:{}", name, self.witness_target_key_with(*ty, norm))
                        }
                        TypeTupleItem::Positional(ty) => self.witness_target_key_with(*ty, norm),
                    })
                    .collect();
                format!("({})", parts.join(","))
            }
            TypeKind::Function { from, to } => {
                format!(
                    "({}->{})",
                    self.witness_target_key_with(from, norm),
                    self.witness_target_key_with(to, norm)
                )
            }
            // Witness params normalize to positional holes; other vars/aliases
            // key by binding index (shadow-safe).
            TypeKind::TypeVar(b) => match norm.get(&b) {
                Some(i) => format!("#{i}"),
                None => format!("@{}", b.0),
            },
            TypeKind::Alias(b) => format!("@{}", b.0),
            TypeKind::AliasApply { binding, args } => {
                let parts: Vec<String> = args
                    .iter()
                    .map(|&a| self.witness_target_key_with(a, norm))
                    .collect();
                format!("${}[{}]", binding.0, parts.join(","))
            }
            TypeKind::InferVar(v) => format!("?{v}"),
            TypeKind::Error => "<error>".to_string(),
        }
    }

    /// Collect all `TypeVar` binding IDs that appear free in `ty`, in a
    /// deduped stable order (by binding index).
    pub(super) fn collect_type_vars(&self, ty: TypeId) -> Vec<BindingId> {
        let mut vars: Vec<BindingId> = Vec::new();
        self.collect_type_vars_into(ty, &mut vars);
        vars.sort_by_key(|b| b.0);
        vars.dedup();
        vars
    }

    fn collect_type_vars_into(&self, ty: TypeId, out: &mut Vec<BindingId>) {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) => out.push(b),
            TypeKind::Function { from, to } => {
                self.collect_type_vars_into(from, out);
                self.collect_type_vars_into(to, out);
            }
            TypeKind::List(inner) | TypeKind::Optional(inner) => {
                self.collect_type_vars_into(inner, out);
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(payload) = v.payload {
                        self.collect_type_vars_into(payload, out);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => ty,
                        TypeTupleItem::Positional(ty) => ty,
                    };
                    self.collect_type_vars_into(inner, out);
                }
            }
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.collect_type_vars_into(f.ty, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in args {
                    self.collect_type_vars_into(a, out);
                }
            }
            _ => {}
        }
    }

    /// Collect all rigid row variables (`RowTail::Param`) appearing in `ty`, in a
    /// deduped stable order. These `<Rest>` row parameters are instantiated with
    /// fresh flexible row variables at each call site, like type parameters.
    pub(super) fn collect_row_params(&self, ty: TypeId) -> Vec<BindingId> {
        let mut vars: Vec<BindingId> = Vec::new();
        self.collect_row_params_into(ty, &mut vars);
        vars.sort_by_key(|b| b.0);
        vars.dedup();
        vars
    }

    fn collect_row_params_into(&self, ty: TypeId, out: &mut Vec<BindingId>) {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                self.collect_row_params_into(from, out);
                self.collect_row_params_into(to, out);
            }
            TypeKind::List(inner) | TypeKind::Optional(inner) => {
                self.collect_row_params_into(inner, out);
            }
            TypeKind::Record(fields, tail) => {
                for f in &fields {
                    self.collect_row_params_into(f.ty, out);
                }
                if let RowTail::Param(b) = tail {
                    out.push(b);
                }
            }
            TypeKind::Union(variants, tail) => {
                for v in &variants {
                    if let Some(payload) = v.payload {
                        self.collect_row_params_into(payload, out);
                    }
                }
                if let RowTail::Param(b) = tail {
                    out.push(b);
                }
            }
            TypeKind::Tuple(items) => {
                for item in &items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => *ty,
                        TypeTupleItem::Positional(ty) => *ty,
                    };
                    self.collect_row_params_into(inner, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in &args {
                    self.collect_row_params_into(*a, out);
                }
            }
            _ => {}
        }
    }

    /// Substitute `TypeVar`s appearing in `ty` according to `subst`.
    /// Allocates new `Type` nodes for any structural type that contains
    /// substituted vars; leaf types and unchanged subtrees are reused.
    pub(super) fn instantiate_type_vars(
        &mut self,
        ty: TypeId,
        subst: &HashMap<BindingId, TypeId>,
    ) -> TypeId {
        if subst.is_empty() {
            return ty;
        }
        let span = self.type_arena[ty.0 as usize].span;
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) => subst.get(&b).copied().unwrap_or(ty),
            TypeKind::Function { from, to } => {
                let new_from = self.instantiate_type_vars(from, subst);
                let new_to = self.instantiate_type_vars(to, subst);
                if new_from == from && new_to == to {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Function {
                        from: new_from,
                        to: new_to,
                    },
                    span,
                })
            }
            TypeKind::List(inner) => {
                let new_inner = self.instantiate_type_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::List(new_inner),
                    span,
                })
            }
            TypeKind::Optional(inner) => {
                let new_inner = self.instantiate_type_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Optional(new_inner),
                    span,
                })
            }
            TypeKind::Union(variants, tail) => {
                let new_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|p| self.instantiate_type_vars(p, subst)),
                        span: v.span,
                    })
                    .collect();
                if new_variants
                    .iter()
                    .zip(variants.iter())
                    .all(|(n, o)| n.payload == o.payload)
                {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Union(new_variants, tail),
                    span,
                })
            }
            TypeKind::Tuple(items) => {
                let new_items: Vec<TypeTupleItem> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named {
                            name,
                            ty: inner,
                            span: s,
                        } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.instantiate_type_vars(*inner, subst),
                            span: *s,
                        },
                        TypeTupleItem::Positional(inner) => {
                            TypeTupleItem::Positional(self.instantiate_type_vars(*inner, subst))
                        }
                    })
                    .collect();
                if new_items == items {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(new_items),
                    span,
                })
            }
            TypeKind::Record(fields, tail) => {
                let new_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|f| TypeRecordField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: self.instantiate_type_vars(f.ty, subst),
                        span: f.span,
                    })
                    .collect();
                if new_fields == fields {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Record(new_fields, tail),
                    span,
                })
            }
            TypeKind::AliasApply { binding, args } => {
                let new_args: Vec<TypeId> = args
                    .iter()
                    .map(|&a| self.instantiate_type_vars(a, subst))
                    .collect();
                if new_args == args {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::AliasApply {
                        binding,
                        args: new_args,
                    },
                    span,
                })
            }
            _ => ty,
        }
    }

    /// Replace rigid row variables (`RowTail::Param`) in `ty` with the flexible
    /// row variables given by `subst`, rebuilding only structural nodes.
    pub(super) fn instantiate_row_params(
        &mut self,
        ty: TypeId,
        subst: &HashMap<BindingId, RowTail>,
    ) -> TypeId {
        if subst.is_empty() {
            return ty;
        }
        let span = self.type_arena[ty.0 as usize].span;
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Function { from, to } => {
                let nf = self.instantiate_row_params(from, subst);
                let nt = self.instantiate_row_params(to, subst);
                if nf == from && nt == to {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Function { from: nf, to: nt },
                    span,
                })
            }
            TypeKind::List(inner) => {
                let ni = self.instantiate_row_params(inner, subst);
                if ni == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::List(ni),
                    span,
                })
            }
            TypeKind::Optional(inner) => {
                let ni = self.instantiate_row_params(inner, subst);
                if ni == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Optional(ni),
                    span,
                })
            }
            TypeKind::Record(fields, tail) => {
                let new_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|f| TypeRecordField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: self.instantiate_row_params(f.ty, subst),
                        span: f.span,
                    })
                    .collect();
                let new_tail = match tail {
                    RowTail::Param(b) => subst.get(&b).copied().unwrap_or(tail),
                    _ => tail,
                };
                if new_fields == fields && new_tail == tail {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Record(new_fields, new_tail),
                    span,
                })
            }
            TypeKind::Union(variants, tail) => {
                let new_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|p| self.instantiate_row_params(p, subst)),
                        span: v.span,
                    })
                    .collect();
                let new_tail = match tail {
                    RowTail::Param(b) => subst.get(&b).copied().unwrap_or(tail),
                    _ => tail,
                };
                self.alloc_type(Type {
                    kind: TypeKind::Union(new_variants, new_tail),
                    span,
                })
            }
            TypeKind::Tuple(items) => {
                let new_items: Vec<TypeTupleItem> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named {
                            name,
                            ty: inner,
                            span: s,
                        } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.instantiate_row_params(*inner, subst),
                            span: *s,
                        },
                        TypeTupleItem::Positional(inner) => {
                            TypeTupleItem::Positional(self.instantiate_row_params(*inner, subst))
                        }
                    })
                    .collect();
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(new_items),
                    span,
                })
            }
            _ => ty,
        }
    }

    // ── HM let-generalization ────────────────────────────────────────────────

    /// Collect every unresolved `InferVar` id that appears free in `ty`, deduped
    /// in stable order. Resolves chains at entry so partially-solved variables
    /// (e.g. `?1` pointing at `?0`) are reported by their canonical id.
    pub(super) fn free_infer_vars_in(&self, ty: TypeId) -> Vec<u32> {
        let mut vars: Vec<u32> = Vec::new();
        self.free_infer_vars_into(ty, &mut vars);
        vars.sort_unstable();
        vars.dedup();
        vars
    }

    fn free_infer_vars_into(&self, ty: TypeId, out: &mut Vec<u32>) {
        let ty = self.resolve(ty);
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::InferVar(v) => out.push(v),
            TypeKind::Function { from, to } => {
                self.free_infer_vars_into(from, out);
                self.free_infer_vars_into(to, out);
            }
            TypeKind::List(inner) | TypeKind::Optional(inner) => {
                self.free_infer_vars_into(inner, out);
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(payload) = v.payload {
                        self.free_infer_vars_into(payload, out);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let inner = match item {
                        TypeTupleItem::Named { ty, .. } => ty,
                        TypeTupleItem::Positional(ty) => ty,
                    };
                    self.free_infer_vars_into(inner, out);
                }
            }
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.free_infer_vars_into(f.ty, out);
                }
            }
            TypeKind::AliasApply { args, .. } => {
                for a in args {
                    self.free_infer_vars_into(a, out);
                }
            }
            _ => {}
        }
    }

    /// All `InferVar` ids free in the stored type of any binding other than
    /// `exclude`. These are "in the environment": generalizing them would be
    /// unsound, so they stay monomorphic.
    fn env_infer_vars(&self, exclude: BindingId) -> HashSet<u32> {
        let mut set = HashSet::new();
        for (&binding, &ty) in &self.value_types {
            if binding == exclude {
                continue;
            }
            for v in self.free_infer_vars_in(ty) {
                set.insert(v);
            }
        }
        set
    }

    /// HM "gen" rule: generalize `binding`'s free inference variables that are not
    /// shared with the surrounding environment. Call AFTER the binding's body is
    /// fully lowered and its type is in `value_types`.
    ///
    /// Source-order / define-before-use: only references that appear textually
    /// after this point observe the scheme. Polymorphic recursion is not inferred.
    pub(super) fn generalize_if_polymorphic(&mut self, binding: BindingId, ty: TypeId) {
        let env = self.env_infer_vars(binding);
        let scheme: Vec<u32> = self
            .free_infer_vars_in(ty)
            .into_iter()
            .filter(|v| !env.contains(v))
            .collect();
        if !scheme.is_empty() {
            self.poly_schemes.insert(binding, scheme);
        }
    }

    /// Substitute `InferVar`s appearing in `ty` according to `subst`, allocating
    /// new nodes only where a substitution occurs. Unlike `instantiate_type_vars`,
    /// this resolves chains at entry because stored signatures contain
    /// partially-solved `InferVar`s.
    pub(super) fn instantiate_infer_vars(
        &mut self,
        ty: TypeId,
        subst: &HashMap<u32, TypeId>,
    ) -> TypeId {
        let ty = self.resolve(ty);
        if subst.is_empty() {
            return ty;
        }
        let span = self.type_arena[ty.0 as usize].span;
        match self.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::InferVar(v) => subst.get(&v).copied().unwrap_or(ty),
            TypeKind::Function { from, to } => {
                let new_from = self.instantiate_infer_vars(from, subst);
                let new_to = self.instantiate_infer_vars(to, subst);
                if new_from == from && new_to == to {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Function {
                        from: new_from,
                        to: new_to,
                    },
                    span,
                })
            }
            TypeKind::List(inner) => {
                let new_inner = self.instantiate_infer_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::List(new_inner),
                    span,
                })
            }
            TypeKind::Optional(inner) => {
                let new_inner = self.instantiate_infer_vars(inner, subst);
                if new_inner == inner {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Optional(new_inner),
                    span,
                })
            }
            TypeKind::Union(variants, tail) => {
                let new_variants: Vec<UnionVariant> = variants
                    .iter()
                    .map(|v| UnionVariant {
                        name: v.name.clone(),
                        payload: v.payload.map(|p| self.instantiate_infer_vars(p, subst)),
                        span: v.span,
                    })
                    .collect();
                if new_variants
                    .iter()
                    .zip(variants.iter())
                    .all(|(n, o)| n.payload == o.payload)
                {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Union(new_variants, tail),
                    span,
                })
            }
            TypeKind::Tuple(items) => {
                let new_items: Vec<TypeTupleItem> = items
                    .iter()
                    .map(|item| match item {
                        TypeTupleItem::Named {
                            name,
                            ty: inner,
                            span: s,
                        } => TypeTupleItem::Named {
                            name: name.clone(),
                            ty: self.instantiate_infer_vars(*inner, subst),
                            span: *s,
                        },
                        TypeTupleItem::Positional(inner) => {
                            TypeTupleItem::Positional(self.instantiate_infer_vars(*inner, subst))
                        }
                    })
                    .collect();
                if new_items == items {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Tuple(new_items),
                    span,
                })
            }
            TypeKind::Record(fields, tail) => {
                let new_fields: Vec<TypeRecordField> = fields
                    .iter()
                    .map(|f| TypeRecordField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: self.instantiate_infer_vars(f.ty, subst),
                        span: f.span,
                    })
                    .collect();
                if new_fields == fields {
                    return ty;
                }
                self.alloc_type(Type {
                    kind: TypeKind::Record(new_fields, tail),
                    span,
                })
            }
            _ => ty,
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

/// Encode a row tail into a structural coherence/dispatch key suffix. `Closed`
/// adds nothing, so closed (concrete) witness targets key exactly as before;
/// open and row-variable tails get a distinct marker so they never collide with
/// a closed target. Must stay in sync with the evaluator's `type_key`.
fn row_tail_key(tail: RowTail) -> String {
    match tail {
        RowTail::Closed => String::new(),
        RowTail::Open => "...".to_string(),
        RowTail::Param(b) => format!("...#{}", b.0),
        RowTail::Infer(v) => format!("...?{v}"),
    }
}
