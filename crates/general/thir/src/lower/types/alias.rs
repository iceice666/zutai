use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn resolve_alias(
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
                if params.len() != args.len() {
                    return ty; // partial application → leave inert (not a saturated type)
                }
                let Some(body) = self.aliases.get(&binding).copied() else {
                    return ty;
                };
                let subst: HashMap<BindingId, TypeId> = params.into_iter().zip(args).collect();
                let expanded = self.instantiate_type_vars(body, &subst);
                self.resolve_alias(expanded, seen, span)
            }
            TypeKind::Apply { .. } => {
                // Canonical reducer for curried applications: fold builtin `Con`
                // applications, expand saturated named-alias applications, and
                // leave abstract (var-headed) or under-saturated heads inert.
                let (head, spine_args) = self.app_spine(ty);
                let head = self.resolve(head);
                match self.type_arena[head.0 as usize].kind.clone() {
                    TypeKind::Con(b) => {
                        let name = self.hir.bindings[b.0 as usize].name.clone();
                        match (name.as_str(), spine_args.len()) {
                            ("List", 1) => self.alloc_type(Type {
                                kind: TypeKind::List(spine_args[0]),
                                span,
                            }),
                            ("Optional", 1) => self.optional_type(spine_args[0], span),
                            _ => ty, // partial/over-applied builtin → inert
                        }
                    }
                    TypeKind::Alias(b) => {
                        let Some(params) = self.alias_params.get(&b).cloned() else {
                            return ty;
                        };
                        if params.len() != spine_args.len() {
                            return ty; // partial → inert
                        }
                        if !seen.insert(b) {
                            self.push_alias_cycle(b, span);
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
                        let Some(body) = self.aliases.get(&b).copied() else {
                            return ty;
                        };
                        let subst: HashMap<BindingId, TypeId> =
                            params.into_iter().zip(spine_args).collect();
                        let expanded = self.instantiate_type_vars(body, &subst);
                        self.resolve_alias(expanded, seen, span)
                    }
                    _ => ty, // abstract head (TypeVar / InferVar) → inert
                }
            }
            _ => ty,
        }
    }

    pub(in crate::lower) fn push_alias_cycle(&mut self, binding: BindingId, span: Span) {
        let name = self.hir.bindings[binding.0 as usize].name.clone();
        self.diagnostics.push(ThirDiagnostic {
            kind: ThirDiagnosticKind::AliasCycle { name },
            span,
        });
    }

    pub(in crate::lower) fn type_name(&mut self, ty: TypeId) -> String {
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
            TypeKind::Record(fields, tail) => self.record_type_name(fields, tail),
            TypeKind::Union(_, _) => "union".to_string(),
            TypeKind::Tuple(_) => "tuple".to_string(),
            TypeKind::Function { .. } => "function".to_string(),
            TypeKind::Effect { base, row } => {
                format!("{}!{}", self.type_name(base), self.effect_row_name(&row))
            }
            TypeKind::Never => "Never".to_string(),
            TypeKind::TypeVar(binding) | TypeKind::Alias(binding) => {
                self.hir.bindings[binding.0 as usize].name.clone()
            }
            TypeKind::AliasApply { binding, args } => {
                let head = self.hir.bindings[binding.0 as usize].name.clone();
                let parts: Vec<String> = args.iter().map(|&a| self.type_name(a)).collect();
                format!("{head} {}", parts.join(" "))
            }
            TypeKind::Con(binding) => self.hir.bindings[binding.0 as usize].name.clone(),
            TypeKind::Apply { func, arg } => {
                format!("{} {}", self.type_name(func), self.type_name(arg))
            }
            TypeKind::InferVar(v) => format!("?{v}"),
            TypeKind::Error => "<error>".to_string(),
        }
    }

    fn record_type_name(&mut self, fields: Vec<TypeRecordField>, tail: RowTail) -> String {
        let (fields, tail) = self.flatten_record_row(fields, tail);
        let mut rendered = String::from("{ ");

        for field in &fields {
            rendered.push_str(&self.record_field_type_name(field));
            rendered.push_str("; ");
        }

        match tail {
            RowTail::Closed => {}
            RowTail::Open => rendered.push_str("...; "),
            RowTail::Param(binding) => {
                rendered.push_str("...");
                rendered.push_str(&self.hir.bindings[binding.0 as usize].name);
                rendered.push_str("; ");
            }
            RowTail::Infer(id) => {
                rendered.push_str("...?");
                rendered.push_str(&id.to_string());
                rendered.push_str("; ");
            }
        }

        rendered.push('}');
        rendered
    }

    pub(in crate::lower) fn record_field_type_name(&mut self, field: &TypeRecordField) -> String {
        let ty = self.type_name(field.ty);
        let mut rendered = String::with_capacity(field.name.len() + ty.len() + 4);
        rendered.push_str(&field.name);
        if field.optional {
            rendered.push_str("? : ");
        } else {
            rendered.push_str(" : ");
        }
        rendered.push_str(&ty);
        rendered
    }

    pub(in crate::lower) fn union_variant_type_name(&mut self, variant: &UnionVariant) -> String {
        let mut rendered = String::with_capacity(variant.name.len() + 1);
        rendered.push('#');
        rendered.push_str(&variant.name);
        if let Some(payload) = variant.payload {
            let ty = self.type_name(payload);
            rendered.reserve(ty.len() + 3);
            rendered.push_str(" : ");
            rendered.push_str(&ty);
        }
        rendered
    }

    /// Structural coherence key for a witness target type.
    ///
    /// Unlike `type_name`, this function recurses into compound types
    /// (`Record`, `Union`, `Tuple`, `Function`) so that distinct types
    /// always produce distinct keys. This is used as the second half of
    /// the coherence-check map key `(constraint BindingId, target key)`.
    pub(in crate::lower) fn witness_target_key(&mut self, ty: TypeId) -> String {
        self.witness_target_key_with(ty, &HashMap::new())
    }

    /// Like `witness_target_key`, but each binding in `norm` (a witness's own
    /// type params) keys to its positional `#index` instead of `@<binding>`, so
    /// two conditional witnesses that differ only in param identity — e.g. two
    /// `Eq @(List A)` — produce the same key and are flagged as conflicting.
    pub(in crate::lower) fn witness_target_key_with(
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
            TypeKind::Effect { base, row } => {
                let ops = row
                    .ops
                    .iter()
                    .map(|op| {
                        format!(
                            "{}:{}->{}",
                            op.name,
                            self.witness_target_key_with(op.param, norm),
                            self.witness_target_key_with(op.result, norm)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{}!{{{}{}}}",
                    self.witness_target_key_with(base, norm),
                    ops,
                    row_tail_key(row.tail)
                )
            }
            TypeKind::Never => "Never".to_string(),
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
            TypeKind::Con(binding) => format!("@{}", binding.0),
            TypeKind::Apply { .. } => {
                let (head, args) = self.app_spine(ty);
                let head_key = match self.type_arena[head.0 as usize].kind.clone() {
                    TypeKind::TypeVar(b) => match norm.get(&b) {
                        Some(i) => format!("#{i}"),
                        None => format!("@{}", b.0),
                    },
                    TypeKind::Alias(b) | TypeKind::Con(b) => format!("@{}", b.0),
                    _ => self.witness_target_key_with(head, norm),
                };
                let parts: Vec<String> = args
                    .iter()
                    .map(|&a| self.witness_target_key_with(a, norm))
                    .collect();
                format!("{}[{}]", head_key, parts.join(","))
            }
            TypeKind::InferVar(v) => format!("?{v}"),
            TypeKind::Error => "<error>".to_string(),
        }
    }
}

/// Encode a row tail into a structural coherence/dispatch key suffix. `Closed`
/// adds nothing, so closed (concrete) witness targets key exactly as before;
/// open and row-variable tails get a distinct marker so they never collide with
/// a closed target. Must stay in sync with the evaluator's `type_key`.
pub(super) fn row_tail_key(tail: RowTail) -> String {
    match tail {
        RowTail::Closed => String::new(),
        RowTail::Open => "...".to_string(),
        RowTail::Param(b) => format!("...#{}", b.0),
        RowTail::Infer(v) => format!("...?{v}"),
    }
}
