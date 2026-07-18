use super::*;

pub(crate) fn is_stdlib_module(source: &zutai_thir::ImportKey, module: &str) -> bool {
    matches!(source, zutai_hir::HirImportSource::Path(parts)
        if matches!(parts.as_slice(), [root, name] if root == "stdlib" && name == module))
}

pub(crate) fn stdlib_module_field<'a>(
    hir: &zutai_hir::HirFile,
    expr: zutai_hir::HirExprId,
    module: &str,
    fields: &'a [&'a str],
    seen: &mut rustc_hash::FxHashSet<zutai_hir::BindingId>,
) -> Option<&'a str> {
    match &hir.expr_arena[expr].kind {
        zutai_hir::HirExprKind::Access { receiver, field } if fields.contains(&field.as_str()) => {
            expr_is_stdlib_import(hir, *receiver, module, seen)
                .then(|| {
                    fields
                        .iter()
                        .copied()
                        .find(|candidate| *candidate == field.as_str())
                })
                .flatten()
        }
        zutai_hir::HirExprKind::BindingRef(binding) => {
            if !seen.insert(*binding) {
                return None;
            }
            value_decl_expr(hir, *binding)
                .and_then(|value| stdlib_module_field(hir, value, module, fields, seen))
        }
        _ => None,
    }
}

pub(crate) fn expr_is_stdlib_import(
    hir: &zutai_hir::HirFile,
    expr: zutai_hir::HirExprId,
    module: &str,
    seen: &mut rustc_hash::FxHashSet<zutai_hir::BindingId>,
) -> bool {
    match &hir.expr_arena[expr].kind {
        zutai_hir::HirExprKind::Import(zutai_hir::HirImportSource::Path(parts)) => {
            matches!(parts.as_slice(), [root, name] if root == "stdlib" && name == module)
        }
        zutai_hir::HirExprKind::BindingRef(binding) => {
            if !seen.insert(*binding) {
                return false;
            }
            value_decl_expr(hir, *binding)
                .is_some_and(|value| expr_is_stdlib_import(hir, value, module, seen))
        }
        _ => false,
    }
}

pub(crate) fn value_decl_expr(
    hir: &zutai_hir::HirFile,
    binding: zutai_hir::BindingId,
) -> Option<zutai_hir::HirExprId> {
    hir.decls.iter().find_map(|decl_id| {
        let decl = &hir.decl_arena[*decl_id];
        if decl.binding != binding {
            return None;
        }
        let zutai_hir::HirDeclKind::Value { value, .. } = decl.kind else {
            return None;
        };
        Some(value)
    })
}

pub(crate) fn thir_decl_exprs(
    file: &zutai_thir::ThirFile,
    binding: zutai_hir::BindingId,
) -> Vec<zutai_thir::ThirExprId> {
    file.decls
        .iter()
        .find_map(|decl_id| {
            let decl = &file.decl_arena[*decl_id];
            if decl.binding != binding {
                return None;
            }
            match &decl.kind {
                zutai_thir::ThirDeclKind::Value { value, .. } => Some(vec![*value]),
                zutai_thir::ThirDeclKind::Function { clauses, .. } => Some(
                    clauses
                        .iter()
                        .flat_map(|clause| {
                            clause.guard.into_iter().chain(std::iter::once(clause.body))
                        })
                        .collect(),
                ),
                _ => Some(Vec::new()),
            }
        })
        .unwrap_or_default()
}

pub(crate) fn thir_expr_is_stdlib_reflect_alias(
    hir: &zutai_hir::HirFile,
    file: &zutai_thir::ThirFile,
    expr: zutai_thir::ThirExprId,
    fields: &[&str],
    seen_bindings: &mut FxHashSet<zutai_hir::BindingId>,
) -> bool {
    if stdlib_module_field(
        hir,
        file.expr_arena[expr].source,
        "reflect",
        fields,
        &mut FxHashSet::default(),
    )
    .is_some()
    {
        return true;
    }
    match &file.expr_arena[expr].kind {
        zutai_thir::ThirExprKind::BindingRef { binding, .. } => {
            if seen_bindings.insert(*binding) {
                for body in thir_decl_exprs(file, *binding) {
                    if thir_expr_is_stdlib_reflect_alias(hir, file, body, fields, seen_bindings) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}
