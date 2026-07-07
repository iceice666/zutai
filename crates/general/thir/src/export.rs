//! Export a completed module's type for use by an importer.
//!
//! A `.zt` module import is typed by its final expression's type.  That type
//! lives in the imported module's own arena and may reference type aliases
//! defined there, so it cannot be handed to a different module's lowerer
//! directly.  [`export_type`] walks it into a neutral [`ImportedType`]
//! descriptor (alias-expanded, arena-independent) that the importer interns
//! into its own arena.
//!
//! Functions and type-values cross module boundaries using home-module handles
//! stamped by the evaluator (for closures) and denotation descriptors embedded
//! in `ImportedType::Type` (for type aliases).  Both are now representable.

use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;

use crate::import::{
    ImportedEffectOp, ImportedField, ImportedRowTail, ImportedTupleItem, ImportedType,
    ImportedUnionVariant,
};
use crate::ir::{EffectRow, RowTail, ThirDeclKind, ThirFile, TypeId, TypeKind, TypeTupleItem};

/// A type that cannot be exported across a module import in this phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportUnsupported {
    pub reason: &'static str,
}

/// High bit tagging an `ImportedType::TyVar` id derived from an inference
/// variable, keeping it disjoint from ids derived from named type-variable
/// bindings (which are masked to clear this bit).
const TYVAR_INFER_TAG: u32 = 1 << 31;

/// Convert `ty` (a type in `file`'s arena) into a neutral [`ImportedType`].
pub fn export_type(file: &ThirFile, ty: TypeId) -> Result<ImportedType, ExportUnsupported> {
    let aliases = build_alias_map(file);
    if has_opaque_data_leaf(file, &aliases, ty, false, &mut FxHashSet::default()) {
        return Err(ExportUnsupported {
            reason: "opaque host handles cannot cross a module boundary",
        });
    }
    let mut seen = FxHashSet::default();
    export(file, &aliases, ty, &mut seen)
}

/// Export a type-value's denotation, preserving a parametric constructor's
/// binder so it can be applied on the import side (`s.Stream Int`).
///
/// When `tid` denotes a *parametric* type alias (one with type parameters), the
/// result is an [`ImportedType::TypeCon`] carrying the parameter list and the
/// alias body (with the recursive self-reference kept bounded as
/// [`ImportedType::ConApply`]). Otherwise it falls back to [`export_type`], so
/// non-parametric type aliases (`serverLib.Server`) export exactly as before.
pub fn export_type_value(file: &ThirFile, tid: TypeId) -> Result<ImportedType, ExportUnsupported> {
    let aliases = build_alias_map(file);
    // Follow a bare-alias chain (`Stream = Stream` lowers to `Alias(Stream)`) to
    // the parametric constructor it names, without unfolding the body.
    let mut head = tid;
    let mut guard = FxHashSet::default();
    while let TypeKind::Alias(binding) = file.type_arena[head.0 as usize].kind {
        if let Some(params) = aliases.params.get(&binding)
            && !params.is_empty()
        {
            let body_ty = aliases.bodies[&binding];
            // Refuse higher-kinded constructor parameters: the descriptor carries
            // only ground type parameters in this phase. HIR does not retain an
            // alias param's kind annotation, so detect higher-kindedness by its
            // use — a param applied as a constructor (`F A`) lowers to an `Apply`
            // whose spine head is `TypeVar(param)`.
            let param_set: FxHashSet<BindingId> = params.iter().copied().collect();
            let mut visited = FxHashSet::default();
            if body_applies_param(file, body_ty, &param_set, &mut visited) {
                return Err(ExportUnsupported {
                    reason: "higher-kinded imported type-constructor parameter",
                });
            }
            let mut seen = FxHashSet::default();
            seen.insert(binding);
            let mut subst = FxHashMap::default();
            for param in params {
                subst.insert(*param, ImportedType::TyVar(param.0 & !TYVAR_INFER_TAG));
            }
            let body = export_typecon_body(file, &aliases, body_ty, &mut seen, &subst)?;
            let param_ids = params
                .iter()
                .map(|p| p.0 & !TYVAR_INFER_TAG)
                .collect::<Vec<_>>();
            return Ok(ImportedType::TypeCon {
                params: param_ids,
                body: Box::new(body),
            });
        }
        if !guard.insert(binding) {
            break;
        }
        match aliases.bodies.get(&binding).copied() {
            Some(next) => head = next,
            None => break,
        }
    }
    export_type(file, tid)
}

