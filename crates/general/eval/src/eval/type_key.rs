use super::*;

/// Structural type key mirroring `witness_target_key` in the THIR lowerer.
///
/// Resolves named `Alias` chains and expands parametric `AliasApply` (with its
/// type args substituted) so a witness target written as `Pair A` matches an
/// operand whose inferred type is the equivalent structural record.
pub(super) fn type_key(
    type_arena: &[Type],
    aliases: &FxHashMap<BindingId, (Vec<BindingId>, TypeId)>,
    ty: TypeId,
) -> String {
    type_key_subst(type_arena, aliases, &FxHashMap::default(), ty, 0)
}

pub(super) fn type_key_subst(
    type_arena: &[Type],
    aliases: &FxHashMap<BindingId, (Vec<BindingId>, TypeId)>,
    subst: &FxHashMap<BindingId, TypeId>,
    ty: TypeId,
    depth: u32,
) -> String {
    // A structurally-recursive parametric alias (e.g. `Rec :: <A> type { #rec: A; }`)
    // would expand forever; cap the depth and fall back to an ambiguous marker so
    // dispatch refuses rather than overflowing the stack.
    if depth > 256 {
        return format!("$deep{}", ty.0);
    }
    let d = depth + 1;
    let ty = resolve_alias_chain(type_arena, aliases, ty);
    match &type_arena[ty.0 as usize].kind {
        TypeKind::Int => "Int".into(),
        TypeKind::Bool => "Bool".into(),
        TypeKind::Text => "Text".into(),
        TypeKind::Float => "Float".into(),
        TypeKind::FixedNum(fw) => fw.name().into(),
        TypeKind::Posit(spec) => spec.type_name(),
        TypeKind::Opaque(name) => name.clone(),
        TypeKind::Type(_) => "Type".into(),
        TypeKind::True => "true".into(),
        TypeKind::False => "false".into(),
        TypeKind::Atom(a) => format!("#{a}"),
        TypeKind::List(inner) => {
            format!(
                "[{}]",
                type_key_subst(type_arena, aliases, subst, *inner, d)
            )
        }
        TypeKind::Optional(inner) => {
            format!("{}?", type_key_subst(type_arena, aliases, subst, *inner, d))
        }
        TypeKind::Maybe(inner) => {
            format!(
                "Maybe[{}]",
                type_key_subst(type_arena, aliases, subst, *inner, d)
            )
        }
        TypeKind::Patch { target, deep } => {
            let head = if *deep { "DeepPatch" } else { "Patch" };
            format!(
                "{head}[{}]",
                type_key_subst(type_arena, aliases, subst, *target, d)
            )
        }
        TypeKind::Record(fields, tail) => {
            let mut parts: Vec<String> = fields
                .iter()
                .map(|f| {
                    format!(
                        "{}:{}",
                        f.name,
                        type_key_subst(type_arena, aliases, subst, f.ty, d)
                    )
                })
                .collect();
            parts.sort();
            format!("{{{}{}}}", parts.join(","), row_tail_key(*tail))
        }
        TypeKind::Union(variants, tail) => {
            let parts: Vec<String> = variants
                .iter()
                .map(|v| match v.payload {
                    Some(p) => {
                        format!(
                            "{}({})",
                            v.name,
                            type_key_subst(type_arena, aliases, subst, p, d)
                        )
                    }
                    None => v.name.clone(),
                })
                .collect();
            format!("<{}{}>", parts.join("|"), row_tail_key(*tail))
        }
        TypeKind::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    TypeTupleItem::Named { name, ty, .. } => {
                        format!(
                            "{}:{}",
                            name,
                            type_key_subst(type_arena, aliases, subst, *ty, d)
                        )
                    }
                    TypeTupleItem::Positional(ty) => {
                        type_key_subst(type_arena, aliases, subst, *ty, d)
                    }
                })
                .collect();
            format!("({})", parts.join(","))
        }
        TypeKind::Function { from, to } => {
            format!(
                "({}->{}",
                type_key_subst(type_arena, aliases, subst, *from, d),
                type_key_subst(type_arena, aliases, subst, *to, d)
            )
        }
        TypeKind::Effect { base, .. } => type_key_subst(type_arena, aliases, subst, *base, d),
        TypeKind::Never => "Never".into(),
        TypeKind::Alias(b) => format!("@{}", b.0),
        TypeKind::TypeVar(b) => match subst.get(b) {
            Some(&t) => type_key_subst(type_arena, aliases, subst, t, d),
            None => format!("@{}", b.0),
        },
        TypeKind::AliasApply { binding, args } => {
            // Expand the parametric alias: substitute its params with the applied
            // args and re-key the body, so `Pair Int` keys as `{fst:Int,snd:Int}`
            // and matches a structurally-keyed witness target.
            if let Some((params, body)) = aliases.get(binding)
                && params.len() == args.len()
            {
                let mut child = subst.clone();
                for (p, a) in params.iter().zip(args.iter()) {
                    child.insert(*p, *a);
                }
                return type_key_subst(type_arena, aliases, &child, *body, d);
            }
            let arg_parts: Vec<String> = args
                .iter()
                .map(|a| type_key_subst(type_arena, aliases, subst, *a, d))
                .collect();
            format!("${}[{}]", binding.0, arg_parts.join(","))
        }
        TypeKind::Con(b) => format!("@{}", b.0),
        TypeKind::Apply { .. } => {
            // Flatten the curried spine to head + args.
            let mut args_acc: Vec<TypeId> = Vec::new();
            let mut cur = ty;
            while let TypeKind::Apply { func, arg } = &type_arena[cur.0 as usize].kind {
                args_acc.push(*arg);
                cur = *func;
            }
            args_acc.reverse();
            // Saturated named-alias head: expand + substitute (mirror AliasApply).
            if let TypeKind::Alias(b) = &type_arena[cur.0 as usize].kind
                && let Some((params, body)) = aliases.get(b)
                && params.len() == args_acc.len()
            {
                let mut child = subst.clone();
                for (p, a) in params.iter().zip(args_acc.iter()) {
                    child.insert(*p, *a);
                }
                return type_key_subst(type_arena, aliases, &child, *body, d);
            }
            let head_key = type_key_subst(type_arena, aliases, subst, cur, d);
            let arg_parts: Vec<String> = args_acc
                .iter()
                .map(|a| type_key_subst(type_arena, aliases, subst, *a, d))
                .collect();
            format!("{}[{}]", head_key, arg_parts.join(","))
        }
        TypeKind::ForAll { params, body, .. } => {
            let param_parts: Vec<String> = params.iter().map(|b| format!("@{}", b.0)).collect();
            format!(
                "forall[{}].{}",
                param_parts.join(","),
                type_key_subst(type_arena, aliases, subst, *body, d)
            )
        }
        TypeKind::InferVar(v) => format!("?{v}"),
        TypeKind::Error => "<error>".into(),
    }
}

