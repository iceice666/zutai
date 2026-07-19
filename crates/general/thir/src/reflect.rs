//! Shared compile-time `schema` reflection over THIR type structure.
//!
//! `schema T` is fully determined by the THIR type structure of `T`: the
//! computation walks records, unions, and (generic) alias expansions and
//! produces first-order serializable data. Both the THIR evaluator oracle
//! (`zutai-eval`) and the THIR→TLC reflection fold call this single
//! implementation, so a schema value folded into TLC is equal to the oracle's
//! runtime value by construction — never a reimplementation that could drift.

use std::rc::Rc;

use zutai_hir::BindingId;

use crate::ir::{
    RowTail, ThirDeclKind, ThirFile, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
};

/// A type reference into a specific module's THIR arena, carrying the alias
/// substitutions accumulated while expanding generic aliases. Mirrors the
/// evaluator's `RuntimeType` so both callers describe types the same way.
#[derive(Clone, Debug)]
pub struct ReflectedType {
    pub module: usize,
    pub ty: TypeId,
    pub subst: Rc<[(BindingId, ReflectedType)]>,
}

impl ReflectedType {
    pub fn new(module: usize, ty: TypeId) -> Self {
        Self {
            module,
            ty,
            subst: Rc::from([]),
        }
    }

    pub fn with_subst(module: usize, ty: TypeId, subst: Rc<[(BindingId, ReflectedType)]>) -> Self {
        Self { module, ty, subst }
    }

    pub fn with_ty(&self, ty: TypeId) -> Self {
        Self {
            module: self.module,
            ty,
            subst: self.subst.clone(),
        }
    }
}

