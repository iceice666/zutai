use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;
use zutai_thir::{ThirDeclKind, TypeId, TypeKind, TypeTupleItem};

use super::witness::{WitnessTargetKey, row_tail_key};
use super::*;

impl<'thir> Lowerer<'thir> {
    pub(super) fn thir_type_to_resolved_witness_key(&self, ty: TypeId) -> Option<WitnessTargetKey> {
        let key = self.structural_witness_key(ty, &mut FxHashSet::default())?;
        match key.as_str() {
            "Int" => Some(WitnessTargetKey::Int),
            "Float" => Some(WitnessTargetKey::Float),
            "Bool" => Some(WitnessTargetKey::Bool),
            "Text" => Some(WitnessTargetKey::Str),
            "Atom" => Some(WitnessTargetKey::Atom),
            name if name.starts_with("Posit") => {
                let spec = zutai_syntax::posit::parse_posit_type_name(name)?;
                Some(WitnessTargetKey::Posit(spec))
            }
            _ => Some(WitnessTargetKey::Structural(key)),
        }
    }

    /// Flatten a curried THIR `Apply` chain into head + left-to-right args.
    pub(super) fn thir_app_spine(&self, ty: TypeId) -> (TypeId, Vec<TypeId>) {
        let mut args: Vec<TypeId> = Vec::new();
        let mut cur = ty;
        while let TypeKind::Apply { func, arg } = self.thir.type_arena[cur.0 as usize].kind {
            args.push(arg);
            cur = func;
        }
        args.reverse();
        (cur, args)
    }

