use zutai_syntax::Span;
use zutai_thir::{
    ImportedField, ImportedFieldProvenance, ImportedProvenance, ImportedProvenanceChildren,
    ImportedType, ThirDeclKind, ThirExprKind, ThirFile,
};

/// types; an empty array yields `Unknown` (a fresh inference variable in THIR).
pub(crate) fn imported_type(value: &zutai_im::Value) -> ImportedType {
    use zutai_im::Value;
    match value {
        Value::True | Value::False => ImportedType::Bool,
        Value::Integer(_) => ImportedType::Int,
        Value::Float(_) => ImportedType::Float,
        Value::String(_) => ImportedType::Text,
        Value::Atom(name) => ImportedType::Atom(name.clone()),
        Value::Block(block) => ImportedType::Record(
            block
                .iter()
                .map(|pair| ImportedField {
                    name: pair.field_name.clone(),
                    optional: false,
                    ty: imported_type(&pair.value),
                })
                .collect(),
        ),
        Value::Array(items) => ImportedType::List(Box::new(array_element_type(items))),
    }
}

pub(crate) fn immediate_span(span: zutai_im::ByteSpan) -> Span {
    Span::new(span.start, span.end)
}

pub(crate) fn block_provenance(block: &zutai_im::LocatedBlock) -> ImportedProvenance {
    let value = zutai_im::Value::Block(block.value.clone());
    ImportedProvenance {
        ty: imported_type(&value),
        span: immediate_span(block.span),
        name_span: None,
        children: ImportedProvenanceChildren::Record(
            block
                .fields
                .iter()
                .map(|field| ImportedFieldProvenance {
                    name: field.field_name.clone(),
                    value: value_provenance(&field.value, Some(field.name_span)),
                })
                .collect(),
        ),
    }
}

pub(crate) fn value_provenance(
    value: &zutai_im::LocatedValue,
    name_span: Option<zutai_im::ByteSpan>,
) -> ImportedProvenance {
    let children = match &value.children {
        zutai_im::LocatedChildren::Scalar => ImportedProvenanceChildren::Scalar,
        zutai_im::LocatedChildren::Array(items) => ImportedProvenanceChildren::List(
            items
                .iter()
                .map(|item| value_provenance(item, None))
                .collect(),
        ),
        zutai_im::LocatedChildren::Block(fields) => ImportedProvenanceChildren::Record(
            fields
                .iter()
                .map(|field| ImportedFieldProvenance {
                    name: field.field_name.clone(),
                    value: value_provenance(&field.value, Some(field.name_span)),
                })
                .collect(),
        ),
    };
    ImportedProvenance {
        ty: imported_type(&value.value),
        span: immediate_span(value.span),
        name_span: name_span.map(immediate_span),
        children,
    }
}

pub(crate) fn array_element_type(items: &[zutai_im::Value]) -> ImportedType {
    let mut distinct: Vec<ImportedType> = Vec::new();
    for item in items {
        let ty = imported_type(item);
        if !distinct.contains(&ty) {
            distinct.push(ty);
        }
    }
    match distinct.len() {
        0 => ImportedType::Unknown,
        1 => distinct.pop().unwrap(),
        // Heterogeneous arrays have no meaningful tag names for the variants,
        // so fall back to Unknown and let the consumer unify with what it needs.
        _ => ImportedType::Unknown,
    }
}

/// Enrich `ImportedType::Type` placeholders with their concrete denotations
/// recovered from the module's final expression.
///
/// `export_type` converts a bare `TypeKind::Type` slot (which is payload-less)
/// to `ImportedType::Type(Unknown)`. This function upgrades those placeholders
/// by walking the module's final-expression AST. For each record field whose
/// THIR value is a `TypeValue(tid)`, and for a direct type-valued final
/// expression, it calls `export_type(file, tid)` to obtain the real denotation.
///
/// Non-type final expressions (scalars, functions, …) are returned as-is.
pub(crate) fn enrich_with_type_denotations(ty: ImportedType, file: &ThirFile) -> ImportedType {
    let final_expr = &file.expr_arena[file.final_expr];
    match ty {
        ImportedType::Type(_) => {
            if let ThirExprKind::TypeValue(denotation_tid) = final_expr.kind
                && let Ok(denotation) = zutai_thir::export_type_value(file, denotation_tid)
            {
                ImportedType::Type(Box::new(denotation))
            } else {
                ImportedType::Type(Box::new(ImportedType::Unknown))
            }
        }
        ImportedType::Record(mut fields) => {
            let ThirExprKind::Record(thir_fields) = &final_expr.kind else {
                return ImportedType::Record(fields);
            };

            for thir_field in thir_fields {
                // Only enrich fields that are already `Type(Unknown)` placeholders.
                let Some(imp_field) = fields.iter_mut().find(|f| f.name == thir_field.name) else {
                    continue;
                };
                if !matches!(imp_field.ty, ImportedType::Type(_)) {
                    continue;
                }
                // The THIR field value must be a TypeValue to carry a denotation.
                let value_expr = &file.expr_arena[thir_field.value];
                if let ThirExprKind::TypeValue(denotation_tid) = value_expr.kind
                    && let Ok(denotation) = zutai_thir::export_type_value(file, denotation_tid)
                {
                    imp_field.ty = ImportedType::Type(Box::new(denotation));
                }
            }

            ImportedType::Record(fields)
        }
        other => other,
    }
}

/// Attach top-level type aliases to a module import as annotation-only exports.
/// These names are intentionally not added to the runtime value's record type,
/// so importing a module for effectful functions does not force the evaluator
/// down the TypeValue/reflection path.
pub(crate) fn attach_type_only_exports(ty: ImportedType, file: &ThirFile) -> ImportedType {
    let mut types = Vec::new();
    for (_, decl) in file.decl_arena.iter() {
        if !matches!(decl.kind, ThirDeclKind::TypeAlias { .. }) {
            continue;
        }
        let name = file.binding_names[decl.binding.0 as usize].clone();
        if let Ok(denotation) = zutai_thir::export_type_alias_value(file, decl.binding) {
            types.push(ImportedField {
                name,
                optional: false,
                ty: ImportedType::Type(Box::new(denotation)),
            });
        }
    }

    if types.is_empty() {
        ty
    } else {
        ImportedType::WithTypeExports {
            value: Box::new(ty),
            types,
        }
    }
}