/// Export a top-level type alias binding as a type-value denotation without
/// requiring the module's final runtime value to contain a `TypeValue` field.
pub fn export_type_alias_value(
    file: &ThirFile,
    binding: BindingId,
) -> Result<ImportedType, ExportUnsupported> {
    let aliases = build_alias_map(file);
    export_alias_binding_value(file, &aliases, binding)
}

fn export_alias_binding_value(
    file: &ThirFile,
    aliases: &AliasMap,
    binding: BindingId,
) -> Result<ImportedType, ExportUnsupported> {
    if let Some(params) = aliases.params.get(&binding)
        && !params.is_empty()
    {
        return export_type_constructor(file, aliases, binding, params);
    }
    let Some(body) = aliases.bodies.get(&binding).copied() else {
        return Err(ExportUnsupported {
            reason: "type alias binding has no body",
        });
    };
    export(file, aliases, body, &mut FxHashSet::default())
}

fn export_type_constructor(
    file: &ThirFile,
    aliases: &AliasMap,
    binding: BindingId,
    params: &[BindingId],
) -> Result<ImportedType, ExportUnsupported> {
    let body_ty = aliases.bodies[&binding];
    // Refuse higher-kinded constructor parameters: the descriptor carries only
    // ground type parameters in this phase.
    let param_set: FxHashSet<BindingId> = params.iter().copied().collect();
    let mut visited = FxHashSet::default();
    if body_applies_param(file, body_ty, &param_set, &mut visited) {
        return Err(ExportUnsupported {
            reason: "higher-kinded imported type-constructor parameter",
        });
    }
    let mut seen = FxHashSet::default();
    seen.insert(binding);
    let mut subst = FxHashMap::default();
    for param in params {
        subst.insert(*param, ImportedType::TyVar(param.0 & !TYVAR_INFER_TAG));
    }
    let body = export_typecon_body(file, aliases, body_ty, &mut seen, &subst)?;
    let param_ids = params
        .iter()
        .map(|p| p.0 & !TYVAR_INFER_TAG)
        .collect::<Vec<_>>();
    Ok(ImportedType::TypeCon {
        params: param_ids,
        body: Box::new(body),
    })
}