    /// The THIR signature of constraint method `name`, by scanning the constraint
    /// decl. Used at a call site to recover the method's exact type-var order.
    pub(super) fn method_sig_for(&self, constraint: BindingId, name: &str) -> Option<TypeId> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint { methods, .. } = &decl.kind
            {
                return methods.iter().find(|m| m.name == name).map(|m| m.sig);
            }
            None
        })
    }

    /// Collect the `TypeVar` bindings free in a THIR type, deduped and sorted by
    /// binding id — exactly reproducing THIR's `collect_type_vars` order, so the
    /// result is positionally aligned with a call site's `instantiation` vector.
    pub(super) fn collect_thir_type_vars(&self, ty: TypeId) -> Vec<BindingId> {
        let mut out: Vec<BindingId> = Vec::new();
        self.collect_thir_type_vars_into(ty, &mut out);
        out.sort_by_key(|b| b.0);
        out.dedup();
        out
    }

    pub(super) fn collect_thir_type_vars_into(&self, ty: TypeId, out: &mut Vec<BindingId>) {
        match &self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(b) => out.push(*b),
            TypeKind::Function { from, to } => {
                self.collect_thir_type_vars_into(*from, out);
                self.collect_thir_type_vars_into(*to, out);
            }
            TypeKind::Effect { base, row } => {
                self.collect_thir_type_vars_into(*base, out);
                for op in &row.ops {
                    self.collect_thir_type_vars_into(op.param, out);
                    self.collect_thir_type_vars_into(op.result, out);
                }
            }
            TypeKind::List(e)
            | TypeKind::Optional(e)
            | TypeKind::Maybe(e)
            | TypeKind::Patch { target: e, .. } => self.collect_thir_type_vars_into(*e, out),
            TypeKind::Apply { func, arg } => {
                self.collect_thir_type_vars_into(*func, out);
                self.collect_thir_type_vars_into(*arg, out);
            }
            TypeKind::AliasApply { args, .. } => {
                for &a in args {
                    self.collect_thir_type_vars_into(a, out);
                }
            }
            TypeKind::Record(fields, _) => {
                for f in fields {
                    self.collect_thir_type_vars_into(f.ty, out);
                }
            }
            TypeKind::Union(variants, _) => {
                for v in variants {
                    if let Some(p) = v.payload {
                        self.collect_thir_type_vars_into(p, out);
                    }
                }
            }
            TypeKind::Tuple(items) => {
                for item in items {
                    let t = match item {
                        TypeTupleItem::Named { ty, .. } => *ty,
                        TypeTupleItem::Positional(ty) => *ty,
                    };
                    self.collect_thir_type_vars_into(t, out);
                }
            }
            _ => {}
        }
    }

    pub(super) fn structural_witness_key(
        &self,
        ty: TypeId,
        seen: &mut FxHashSet<BindingId>,
    ) -> Option<String> {
        match self.thir.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::Int => Some("Int".to_string()),
            TypeKind::Float => Some("Float".to_string()),
            TypeKind::FixedNum(fw) => Some(fw.name().to_string()),
            TypeKind::Posit(spec) => Some(spec.type_name()),
            TypeKind::Bool | TypeKind::True | TypeKind::False => Some("Bool".to_string()),
            TypeKind::Text => Some("Text".to_string()),
            TypeKind::Opaque(name) => Some(name),
            TypeKind::Atom(name) => Some(format!("#{name}")),
            TypeKind::List(inner) => {
                Some(format!("[{}]", self.structural_witness_key(inner, seen)?))
            }
            TypeKind::Optional(inner) => {
                Some(format!("{}?", self.structural_witness_key(inner, seen)?))
            }
            TypeKind::Maybe(inner) => Some(format!(
                "Maybe[{}]",
                self.structural_witness_key(inner, seen)?
            )),
            TypeKind::Patch { target, deep } => {
                let head = if deep { "DeepPatch" } else { "Patch" };
                Some(format!(
                    "{head}[{}]",
                    self.structural_witness_key(target, seen)?
                ))
            }
            TypeKind::Record(fields, tail) => {
                let mut parts: Vec<String> = fields
                    .into_iter()
                    .map(|field| {
                        let key = self.structural_witness_key(field.ty, seen)?;
                        let marker = if field.optional { "?:" } else { ":" };
                        Some(format!("{}{}{}", field.name, marker, key))
                    })
                    .collect::<Option<_>>()?;
                parts.sort();
                Some(format!("{{{}{}}}", parts.join(","), row_tail_key(tail)))
            }
            TypeKind::Union(variants, tail) => {
                let parts: Vec<String> = variants
                    .into_iter()
                    .map(|variant| match variant.payload {
                        Some(payload) => Some(format!(
                            "{}({})",
                            variant.name,
                            self.structural_witness_key(payload, seen)?
                        )),
                        None => Some(variant.name),
                    })
                    .collect::<Option<_>>()?;
                Some(format!("<{}{}>", parts.join("|"), row_tail_key(tail)))
            }
            TypeKind::Tuple(items) => {
                let parts: Vec<String> = items
                    .into_iter()
                    .map(|item| match item {
                        TypeTupleItem::Named { name, ty, .. } => Some(format!(
                            "{}:{}",
                            name,
                            self.structural_witness_key(ty, seen)?
                        )),
                        TypeTupleItem::Positional(ty) => self.structural_witness_key(ty, seen),
                    })
                    .collect::<Option<_>>()?;
                Some(format!("({})", parts.join(",")))
            }
            TypeKind::Function { from, to } => Some(format!(
                "({}->{})",
                self.structural_witness_key(from, seen)?,
                self.structural_witness_key(to, seen)?
            )),
            TypeKind::Alias(binding) => {
                if !seen.insert(binding) {
                    return None;
                }
                let body = self.type_alias_body(binding)?;
                self.structural_witness_key(body, seen)
            }
            TypeKind::AliasApply { binding, args } => {
                if !seen.insert(binding) {
                    return None;
                }
                let (params, body) = self.type_alias_params_body(binding)?;
                self.structural_witness_key_subst(
                    body,
                    &params.into_iter().zip(args).collect(),
                    seen,
                )
            }
            TypeKind::Con(binding) => Some(format!("@{}", binding.0)),
            TypeKind::Apply { .. } => {
                let (head, args) = self.thir_app_spine(ty);
                // Saturated named-alias application keys like the AliasApply arm.
                if let TypeKind::Alias(binding) = self.thir.type_arena[head.0 as usize].kind {
                    if !seen.insert(binding) {
                        return None;
                    }
                    if let Some((params, body)) = self.type_alias_params_body(binding)
                        && params.len() == args.len()
                    {
                        return self.structural_witness_key_subst(
                            body,
                            &params.into_iter().zip(args).collect(),
                            seen,
                        );
                    }
                }
                let head_key = self.structural_witness_key(head, seen)?;
                let arg_keys: Vec<String> = args
                    .iter()
                    .map(|&a| self.structural_witness_key(a, seen))
                    .collect::<Option<_>>()?;
                Some(format!("{}[{}]", head_key, arg_keys.join(",")))
            }
            TypeKind::Effect { base, .. } => self.structural_witness_key(base, seen),
            TypeKind::Never => Some("Never".to_string()),
            TypeKind::TypeVar(binding) => Some(format!("@{}", binding.0)),
            TypeKind::InferVar(v) => Some(format!("?{v}")),
            TypeKind::ForAll { .. } => None,
            TypeKind::Type | TypeKind::Error => None,
        }
    }

    pub(super) fn structural_witness_key_subst(
        &self,
        ty: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
        seen: &mut FxHashSet<BindingId>,
    ) -> Option<String> {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::TypeVar(binding) => subst
                .get(&binding)
                .copied()
                .map(|replacement| self.structural_witness_key(replacement, seen))
                .unwrap_or_else(|| Some(format!("@{}", binding.0))),
            _ => self.structural_witness_key(ty, seen),
        }
    }

    pub(super) fn type_alias_body(&self, binding: BindingId) -> Option<TypeId> {
        self.type_alias_params_body(binding).map(|(_, body)| body)
    }

    pub(super) fn type_alias_params_body(
        &self,
        binding: BindingId,
    ) -> Option<(Vec<BindingId>, TypeId)> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == binding
                && let zutai_thir::ThirDeclKind::TypeAlias { params, ty } = &decl.kind
            {
                return Some((params.clone(), *ty));
            }
            None
        })
    }
}