/// Why a type could not be reflected. `Unsupported` carries the user-facing
/// reason (same wording as the evaluator's refusals); `Internal` marks an
/// arena/registry inconsistency.
#[derive(Debug)]
pub enum ReflectError {
    Unsupported(String),
    Internal(&'static str),
}

/// First-order serializable data produced by `schema` — the exact value shape
/// the oracle builds, without committing to any evaluator's value type.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaData {
    Record(Vec<(&'static str, SchemaData)>),
    List(Vec<SchemaData>),
    Text(String),
    Bool(bool),
    Atom(&'static str),
}

/// A record field surfaced by reflection, with its (still symbolic) type.
pub struct ReflectedField {
    pub name: String,
    pub optional: bool,
    pub ty: ReflectedType,
}

/// A union variant surfaced by reflection.
pub struct ReflectedVariant {
    pub name: String,
    pub payload: Option<ReflectedType>,
}

/// The alias-expanded structural view `fields`/`variants`/`schema` dispatch on.
/// Non-record, non-union types collapse to `Other` — every reflection entry
/// point rejects them with its own message.
pub enum ReflectedView {
    Record(Vec<ReflectedField>, RowTail),
    Union(Vec<ReflectedVariant>, RowTail),
    Other,
}

/// Expand aliases and return the structural view of `ty` for reflection.
pub fn reflected_view(
    files: &[&ThirFile],
    ty: &ReflectedType,
) -> Result<ReflectedView, ReflectError> {
    Ok(match type_view(files, ty, 0, false)? {
        TypeView::Record(fields, tail) => ReflectedView::Record(
            fields
                .into_iter()
                .map(|field| ReflectedField {
                    name: field.name,
                    optional: field.optional,
                    ty: field.ty,
                })
                .collect(),
            tail,
        ),
        TypeView::Union(variants, tail) => ReflectedView::Union(
            variants
                .into_iter()
                .map(|variant| ReflectedVariant {
                    name: variant.name,
                    payload: variant.payload,
                })
                .collect(),
            tail,
        ),
        _ => ReflectedView::Other,
    })
}

/// The open-row refusal every reflection entry point shares.
pub fn open_row_error(kind: &str) -> ReflectError {
    open_row_reflection_error(kind)
}

/// Compute `schema` for `ty`. `files[module]` resolves a `ReflectedType`'s
/// module index to its THIR file (single-module callers pass `&[file]`).
pub fn schema_data(files: &[&ThirFile], ty: &ReflectedType) -> Result<SchemaData, ReflectError> {
    match type_view(files, ty, 0, false)? {
        TypeView::Record(fields, RowTail::Closed) => Ok(SchemaData::Record(vec![
            ("kind", SchemaData::Atom("record")),
            ("fields", schema_fields(files, fields)?),
        ])),
        TypeView::Record(_, _) => Err(open_row_reflection_error("record")),
        TypeView::Union(variants, RowTail::Closed) => {
            let variants = variants
                .into_iter()
                .map(|variant| schema_variant(files, variant))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SchemaData::Record(vec![
                ("kind", SchemaData::Atom("union")),
                ("variants", SchemaData::List(variants)),
            ]))
        }
        TypeView::Union(_, _) => Err(open_row_reflection_error("union")),
        _ => Err(ReflectError::Unsupported(
            "`schema` expects a record or union type".to_string(),
        )),
    }
}

fn schema_fields(
    files: &[&ThirFile],
    fields: Vec<ReflectedRecordField>,
) -> Result<SchemaData, ReflectError> {
    fields
        .into_iter()
        .map(|field| schema_field(files, field))
        .collect::<Result<Vec<_>, _>>()
        .map(SchemaData::List)
}

fn schema_field(
    files: &[&ThirFile],
    field: ReflectedRecordField,
) -> Result<SchemaData, ReflectError> {
    Ok(SchemaData::Record(vec![
        ("name", SchemaData::Text(field.name)),
        ("type", SchemaData::Text(type_label(files, &field.ty)?)),
        ("optional", SchemaData::Bool(field.optional)),
    ]))
}

fn schema_variant(
    files: &[&ThirFile],
    variant: ReflectedUnionVariant,
) -> Result<SchemaData, ReflectError> {
    let fields = match variant.payload {
        Some(payload) => match type_view(files, &payload, 0, false)? {
            TypeView::Record(fields, RowTail::Closed) => schema_fields(files, fields)?,
            TypeView::Record(_, _) => return Err(open_row_reflection_error("record")),
            _ => {
                return Err(ReflectError::Unsupported(
                    "union variant payload reflection expects a record payload".to_string(),
                ));
            }
        },
        None => SchemaData::List(Vec::new()),
    };
    Ok(SchemaData::Record(vec![
        ("name", SchemaData::Text(variant.name)),
        ("fields", fields),
    ]))
}

fn type_label(files: &[&ThirFile], ty: &ReflectedType) -> Result<String, ReflectError> {
    match type_view(files, ty, 0, true)? {
        TypeView::Alias { name, args } => {
            if args.is_empty() {
                return Ok(name);
            }
            let args = args
                .into_iter()
                .map(|arg| type_label(files, &arg))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("{}<{}>", name, args.join(", ")))
        }
        TypeView::Type => Ok("Type".to_string()),
        TypeView::Bool => Ok("Bool".to_string()),
        TypeView::Text => Ok("Text".to_string()),
        TypeView::Int => Ok("Int".to_string()),
        TypeView::Float => Ok("Float".to_string()),
        TypeView::FixedNum(fw) => Ok(fw.name().to_string()),
        TypeView::Posit(spec) => Ok(spec.type_name()),
        TypeView::Atom(name) => Ok(format!("#{name}")),
        TypeView::True => Ok("true".to_string()),
        TypeView::False => Ok("false".to_string()),
        TypeView::Never => Ok("Never".to_string()),
        TypeView::List(inner) => Ok(format!("[{}]", type_label(files, &inner)?)),
        TypeView::Optional(inner) => Ok(format!("{}?", type_label(files, &inner)?)),
        TypeView::Maybe(inner) => Ok(format!("Maybe {}", type_label(files, &inner)?)),
        TypeView::Record(_, RowTail::Closed) => Ok("record".to_string()),
        TypeView::Record(_, _) => Err(open_row_reflection_error("record")),
        TypeView::Opaque(name) => Ok(name),
        TypeView::Union(_, RowTail::Closed) => Ok("union".to_string()),
        TypeView::Union(_, _) => Err(open_row_reflection_error("union")),
        TypeView::Tuple(items) => {
            let parts = items
                .into_iter()
                .map(|item| match item {
                    ReflectedTupleItem::Named { name, ty } => {
                        Ok(format!("{name}: {}", type_label(files, &ty)?))
                    }
                    ReflectedTupleItem::Positional(ty) => type_label(files, &ty),
                })
                .collect::<Result<Vec<_>, ReflectError>>()?;
            Ok(format!("({})", parts.join(", ")))
        }
        TypeView::Function { from, to } => Ok(format!(
            "{} -> {}",
            type_label(files, &from)?,
            type_label(files, &to)?
        )),
        TypeView::Effect { base } => Ok(format!("{} ! effect", type_label(files, &base)?)),
    }
}

fn type_view(
    files: &[&ThirFile],
    ty: &ReflectedType,
    depth: u16,
    preserve_named_aliases: bool,
) -> Result<TypeView, ReflectError> {
    if depth > 256 {
        return Err(ReflectError::Unsupported(
            "type alias expansion exceeded reflection fuel".to_string(),
        ));
    }
    let file = file_for_module(files, ty.module)?;
    let Some(type_node) = file.type_arena.get(ty.ty.0 as usize) else {
        return Err(ReflectError::Internal(
            "type value points outside its module arena",
        ));
    };
    match type_node.kind.clone() {
        TypeKind::Type(_) => Ok(TypeView::Type),
        TypeKind::Bool => Ok(TypeView::Bool),
        TypeKind::Text => Ok(TypeView::Text),
        TypeKind::Int => Ok(TypeView::Int),
        TypeKind::Float => Ok(TypeView::Float),
        TypeKind::FixedNum(fw) => Ok(TypeView::FixedNum(fw)),
        TypeKind::Posit(spec) => Ok(TypeView::Posit(spec)),
        TypeKind::Opaque(name) => Ok(TypeView::Opaque(name)),
        TypeKind::Atom(name) => Ok(TypeView::Atom(name)),
        TypeKind::True => Ok(TypeView::True),
        TypeKind::False => Ok(TypeView::False),
        TypeKind::Never => Ok(TypeView::Never),
        TypeKind::List(inner) => Ok(TypeView::List(ty.with_ty(inner))),
        TypeKind::Optional(inner) => Ok(TypeView::Optional(ty.with_ty(inner))),
        TypeKind::Maybe(inner) => Ok(TypeView::Maybe(ty.with_ty(inner))),
        TypeKind::Code(_) => Err(ReflectError::Unsupported(
            "Code types are staging-only and cannot be reflected".to_string(),
        )),
        TypeKind::Patch { .. } => Err(ReflectError::Unsupported(
            "patch types cannot be reflected in this phase".to_string(),
        )),
        TypeKind::Record(fields, tail) => {
            Ok(TypeView::Record(reflect_record_fields(ty, fields), tail))
        }
        TypeKind::Union(variants, tail) => {
            Ok(TypeView::Union(reflect_union_variants(ty, variants), tail))
        }
        TypeKind::Tuple(items) => Ok(TypeView::Tuple(reflect_tuple_items(ty, items))),
        TypeKind::Function { from, to } => Ok(TypeView::Function {
            from: ty.with_ty(from),
            to: ty.with_ty(to),
        }),
        TypeKind::Effect { base, .. } => Ok(TypeView::Effect {
            base: ty.with_ty(base),
        }),
        TypeKind::TypeVar(binding) => match ty.subst.iter().rev().find(|(b, _)| *b == binding) {
            Some((_, replacement)) => {
                type_view(files, replacement, depth + 1, preserve_named_aliases)
            }
            None => Err(ReflectError::Unsupported(format!(
                "unsubstituted type parameter `{}` cannot be reflected",
                binding_name_in_file(file, binding)
            ))),
        },
        TypeKind::Alias(binding) => {
            let Some((params, body)) = alias_decl(file, binding) else {
                return Err(ReflectError::Unsupported(format!(
                    "unknown type alias `{}` cannot be reflected",
                    binding_name_in_file(file, binding)
                )));
            };
            if !params.is_empty() {
                return Err(ReflectError::Unsupported(format!(
                    "unapplied type constructor `{}` cannot be reflected",
                    binding_name_in_file(file, binding)
                )));
            }
            if preserve_named_aliases {
                return Ok(TypeView::Alias {
                    name: binding_name_in_file(file, binding).to_string(),
                    args: Vec::new(),
                });
            }
            type_view(
                files,
                &ReflectedType::with_subst(ty.module, body, ty.subst.clone()),
                depth + 1,
                preserve_named_aliases,
            )
        }
        TypeKind::AliasApply { binding, args } => {
            let Some((params, body)) = alias_decl(file, binding) else {
                return Err(ReflectError::Unsupported(format!(
                    "unknown type alias `{}` cannot be reflected",
                    binding_name_in_file(file, binding)
                )));
            };
            if params.len() != args.len() {
                return Err(ReflectError::Unsupported(format!(
                    "type constructor `{}` has arity {}, got {}",
                    binding_name_in_file(file, binding),
                    params.len(),
                    args.len()
                )));
            }
            if preserve_named_aliases {
                return Ok(TypeView::Alias {
                    name: binding_name_in_file(file, binding).to_string(),
                    args: args.into_iter().map(|arg| ty.with_ty(arg)).collect(),
                });
            }
            let subst = extend_subst(ty, &params, &args);
            type_view(
                files,
                &ReflectedType::with_subst(ty.module, body, subst),
                depth + 1,
                preserve_named_aliases,
            )
        }
        TypeKind::Apply { .. } => apply_view(files, ty, depth, preserve_named_aliases),
        TypeKind::Con(binding) => Err(ReflectError::Unsupported(format!(
            "unapplied builtin type constructor `{}` cannot be reflected",
            binding_name_in_file(file, binding)
        ))),
        TypeKind::ForAll { .. } => Err(ReflectError::Unsupported(
            "higher-rank polymorphic types cannot be reflected in this phase".to_string(),
        )),
        TypeKind::InferVar(_) | TypeKind::Error => Err(ReflectError::Unsupported(
            "incomplete or erroneous types cannot be reflected".to_string(),
        )),
    }
}

fn apply_view(
    files: &[&ThirFile],
    ty: &ReflectedType,
    depth: u16,
    preserve_named_aliases: bool,
) -> Result<TypeView, ReflectError> {
    let file = file_for_module(files, ty.module)?;
    let mut args = Vec::new();
    let mut head = ty.ty;
    while let TypeKind::Apply { func, arg } = file.type_arena[head.0 as usize].kind.clone() {
        args.push(arg);
        head = func;
    }
    args.reverse();
    match file.type_arena[head.0 as usize].kind.clone() {
        TypeKind::Alias(binding) => {
            let Some((params, body)) = alias_decl(file, binding) else {
                return Err(ReflectError::Unsupported(format!(
                    "unknown type alias `{}` cannot be reflected",
                    binding_name_in_file(file, binding)
                )));
            };
            if params.len() != args.len() {
                return Err(ReflectError::Unsupported(format!(
                    "type constructor `{}` has arity {}, got {}",
                    binding_name_in_file(file, binding),
                    params.len(),
                    args.len()
                )));
            }
            if preserve_named_aliases {
                return Ok(TypeView::Alias {
                    name: binding_name_in_file(file, binding).to_string(),
                    args: args.into_iter().map(|arg| ty.with_ty(arg)).collect(),
                });
            }
            let subst = extend_subst(ty, &params, &args);
            type_view(
                files,
                &ReflectedType::with_subst(ty.module, body, subst),
                depth + 1,
                preserve_named_aliases,
            )
        }
        TypeKind::Con(binding)
            if binding_name_in_file(file, binding) == "List" && args.len() == 1 =>
        {
            Ok(TypeView::List(ty.with_ty(args[0])))
        }
        TypeKind::Con(binding)
            if binding_name_in_file(file, binding) == "Optional" && args.len() == 1 =>
        {
            Ok(TypeView::Optional(ty.with_ty(args[0])))
        }
        TypeKind::Con(binding)
            if binding_name_in_file(file, binding) == "Maybe" && args.len() == 1 =>
        {
            Ok(TypeView::Maybe(ty.with_ty(args[0])))
        }
        _ => Err(ReflectError::Unsupported(
            "higher-kinded or partial type application cannot be reflected".to_string(),
        )),
    }
}

fn file_for_module<'a>(
    files: &[&'a ThirFile],
    module: usize,
) -> Result<&'a ThirFile, ReflectError> {
    files.get(module).copied().ok_or(ReflectError::Internal(
        "type value module is not registered",
    ))
}