fn export_typecon_body(
    file: &ThirFile,
    aliases: &AliasMap,
    ty: TypeId,
    seen: &mut FxHashSet<BindingId>,
    subst: &FxHashMap<BindingId, ImportedType>,
) -> Result<ImportedType, ExportUnsupported> {
    match file.type_arena[ty.0 as usize].kind.clone() {
        TypeKind::TypeVar(binding) => subst
            .get(&binding)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| export(file, aliases, ty, &mut FxHashSet::default())),
        TypeKind::AliasApply { binding, args } if aliases.params.contains_key(&binding) => {
            let exported_args = args
                .iter()
                .map(|arg| export_typecon_body(file, aliases, *arg, seen, subst))
                .collect::<Result<Vec<_>, _>>()?;
            if seen.contains(&binding) {
                let ctor = file.binding_names[binding.0 as usize].clone();
                return Ok(ImportedType::ConApply {
                    ctor,
                    args: exported_args,
                });
            }
            let Some(params) = aliases.params.get(&binding) else {
                return Ok(ImportedType::Unknown);
            };
            let Some(body) = aliases.bodies.get(&binding).copied() else {
                return Ok(ImportedType::Unknown);
            };
            let mut nested_subst = subst.clone();
            for (param, arg) in params.iter().copied().zip(exported_args) {
                nested_subst.insert(param, arg);
            }
            seen.insert(binding);
            let result = export_typecon_body(file, aliases, body, seen, &nested_subst);
            seen.remove(&binding);
            result
        }
        TypeKind::Alias(binding) => {
            if !seen.insert(binding) {
                return Ok(ImportedType::Unknown);
            }
            let result = aliases
                .bodies
                .get(&binding)
                .copied()
                .map(|body| export_typecon_body(file, aliases, body, seen, subst))
                .unwrap_or_else(|| Ok(ImportedType::Unknown));
            seen.remove(&binding);
            result
        }
        TypeKind::Bool | TypeKind::True | TypeKind::False => Ok(ImportedType::Bool),
        TypeKind::Int => Ok(ImportedType::Int),
        TypeKind::Float => Ok(ImportedType::Float),
        TypeKind::FixedNum(fw) => Ok(ImportedType::FixedNum(fw)),
        TypeKind::Text => Ok(ImportedType::Text),
        TypeKind::Posit(spec) => Ok(ImportedType::Posit(spec)),
        TypeKind::Opaque(name) => Ok(ImportedType::Opaque(name)),
        TypeKind::Atom(name) => Ok(ImportedType::Atom(name)),
        TypeKind::List(inner) => Ok(ImportedType::List(Box::new(export_typecon_body(
            file, aliases, inner, seen, subst,
        )?))),
        TypeKind::Optional(inner) => Ok(ImportedType::Optional(Box::new(export_typecon_body(
            file, aliases, inner, seen, subst,
        )?))),
        TypeKind::Maybe(inner) => Ok(ImportedType::Maybe(Box::new(export_typecon_body(
            file, aliases, inner, seen, subst,
        )?))),
        TypeKind::Record(fields, tail) => {
            if tail != RowTail::Closed {
                return Err(ExportUnsupported {
                    reason: "cannot export an open record type across a module boundary",
                });
            }
            fields
                .iter()
                .map(|field| {
                    Ok(ImportedField {
                        name: field.name.clone(),
                        optional: field.optional,
                        ty: export_typecon_body(file, aliases, field.ty, seen, subst)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ImportedType::Record)
        }
        TypeKind::Tuple(items) => items
            .iter()
            .map(|item| match item {
                TypeTupleItem::Named { name, ty, .. } => Ok(ImportedTupleItem::Named {
                    name: name.clone(),
                    ty: export_typecon_body(file, aliases, *ty, seen, subst)?,
                }),
                TypeTupleItem::Positional(ty) => Ok(ImportedTupleItem::Positional(
                    export_typecon_body(file, aliases, *ty, seen, subst)?,
                )),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(ImportedType::Tuple),
        TypeKind::Union(variants, tail) => {
            if tail != RowTail::Closed {
                return Err(ExportUnsupported {
                    reason: "cannot export an open union type across a module boundary",
                });
            }
            variants
                .iter()
                .map(|variant| {
                    Ok(crate::import::ImportedUnionVariant {
                        name: variant.name.clone(),
                        payload: variant
                            .payload
                            .map(|payload| {
                                export_typecon_body(file, aliases, payload, seen, subst)
                                    .map(Box::new)
                            })
                            .transpose()?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(ImportedType::Union)
        }
        TypeKind::InferVar(v) => Ok(ImportedType::TyVar(v | TYVAR_INFER_TAG)),
        TypeKind::ForAll { body, .. } => export_typecon_body(file, aliases, body, seen, subst),
        TypeKind::Function { from, to } => Ok(ImportedType::Function {
            from: Box::new(export_typecon_body(file, aliases, from, seen, subst)?),
            to: Box::new(export_typecon_body(file, aliases, to, seen, subst)?),
        }),
        TypeKind::Effect { base, row } => Ok(ImportedType::Effect {
            base: Box::new(export_typecon_body(file, aliases, base, seen, subst)?),
            ops: export_effect_row_typecon_body(file, aliases, &row, seen, subst)?,
            tail: export_row_tail(row.tail)?,
        }),
        TypeKind::Type(_) => Ok(ImportedType::Type(Box::new(ImportedType::Unknown))),
        TypeKind::AliasApply { .. }
        | TypeKind::Apply { .. }
        | TypeKind::Con(_)
        | TypeKind::Patch { .. } => Ok(ImportedType::Unknown),
        TypeKind::Never => Err(ExportUnsupported {
            reason: "never types cannot cross a module boundary in this phase",
        }),
        TypeKind::Error => Err(ExportUnsupported {
            reason: "imported module has an unresolved type",
        }),
    }
}

/// Whether `ty` uses any binding in `params` in type-constructor position — the
/// signal that the parameter is higher-kinded (`F A`). Walks the type graph with
/// a `visited` set so recursive bodies (`tail: Lst A`) terminate.
fn body_applies_param(
    file: &ThirFile,
    ty: TypeId,
    params: &FxHashSet<BindingId>,
    visited: &mut FxHashSet<TypeId>,
) -> bool {
    if !visited.insert(ty) {
        return false;
    }
    match file.type_arena[ty.0 as usize].kind.clone() {
        TypeKind::Apply { func, arg } => {
            // The spine head being a param means the param is applied.
            let mut head = func;
            while let TypeKind::Apply { func: f, .. } = file.type_arena[head.0 as usize].kind {
                head = f;
            }
            if let TypeKind::TypeVar(b) = file.type_arena[head.0 as usize].kind
                && params.contains(&b)
            {
                return true;
            }
            body_applies_param(file, func, params, visited)
                || body_applies_param(file, arg, params, visited)
        }
        TypeKind::AliasApply { args, .. } => args
            .iter()
            .any(|a| body_applies_param(file, *a, params, visited)),
        TypeKind::List(inner)
        | TypeKind::Optional(inner)
        | TypeKind::Maybe(inner)
        | TypeKind::Patch { target: inner, .. } => body_applies_param(file, inner, params, visited),
        TypeKind::Function { from, to } => {
            body_applies_param(file, from, params, visited)
                || body_applies_param(file, to, params, visited)
        }
        TypeKind::Effect { base, row } => {
            body_applies_param(file, base, params, visited)
                || row.ops.iter().any(|op| {
                    body_applies_param(file, op.param, params, visited)
                        || body_applies_param(file, op.result, params, visited)
                })
        }
        TypeKind::Record(fields, _) => fields
            .iter()
            .any(|f| body_applies_param(file, f.ty, params, visited)),
        TypeKind::Tuple(items) => items.iter().any(|item| {
            let inner = match item {
                TypeTupleItem::Named { ty, .. } => *ty,
                TypeTupleItem::Positional(ty) => *ty,
            };
            body_applies_param(file, inner, params, visited)
        }),
        TypeKind::Union(variants, _) => variants.iter().any(|v| {
            v.payload
                .is_some_and(|p| body_applies_param(file, p, params, visited))
        }),
        TypeKind::ForAll { body, .. } => body_applies_param(file, body, params, visited),
        _ => false,
    }
}

/// Alias bodies and parameter lists for a module, gathered from its
/// `TypeAlias` declarations.
struct AliasMap {
    bodies: FxHashMap<BindingId, TypeId>,
    params: FxHashMap<BindingId, Vec<BindingId>>,
}

fn build_alias_map(file: &ThirFile) -> AliasMap {
    let mut bodies = FxHashMap::default();
    let mut params = FxHashMap::default();
    for (_, decl) in file.decl_arena.iter() {
        if let ThirDeclKind::TypeAlias { ty, params: ps } = &decl.kind {
            bodies.insert(decl.binding, *ty);
            if !ps.is_empty() {
                params.insert(decl.binding, ps.clone());
            }
        }
    }
    AliasMap { bodies, params }
}

fn export_effect_row_typecon_body(
    file: &ThirFile,
    aliases: &AliasMap,
    row: &EffectRow,
    seen: &mut FxHashSet<BindingId>,
    subst: &FxHashMap<BindingId, ImportedType>,
) -> Result<Vec<ImportedEffectOp>, ExportUnsupported> {
    row.ops
        .iter()
        .map(|op| {
            Ok(ImportedEffectOp {
                name: op.name.clone(),
                param: export_typecon_body(file, aliases, op.param, seen, subst)?,
                result: export_typecon_body(file, aliases, op.result, seen, subst)?,
            })
        })
        .collect()
}

fn export_effect_row(
    file: &ThirFile,
    aliases: &AliasMap,
    row: &EffectRow,
    seen: &mut FxHashSet<BindingId>,
) -> Result<Vec<ImportedEffectOp>, ExportUnsupported> {
    row.ops
        .iter()
        .map(|op| {
            Ok(ImportedEffectOp {
                name: op.name.clone(),
                param: export(file, aliases, op.param, seen)?,
                result: export(file, aliases, op.result, seen)?,
            })
        })
        .collect()
}

fn export_row_tail(tail: RowTail) -> Result<ImportedRowTail, ExportUnsupported> {
    match tail {
        RowTail::Closed => Ok(ImportedRowTail::Closed),
        RowTail::Open => Ok(ImportedRowTail::Open),
        RowTail::Param(binding) => Ok(ImportedRowTail::Param(binding.0 & !TYVAR_INFER_TAG)),
        RowTail::Infer(_) => Err(ExportUnsupported {
            reason: "unresolved row variable in exported type",
        }),
    }
}

fn has_opaque_data_leaf(
    file: &ThirFile,
    aliases: &AliasMap,
    ty: TypeId,
    under_function: bool,
    seen: &mut FxHashSet<TypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    match file.type_arena[ty.0 as usize].kind.clone() {
        TypeKind::Opaque(_) => !under_function,
        TypeKind::Alias(binding) => aliases
            .bodies
            .get(&binding)
            .is_some_and(|body| has_opaque_data_leaf(file, aliases, *body, under_function, seen)),
        TypeKind::AliasApply { binding, args } => {
            if let (Some(params), Some(body)) =
                (aliases.params.get(&binding), aliases.bodies.get(&binding))
            {
                let mut subst = FxHashMap::default();
                for (param, arg) in params.iter().copied().zip(args) {
                    subst.insert(param, arg);
                }
                has_opaque_data_leaf_subst(file, aliases, *body, under_function, seen, &subst)
            } else {
                args.into_iter()
                    .any(|arg| has_opaque_data_leaf(file, aliases, arg, under_function, seen))
            }
        }
        TypeKind::List(inner)
        | TypeKind::Optional(inner)
        | TypeKind::Maybe(inner)
        | TypeKind::Patch { target: inner, .. } => {
            has_opaque_data_leaf(file, aliases, inner, under_function, seen)
        }
        TypeKind::Record(fields, _) => fields
            .iter()
            .any(|field| has_opaque_data_leaf(file, aliases, field.ty, under_function, seen)),
        TypeKind::Tuple(items) => items.iter().any(|item| {
            let inner = match item {
                TypeTupleItem::Named { ty, .. } | TypeTupleItem::Positional(ty) => *ty,
            };
            has_opaque_data_leaf(file, aliases, inner, under_function, seen)
        }),
        TypeKind::Union(variants, _) => variants.iter().any(|variant| {
            variant.payload.is_some_and(|payload| {
                has_opaque_data_leaf(file, aliases, payload, under_function, seen)
            })
        }),
        TypeKind::Function { from, to } => {
            has_opaque_data_leaf(file, aliases, from, true, seen)
                || has_opaque_data_leaf(file, aliases, to, true, seen)
        }
        TypeKind::Effect { base, row } => {
            has_opaque_data_leaf(file, aliases, base, under_function, seen)
                || row.ops.iter().any(|op| {
                    has_opaque_data_leaf(file, aliases, op.param, true, seen)
                        || has_opaque_data_leaf(file, aliases, op.result, true, seen)
                })
        }
        TypeKind::ForAll { body, .. } => {
            has_opaque_data_leaf(file, aliases, body, under_function, seen)
        }
        _ => false,
    }
}

fn has_opaque_data_leaf_subst(
    file: &ThirFile,
    aliases: &AliasMap,
    ty: TypeId,
    under_function: bool,
    seen: &mut FxHashSet<TypeId>,
    subst: &FxHashMap<BindingId, TypeId>,
) -> bool {
    match file.type_arena[ty.0 as usize].kind.clone() {
        TypeKind::TypeVar(binding) => subst
            .get(&binding)
            .is_some_and(|arg| has_opaque_data_leaf(file, aliases, *arg, under_function, seen)),
        _ => has_opaque_data_leaf(file, aliases, ty, under_function, seen),
    }
}

fn export(
    file: &ThirFile,
    aliases: &AliasMap,
    ty: TypeId,
    seen: &mut FxHashSet<BindingId>,
) -> Result<ImportedType, ExportUnsupported> {
    match file.type_arena[ty.0 as usize].kind.clone() {
        // Imported data is just a value, so a singleton `true`/`false` type is
        // exported as the wider `Bool`.
        TypeKind::Bool | TypeKind::True | TypeKind::False => Ok(ImportedType::Bool),
        TypeKind::Int => Ok(ImportedType::Int),
        TypeKind::Float => Ok(ImportedType::Float),
        TypeKind::FixedNum(fw) => Ok(ImportedType::FixedNum(fw)),
        TypeKind::Posit(spec) => Ok(ImportedType::Posit(spec)),
        TypeKind::Text => Ok(ImportedType::Text),
        TypeKind::Opaque(name) => Ok(ImportedType::Opaque(name)),
        TypeKind::Atom(name) => Ok(ImportedType::Atom(name)),
        TypeKind::List(inner) => Ok(ImportedType::List(Box::new(export(
            file, aliases, inner, seen,
        )?))),
        TypeKind::Optional(inner) => Ok(ImportedType::Optional(Box::new(export(
            file, aliases, inner, seen,
        )?))),
        TypeKind::Maybe(inner) => Ok(ImportedType::Maybe(Box::new(export(
            file, aliases, inner, seen,
        )?))),
        TypeKind::Record(fields, tail) => {
            if tail != RowTail::Closed {
                return Err(ExportUnsupported {
                    reason: "cannot export an open record type across a module boundary",
                });
            }
            let mut out = Vec::with_capacity(fields.len());
            for field in &fields {
                out.push(ImportedField {
                    name: field.name.clone(),
                    optional: field.optional,
                    ty: export(file, aliases, field.ty, seen)?,
                });
            }
            Ok(ImportedType::Record(out))
        }
        TypeKind::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in &items {
                out.push(match item {
                    TypeTupleItem::Named { name, ty, .. } => ImportedTupleItem::Named {
                        name: name.clone(),
                        ty: export(file, aliases, *ty, seen)?,
                    },
                    TypeTupleItem::Positional(ty) => {
                        ImportedTupleItem::Positional(export(file, aliases, *ty, seen)?)
                    }
                });
            }
            Ok(ImportedType::Tuple(out))
        }
        TypeKind::Union(variants, tail) => {
            if tail != RowTail::Closed {
                return Err(ExportUnsupported {
                    reason: "cannot export an open union type across a module boundary",
                });
            }
            let mut out = Vec::with_capacity(variants.len());
            for variant in &variants {
                let payload = match variant.payload {
                    Some(ty) => Some(Box::new(export(file, aliases, ty, seen)?)),
                    None => None,
                };
                out.push(ImportedUnionVariant {
                    name: variant.name.clone(),
                    payload,
                });
            }
            Ok(ImportedType::Union(out))
        }
        TypeKind::Alias(binding) => {
            if !seen.insert(binding) {
                // Cyclic alias — cannot happen in completed THIR (it would be a
                // diagnostic), but stay total rather than recurse forever.
                return Ok(ImportedType::Unknown);
            }
            let result = match aliases.bodies.get(&binding).copied() {
                Some(target) => export(file, aliases, target, seen),
                None => Ok(ImportedType::Unknown),
            };
            seen.remove(&binding);
            result
        }
        // In an exported value's type, a free type/inference variable is a
        // generalizable (Hindley–Milner) type parameter — the value is polymorphic
        // in it — so export it as a `TyVar`. Repeated occurrences of the same var
        // share an id (the importer maps each id to one fresh inference variable,
        // preserving constraints like `A = A`, and quantifies the binding so each
        // use instantiates fresh). The two id spaces are kept disjoint by tagging
        // inference-variable ids with the high bit.
        TypeKind::TypeVar(binding) => Ok(ImportedType::TyVar(binding.0 & !TYVAR_INFER_TAG)),
        TypeKind::InferVar(v) => Ok(ImportedType::TyVar(v | TYVAR_INFER_TAG)),
        // An explicit quantifier just exports its body; the body's parameters are
        // free type variables there and are generalized by the arm above.
        TypeKind::ForAll { body, .. } => export(file, aliases, body, seen),
        // A saturated application of a parametric alias (`Stream A`) exports as a
        // bounded reference to that constructor — never unfolded, so a recursive
        // body (`tail: Stream A`) terminates. The importer resolves the `ctor`
        // name against the constructors exported by the same module.
        TypeKind::AliasApply { binding, args } if aliases.params.contains_key(&binding) => {
            let ctor = file.binding_names[binding.0 as usize].clone();
            let mut exported = Vec::with_capacity(args.len());
            for arg in &args {
                exported.push(export(file, aliases, *arg, seen)?);
            }
            Ok(ImportedType::ConApply {
                ctor,
                args: exported,
            })
        }
        // Other parametric constructors, curried applications, and patch markers
        // do not cross module boundaries as concrete exported data in this phase.
        TypeKind::AliasApply { .. }
        | TypeKind::Apply { .. }
        | TypeKind::Con(_)
        | TypeKind::Patch { .. } => Ok(ImportedType::Unknown),
        TypeKind::Function { from, to } => Ok(ImportedType::Function {
            from: Box::new(export(file, aliases, from, seen)?),
            to: Box::new(export(file, aliases, to, seen)?),
        }),
        TypeKind::Effect { base, row } => Ok(ImportedType::Effect {
            base: Box::new(export(file, aliases, base, seen)?),
            ops: export_effect_row(file, aliases, &row, seen)?,
            tail: export_row_tail(row.tail)?,
        }),
        // `Type` has no payload — the denotation is recovered separately by
        // the semantic layer from the module's final-expression value (the
        // `ThirExprKind::TypeValue(tid)` for that field).  Here we emit a bare
        // `ImportedType::Type(Unknown)` as a placeholder; `resolve_zt` in the
        // semantic crate overwrites it with the real denotation after walking
        // the final expression.
        TypeKind::Type(_) => Ok(ImportedType::Type(Box::new(ImportedType::Unknown))),
        TypeKind::Never => Err(ExportUnsupported {
            reason: "never types cannot cross a module boundary in this phase",
        }),
        TypeKind::Error => Err(ExportUnsupported {
            reason: "imported module has an unresolved type",
        }),
    }
}
