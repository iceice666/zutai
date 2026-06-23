//! Arena-independent structural pattern for a conditional (parametric) witness
//! target, used for cross-module conditional witness dispatch (Phase B).
//!
//! A conditional witness such as `Eq @(List A) :: <A: Eq>` cannot be matched at
//! an importer's concrete call site by an exact `target_key` string, because the
//! target carries the witness type parameters as holes. [`export_witness_pattern`]
//! walks the witness target into a [`WitnessPattern`] in which each occurrence of
//! a witness parameter becomes a [`WitnessPattern::Hole`] keyed by the parameter's
//! position. The importer matches this pattern against its concrete operand type,
//! recovers the type bound to each hole, and applies the dependency's witness
//! function to the recursively-resolved component dictionaries.

use rustc_hash::FxHashMap;

use zutai_hir::BindingId;

use crate::ir::{RowTail, ThirDeclKind, ThirFile, TypeId, TypeKind, TypeTupleItem};

/// Arena-independent structural matcher for a parametric witness target.
///
/// Concrete leaves (primitives, atoms) collapse to [`WitnessPattern::Leaf`]
/// carrying the leaf's structural witness key; witness parameters become
/// [`WitnessPattern::Hole`]; composite constructors stay structural so a hole
/// nested inside them is recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessPattern {
    /// A witness type parameter, identified by its index in the witness's
    /// parameter list (parallel to `param_bounds`).
    Hole(usize),
    /// A concrete leaf, identified by its structural witness key string
    /// (`"Int"`, `"Bool"`, `"Text"`, `"#ok"`, a fixed-width or posit name…).
    Leaf(String),
    List(Box<WitnessPattern>),
    Optional(Box<WitnessPattern>),
    Maybe(Box<WitnessPattern>),
    Record(Vec<WitnessPatternField>),
    Tuple(Vec<WitnessPatternTupleItem>),
    Union(Vec<WitnessPatternVariant>),
    Function(Box<WitnessPattern>, Box<WitnessPattern>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessPatternField {
    pub name: String,
    pub optional: bool,
    pub ty: WitnessPattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessPatternTupleItem {
    Positional(WitnessPattern),
    Named { name: String, ty: WitnessPattern },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessPatternVariant {
    pub name: String,
    pub payload: Option<WitnessPattern>,
}

/// Build a [`WitnessPattern`] for a conditional witness `target`, treating each
/// `BindingId` in `params` as a hole. Returns `None` when the target contains a
/// free non-parameter type variable or a shape that cannot cross a module
/// boundary structurally (open rows, effects, unresolved/quantified types) — the
/// importer's gate keeps rejecting such witnesses rather than miscompiling.
pub fn export_witness_pattern(
    file: &ThirFile,
    target: TypeId,
    params: &[BindingId],
) -> Option<WitnessPattern> {
    let aliases = build_alias_params_body(file);
    let env = FxHashMap::default();
    pat_of(file, &aliases, target, &env, params, 0)
}

type AliasMap = FxHashMap<BindingId, (Vec<BindingId>, TypeId)>;

fn build_alias_params_body(file: &ThirFile) -> AliasMap {
    let mut map = AliasMap::default();
    for (_, decl) in file.decl_arena.iter() {
        if let ThirDeclKind::TypeAlias { params, ty } = &decl.kind {
            map.insert(decl.binding, (params.clone(), *ty));
        }
    }
    map
}

/// Normalize `ty` under `env`: follow non-hole `TypeVar` substitutions and expand
/// aliases (recording their args in the returned env) until the head is a
/// concrete constructor, a hole, or a free variable.
fn norm(
    file: &ThirFile,
    aliases: &AliasMap,
    ty: TypeId,
    env: &FxHashMap<BindingId, TypeId>,
    params: &[BindingId],
) -> (TypeId, FxHashMap<BindingId, TypeId>) {
    let mut ty = ty;
    let mut env = env.clone();
    let mut fuel = 64u32;
    while fuel > 0 {
        fuel -= 1;
        match file.type_arena[ty.0 as usize].kind.clone() {
            TypeKind::TypeVar(b) if !params.contains(&b) => match env.get(&b) {
                Some(&next) => ty = next,
                None => break,
            },
            TypeKind::Alias(b) => match aliases.get(&b) {
                Some((_, body)) => ty = *body,
                None => break,
            },
            TypeKind::AliasApply { binding, args } => match aliases.get(&binding) {
                Some((aparams, body)) => {
                    for (p, a) in aparams.iter().zip(args.iter()) {
                        env.insert(*p, *a);
                    }
                    ty = *body;
                }
                None => break,
            },
            TypeKind::Apply { .. } => {
                let (head, args) = app_spine(file, ty);
                if let TypeKind::Alias(b) = file.type_arena[head.0 as usize].kind
                    && let Some((aparams, body)) = aliases.get(&b)
                    && aparams.len() == args.len()
                {
                    for (p, a) in aparams.iter().zip(args.iter()) {
                        env.insert(*p, *a);
                    }
                    ty = *body;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    (ty, env)
}

fn app_spine(file: &ThirFile, ty: TypeId) -> (TypeId, Vec<TypeId>) {
    let mut args = Vec::new();
    let mut head = ty;
    while let TypeKind::Apply { func, arg } = file.type_arena[head.0 as usize].kind {
        args.push(arg);
        head = func;
    }
    args.reverse();
    (head, args)
}

fn pat_of(
    file: &ThirFile,
    aliases: &AliasMap,
    ty: TypeId,
    env: &FxHashMap<BindingId, TypeId>,
    params: &[BindingId],
    depth: u32,
) -> Option<WitnessPattern> {
    if depth > 64 {
        return None;
    }
    let (ty, env) = norm(file, aliases, ty, env, params);
    let kind = file.type_arena[ty.0 as usize].kind.clone();
    if let TypeKind::TypeVar(b) = kind {
        let idx = params.iter().position(|p| *p == b)?;
        return Some(WitnessPattern::Hole(idx));
    }
    let recur = |t: TypeId| pat_of(file, aliases, t, &env, params, depth + 1);
    match kind {
        TypeKind::Int => Some(WitnessPattern::Leaf("Int".to_string())),
        TypeKind::Float => Some(WitnessPattern::Leaf("Float".to_string())),
        TypeKind::FixedNum(fw) => Some(WitnessPattern::Leaf(fw.name().to_string())),
        TypeKind::Posit(spec) => Some(WitnessPattern::Leaf(spec.type_name())),
        TypeKind::Bool | TypeKind::True | TypeKind::False => {
            Some(WitnessPattern::Leaf("Bool".to_string()))
        }
        TypeKind::Text => Some(WitnessPattern::Leaf("Text".to_string())),
        TypeKind::Opaque(name) => Some(WitnessPattern::Leaf(name)),
        TypeKind::Atom(name) => Some(WitnessPattern::Leaf(format!("#{name}"))),
        TypeKind::List(inner) => Some(WitnessPattern::List(Box::new(recur(inner)?))),
        TypeKind::Optional(inner) => Some(WitnessPattern::Optional(Box::new(recur(inner)?))),
        TypeKind::Maybe(inner) => Some(WitnessPattern::Maybe(Box::new(recur(inner)?))),
        TypeKind::Record(fields, RowTail::Closed) => {
            let mut out: Vec<WitnessPatternField> = fields
                .iter()
                .map(|f| {
                    Some(WitnessPatternField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: recur(f.ty)?,
                    })
                })
                .collect::<Option<_>>()?;
            // Match `structural_witness_key`'s ordering, which sorts the rendered
            // `"{name}{marker}{value}"` parts. Field names are unique, so the
            // name+marker prefix decides the order (the value never tie-breaks);
            // sorting by that prefix reproduces the dispatch-key field order the
            // interpreter parses positionally.
            out.sort_by_key(|f| format!("{}{}", f.name, if f.optional { "?:" } else { ":" }));
            Some(WitnessPattern::Record(out))
        }
        TypeKind::Tuple(items) => {
            let out: Vec<WitnessPatternTupleItem> = items
                .iter()
                .map(|item| match item {
                    TypeTupleItem::Positional(t) => {
                        Some(WitnessPatternTupleItem::Positional(recur(*t)?))
                    }
                    TypeTupleItem::Named { name, ty, .. } => Some(WitnessPatternTupleItem::Named {
                        name: name.clone(),
                        ty: recur(*ty)?,
                    }),
                })
                .collect::<Option<_>>()?;
            Some(WitnessPattern::Tuple(out))
        }
        TypeKind::Union(variants, RowTail::Closed) => {
            let out: Vec<WitnessPatternVariant> = variants
                .iter()
                .map(|v| {
                    Some(WitnessPatternVariant {
                        name: v.name.clone(),
                        payload: match v.payload {
                            Some(p) => Some(recur(p)?),
                            None => None,
                        },
                    })
                })
                .collect::<Option<_>>()?;
            Some(WitnessPattern::Union(out))
        }
        TypeKind::Function { from, to } => Some(WitnessPattern::Function(
            Box::new(recur(from)?),
            Box::new(recur(to)?),
        )),
        _ => None,
    }
}