struct ReflectedRecordField {
    name: String,
    optional: bool,
    ty: ReflectedType,
}

struct ReflectedUnionVariant {
    name: String,
    payload: Option<ReflectedType>,
}

enum ReflectedTupleItem {
    Named { name: String, ty: ReflectedType },
    Positional(ReflectedType),
}

enum TypeView {
    Alias {
        name: String,
        args: Vec<ReflectedType>,
    },
    Type,
    Bool,
    Text,
    Int,
    Float,
    FixedNum(crate::ir::FixedWidth),
    Posit(zutai_syntax::posit::PositSpec),
    Atom(String),
    True,
    False,
    Never,
    Opaque(String),
    List(ReflectedType),
    Optional(ReflectedType),
    Maybe(ReflectedType),
    Record(Vec<ReflectedRecordField>, RowTail),
    Union(Vec<ReflectedUnionVariant>, RowTail),
    Tuple(Vec<ReflectedTupleItem>),
    Function {
        from: ReflectedType,
        to: ReflectedType,
    },
    Effect {
        base: ReflectedType,
    },
}

fn reflect_record_fields(
    owner: &ReflectedType,
    fields: Vec<TypeRecordField>,
) -> Vec<ReflectedRecordField> {
    fields
        .into_iter()
        .map(|field| ReflectedRecordField {
            name: field.name,
            optional: field.optional,
            ty: owner.with_ty(field.ty),
        })
        .collect()
}