/// Row-tail key suffix, mirroring the THIR lowerer's `row_tail_key`. `Closed`
/// adds nothing so concrete witness targets key exactly as before; open and
/// row-variable tails get a distinct marker.
pub(super) fn row_tail_key(tail: RowTail) -> String {
    match tail {
        RowTail::Closed => String::new(),
        RowTail::Open => "...".to_string(),
        RowTail::Param(b) => format!("...#{}", b.0),
        RowTail::Infer(v) => format!("...?{v}"),
    }
}

/// Repeatedly resolves `TypeKind::Alias` entries through `aliases` until
/// a non-alias type or an unknown alias is reached.
pub(super) fn resolve_alias_chain(
    type_arena: &[Type],
    aliases: &FxHashMap<BindingId, (Vec<BindingId>, TypeId)>,
    mut ty: TypeId,
) -> TypeId {
    let mut fuel = 64u8;
    while fuel > 0 {
        match &type_arena[ty.0 as usize].kind {
            TypeKind::Alias(b) => match aliases.get(b) {
                Some((_, next)) => {
                    ty = *next;
                    fuel -= 1;
                }
                None => break,
            },
            _ => break,
        }
    }
    ty
}

/// Returns `true` if the type key contains unresolvable components
/// (`?` for `InferVar`, `$` for `AliasApply`) that could cause a
/// dispatch miss despite a witness being present.
pub(super) fn key_is_ambiguous(key: &str) -> bool {
    key.starts_with('@') || key.contains('?') || key.contains('$')
}
