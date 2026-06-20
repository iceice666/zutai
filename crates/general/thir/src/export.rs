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

use std::collections::{HashMap, HashSet};

use zutai_hir::BindingId;

use crate::import::{ImportedField, ImportedTupleItem, ImportedType, ImportedUnionVariant};
use crate::ir::{RowTail, ThirDeclKind, ThirFile, TypeId, TypeKind, TypeTupleItem};

/// A type that cannot be exported across a module import in this phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportUnsupported {
    pub reason: &'static str,
}

/// Convert `ty` (a type in `file`'s arena) into a neutral [`ImportedType`].
pub fn export_type(file: &ThirFile, ty: TypeId) -> Result<ImportedType, ExportUnsupported> {
    let aliases = build_alias_map(file);
    let mut seen = HashSet::new();
    export(file, &aliases, ty, &mut seen)
}

fn build_alias_map(file: &ThirFile) -> HashMap<BindingId, TypeId> {
    let mut map = HashMap::new();
    for (_, decl) in file.decl_arena.iter() {
        if let ThirDeclKind::TypeAlias { ty, .. } = decl.kind {
            map.insert(decl.binding, ty);
        }
    }
    map
}

fn export(
    file: &ThirFile,
    aliases: &HashMap<BindingId, TypeId>,
    ty: TypeId,
    seen: &mut HashSet<BindingId>,
) -> Result<ImportedType, ExportUnsupported> {
    match file.type_arena[ty.0 as usize].kind.clone() {
        // Imported data is just a value, so a singleton `true`/`false` type is
        // exported as the wider `Bool`.
        TypeKind::Bool | TypeKind::True | TypeKind::False => Ok(ImportedType::Bool),
        TypeKind::Int => Ok(ImportedType::Int),
        TypeKind::Float => Ok(ImportedType::Float),
        TypeKind::Text => Ok(ImportedType::Text),
        TypeKind::Atom(name) => Ok(ImportedType::Atom(name)),
        TypeKind::List(inner) => Ok(ImportedType::List(Box::new(export(
            file, aliases, inner, seen,
        )?))),
        TypeKind::Optional(inner) => Ok(ImportedType::Optional(Box::new(export(
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
            let result = match aliases.get(&binding).copied() {
                Some(target) => export(file, aliases, target, seen),
                None => Ok(ImportedType::Unknown),
            };
            seen.remove(&binding);
            result
        }
        // Free inference / type variables represent unconstrained positions; the
        // importer re-introduces a fresh variable for them.
        TypeKind::InferVar(_) | TypeKind::TypeVar(_) => Ok(ImportedType::Unknown),
        // Parametric constructors cannot cross module boundaries unapplied; treat
        // as unknown at the import boundary (the same fallback as TypeVar).
        TypeKind::AliasApply { .. } | TypeKind::Apply { .. } | TypeKind::Con(_) => {
            Ok(ImportedType::Unknown)
        }
        TypeKind::Function { from, to } => Ok(ImportedType::Function {
            from: Box::new(export(file, aliases, from, seen)?),
            to: Box::new(export(file, aliases, to, seen)?),
        }),
        // `Type` has no payload — the denotation is recovered separately by
        // the semantic layer from the module's final-expression value (the
        // `ThirExprKind::TypeValue(tid)` for that field).  Here we emit a bare
        // `ImportedType::Type(Unknown)` as a placeholder; `resolve_zt` in the
        // semantic crate overwrites it with the real denotation after walking
        // the final expression.
        TypeKind::Type => Ok(ImportedType::Type(Box::new(ImportedType::Unknown))),
        TypeKind::Error => Err(ExportUnsupported {
            reason: "imported module has an unresolved type",
        }),
    }
}