fn reflect_union_variants(
    owner: &ReflectedType,
    variants: Vec<crate::ir::UnionVariant>,
) -> Vec<ReflectedUnionVariant> {
    variants
        .into_iter()
        .map(|variant| ReflectedUnionVariant {
            name: variant.name,
            payload: variant.payload.map(|payload| owner.with_ty(payload)),
        })
        .collect()
}

fn reflect_tuple_items(
    owner: &ReflectedType,
    items: Vec<TypeTupleItem>,
) -> Vec<ReflectedTupleItem> {
    items
        .into_iter()
        .map(|item| match item {
            TypeTupleItem::Named { name, ty, .. } => ReflectedTupleItem::Named {
                name,
                ty: owner.with_ty(ty),
            },
            TypeTupleItem::Positional(ty) => ReflectedTupleItem::Positional(owner.with_ty(ty)),
        })
        .collect()
}

fn alias_decl(file: &ThirFile, binding: BindingId) -> Option<(Vec<BindingId>, TypeId)> {
    file.decls.iter().find_map(|decl_id| {
        let decl = &file.decl_arena[*decl_id];
        match &decl.kind {
            ThirDeclKind::TypeAlias { params, ty } if decl.binding == binding => {
                Some((params.clone(), *ty))
            }
            _ => None,
        }
    })
}

fn binding_name_in_file(file: &ThirFile, binding: BindingId) -> &str {
    file.binding_names
        .get(binding.0 as usize)
        .map_or("<unknown>", String::as_str)
}

fn extend_subst(
    owner: &ReflectedType,
    params: &[BindingId],
    args: &[TypeId],
) -> Rc<[(BindingId, ReflectedType)]> {
    let mut subst: Vec<(BindingId, ReflectedType)> = owner.subst.iter().cloned().collect();
    subst.extend(
        params
            .iter()
            .zip(args.iter())
            .map(|(param, arg)| (*param, owner.with_ty(*arg))),
    );
    Rc::from(subst.into_boxed_slice())
}

fn open_row_reflection_error(kind: &str) -> ReflectError {
    ReflectError::Unsupported(format!(
        "reflection rejects open {kind} rows; close the row before calling `fields` or `schema`"
    ))
}
