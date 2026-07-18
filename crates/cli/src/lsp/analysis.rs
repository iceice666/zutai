//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::diagnostics::format_import_diagnostic;

use super::*;

pub(super) fn imported_member_selection(
    source: &str,
    file: &zutai_hir::HirFile,
    offset: usize,
) -> Option<(usize, usize)> {
    file.expr_arena
        .iter()
        .filter_map(|(_, expr)| match &expr.kind {
            zutai_hir::HirExprKind::Access { field, .. } => {
                let range = access_field_range(source, expr.span, field)?;
                ((range.0..=range.1).contains(&offset)).then_some(range)
            }
            _ => None,
        })
        .chain(file.type_arena.iter().filter_map(|(_, ty)| match &ty.kind {
            zutai_hir::HirTypeKind::Access { field, .. } => {
                let range = access_field_range(source, ty.span, field)?;
                ((range.0..=range.1).contains(&offset)).then_some(range)
            }
            _ => None,
        }))
        .min_by_key(|(start, end)| end - start)
}

pub(super) fn exported_member_origin(
    project: &ProjectAnalysis,
    analysis: &zutai_semantic::Analysis,
    member: &str,
) -> Option<(PathBuf, String)> {
    let file = analysis.hir.as_ref().map(|lowered| &lowered.file)?;
    let field = match &file.expr_arena[file.final_expr].kind {
        zutai_hir::HirExprKind::Record(items) => items.iter().find_map(|item| match item {
            zutai_hir::HirRecordItem::Field(field) if field.name == member => Some(field),
            _ => None,
        })?,
        _ => return None,
    };
    let zutai_hir::HirExprKind::Access { receiver, field } = &file.expr_arena[field.value].kind
    else {
        return None;
    };
    let zutai_hir::HirExprKind::BindingRef(import_binding) = file.expr_arena[*receiver].kind else {
        return None;
    };
    let import = import_source_for_binding(file, import_binding)?;
    let target = analysis.import_modules.get(&import)?;
    Some(resolve_exported_member_target(project, target, field))
}

pub(super) fn resolve_exported_member_target(
    project: &ProjectAnalysis,
    analysis: &zutai_semantic::Analysis,
    member: &str,
) -> (PathBuf, String) {
    exported_member_origin(project, analysis, member)
        .unwrap_or_else(|| (project.module_identity(analysis), member.to_owned()))
}

pub(super) fn exported_member_local_binding(
    analysis: &zutai_semantic::Analysis,
    member: &str,
) -> Option<zutai_hir::BindingId> {
    let file = analysis.hir.as_ref().map(|lowered| &lowered.file)?;
    let zutai_hir::HirExprKind::Record(items) = &file.expr_arena[file.final_expr].kind else {
        return None;
    };
    items.iter().find_map(|item| {
        let zutai_hir::HirRecordItem::Field(field) = item else {
            return None;
        };
        if field.name != member {
            return None;
        }
        match file.expr_arena[field.value].kind {
            zutai_hir::HirExprKind::BindingRef(binding) => file
                .bindings
                .get(binding.0 as usize)
                .is_some_and(|binding| binding.name == member)
                .then_some(binding),
            _ => None,
        }
    })
}

pub(super) fn exported_member_for_binding(
    analysis: &zutai_semantic::Analysis,
    binding: zutai_hir::BindingId,
) -> Option<String> {
    let file = analysis.hir.as_ref().map(|lowered| &lowered.file)?;
    let zutai_hir::HirExprKind::Record(items) = &file.expr_arena[file.final_expr].kind else {
        return None;
    };
    items.iter().find_map(|item| {
        let zutai_hir::HirRecordItem::Field(field) = item else {
            return None;
        };
        matches!(file.expr_arena[field.value].kind, zutai_hir::HirExprKind::BindingRef(candidate) if candidate == binding)
            .then(|| field.name.clone())
    })
}

pub(super) fn exported_field_range(
    source: &str,
    analysis: &zutai_semantic::Analysis,
    member: &str,
) -> Option<(usize, usize)> {
    let file = analysis.hir.as_ref().map(|lowered| &lowered.file)?;
    let zutai_hir::HirExprKind::Record(items) = &file.expr_arena[file.final_expr].kind else {
        return None;
    };
    items.iter().find_map(|item| {
        let zutai_hir::HirRecordItem::Field(field) = item else {
            return None;
        };
        (field.name == member)
            .then(|| name_range_in_span(source, field.span, member))
            .flatten()
    })
}

pub(super) fn imported_member_reference_ranges(
    source: &str,
    analysis: &zutai_semantic::Analysis,
    project: &ProjectAnalysis,
    target_module: &Path,
    member: &str,
) -> Vec<(usize, usize)> {
    let Some(file) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
        return Vec::new();
    };
    let imports: HashSet<_> = file
        .decl_arena
        .iter()
        .filter_map(|(_, decl)| {
            let zutai_hir::HirDeclKind::Value { value, .. } = decl.kind else {
                return None;
            };
            let zutai_hir::HirExprKind::Import(import) = &file.expr_arena[value].kind else {
                return None;
            };
            let module = analysis.import_modules.get(import)?;
            let (origin, origin_member) = resolve_exported_member_target(project, module, member);
            (origin == target_module && origin_member == member).then_some(decl.binding)
        })
        .collect();
    let mut ranges: Vec<_> = file
        .expr_arena
        .iter()
        .filter_map(|(_, expr)| {
            let zutai_hir::HirExprKind::Access { receiver, field } = &expr.kind else {
                return None;
            };
            if field != member {
                return None;
            }
            let zutai_hir::HirExprKind::BindingRef(binding) = file.expr_arena[*receiver].kind
            else {
                return None;
            };
            imports
                .contains(&binding)
                .then(|| access_field_range(source, expr.span, field))
                .flatten()
        })
        .chain(file.type_arena.iter().filter_map(|(_, ty)| {
            let zutai_hir::HirTypeKind::Access { receiver, field } = &ty.kind else {
                return None;
            };
            if field != member {
                return None;
            }
            let zutai_hir::HirTypeKind::BindingRef(binding) = file.type_arena[*receiver].kind
            else {
                return None;
            };
            imports
                .contains(&binding)
                .then(|| access_field_range(source, ty.span, field))
                .flatten()
        }))
        .collect();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

pub(super) fn analyze(source: &str, uri: &str) -> Option<zutai_semantic::Analysis> {
    let path = file_path(uri)?;
    if path.extension().and_then(|ext| ext.to_str()) != Some("zt") {
        return None;
    }
    Some(zutai_semantic::analyze_with_base(
        source,
        path.parent(),
        zutai_semantic::AnalysisOptions::default(),
    ))
}

#[cfg(test)]
pub(super) fn diagnostics(source: &str, analysis: &zutai_semantic::Analysis) -> Vec<Value> {
    analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic_value(source, "file:///test.zt", diagnostic))
        .collect()
}

pub(super) fn diagnostic_value(
    source: &str,
    uri: &str,
    diagnostic: &zutai_semantic::SemanticDiagnostic,
) -> Value {
    let metadata = diagnostic.metadata();
    let message = match &diagnostic.kind {
        zutai_semantic::SemanticDiagnosticKind::Parse(parse) => parse.message.clone(),
        zutai_semantic::SemanticDiagnosticKind::Import(import) => format_import_diagnostic(import),
        _ => {
            zutai_eval::describe_semantic_diagnostic(diagnostic)
                .expect("HIR and THIR diagnostics always have a source span")
                .0
        }
    };
    let mut value = json!({
        "range": range(
            source,
            metadata.primary_span.start as usize,
            metadata.primary_span.end as usize,
        ),
        "severity": severity(metadata.severity),
        "code": metadata.code,
        "source": "zutai",
        "message": message,
    });
    if let Some((related, label)) = metadata.related {
        value["relatedInformation"] = json!([{
            "location": {
                "uri": uri,
                "range": range(source, related.start as usize, related.end as usize),
            },
            "message": label,
        }]);
    }
    if let zutai_semantic::SemanticDiagnosticKind::Thir(thir) = &diagnostic.kind
        && let Some((related, label)) = thir.related_location_in(source)
    {
        value["relatedInformation"] = json!([{
            "location": {
                "uri": uri,
                "range": range(source, related.start as usize, related.end as usize),
            },
            "message": label,
        }]);
    }
    value
}

/// Find the narrowest resolved value or type reference at an LSP byte offset.
/// This deliberately consults HIR rather than THIR: name resolution survives
/// later type errors, so definition navigation remains useful while editing an
/// incomplete program.
pub(super) fn binding_at(file: &zutai_hir::HirFile, offset: usize) -> Option<zutai_hir::BindingId> {
    file.expr_arena
        .iter()
        .filter_map(|(_, expr)| match expr.kind {
            zutai_hir::HirExprKind::BindingRef(binding) if contains(expr.span, offset) => {
                Some((binding, expr.span))
            }
            _ => None,
        })
        .chain(file.type_arena.iter().filter_map(|(_, ty)| match ty.kind {
            zutai_hir::HirTypeKind::BindingRef(binding) if contains(ty.span, offset) => {
                Some((binding, ty.span))
            }
            _ => None,
        }))
        .min_by_key(|(_, span)| span.end.saturating_sub(span.start))
        .map(|(binding, _)| binding)
}

/// Return the source range of a declaration/binder only when it belongs to the
/// current document. Embedded preludes share the HIR binding table but have
/// spans into a different source buffer, so callers must not expose them as
/// locations or edits in the editor document.
pub(super) fn binding_range(source: &str, binding: &zutai_hir::Binding) -> Option<(usize, usize)> {
    let start = binding.span.start as usize;
    let end = start.checked_add(binding.name.len())?;
    (source.get(start..end) == Some(binding.name.as_str())).then_some((start, end))
}

pub(super) fn binding_declaration_at(
    source: &str,
    file: &zutai_hir::HirFile,
    offset: usize,
) -> Option<zutai_hir::BindingId> {
    file.bindings
        .iter()
        .enumerate()
        .filter_map(|(index, binding)| {
            let (start, end) = binding_range(source, binding)?;
            ((start..=end).contains(&offset))
                .then_some((zutai_hir::BindingId(index as u32), end - start))
        })
        .min_by_key(|(_, length)| *length)
        .map(|(binding, _)| binding)
}

pub(super) fn binding_reference_ranges(
    source: &str,
    file: &zutai_hir::HirFile,
    binding: zutai_hir::BindingId,
    include_declaration: bool,
) -> Vec<(usize, usize)> {
    let mut ranges: Vec<_> = file
        .expr_arena
        .iter()
        .filter_map(|(_, expr)| match expr.kind {
            zutai_hir::HirExprKind::BindingRef(candidate) if candidate == binding => {
                let start = expr.span.start as usize;
                let end = expr.span.end as usize;
                source.get(start..end).is_some().then_some((start, end))
            }
            _ => None,
        })
        .chain(file.type_arena.iter().filter_map(|(_, ty)| match ty.kind {
            zutai_hir::HirTypeKind::BindingRef(candidate) if candidate == binding => {
                let start = ty.span.start as usize;
                let end = ty.span.end as usize;
                source.get(start..end).is_some().then_some((start, end))
            }
            _ => None,
        }))
        .collect();
    if include_declaration
        && let Some(binding) = file.bindings.get(binding.0 as usize)
        && let Some(range) = binding_range(source, binding)
    {
        ranges.push(range);
    }
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

pub(super) fn imported_member_at(
    source: &str,
    file: &zutai_hir::HirFile,
    offset: usize,
) -> Option<(zutai_hir::BindingId, String)> {
    file.expr_arena
        .iter()
        .filter_map(|(_, expr)| {
            let zutai_hir::HirExprKind::Access { receiver, field } = &expr.kind else {
                return None;
            };
            let field_range = access_field_range(source, expr.span, field)?;
            if !(field_range.0..=field_range.1).contains(&offset) {
                return None;
            }
            let zutai_hir::HirExprKind::BindingRef(binding) = file.expr_arena[*receiver].kind
            else {
                return None;
            };
            Some((binding, field.clone(), field_range.1 - field_range.0))
        })
        .chain(file.type_arena.iter().filter_map(|(_, ty)| {
            let zutai_hir::HirTypeKind::Access { receiver, field } = &ty.kind else {
                return None;
            };
            let field_range = access_field_range(source, ty.span, field)?;
            if !(field_range.0..=field_range.1).contains(&offset) {
                return None;
            }
            let zutai_hir::HirTypeKind::BindingRef(binding) = file.type_arena[*receiver].kind
            else {
                return None;
            };
            Some((binding, field.clone(), field_range.1 - field_range.0))
        }))
        .min_by_key(|(_, _, length)| *length)
        .map(|(binding, member, _)| (binding, member))
}

pub(super) fn document_symbol_values(source: &str, hir: &zutai_hir::HirFile) -> Vec<Value> {
    hir.decl_arena
        .iter()
        .filter_map(|(_, decl)| {
            let binding = hir.bindings.get(decl.binding.0 as usize)?;
            let (start, end) = binding_range(source, binding)?;
            Some(json!({
                "name": binding.name,
                "detail": binding_kind_label(binding.kind),
                "kind": symbol_kind(binding.kind),
                "range": range(source, decl.span.start as usize, decl.span.end as usize),
                "selectionRange": range(source, start, end),
            }))
        })
        .collect()
}

pub(super) fn append_workspace_symbols(
    symbols: &mut Vec<Value>,
    query: &str,
    analysis: &zutai_semantic::Analysis,
    uri: &str,
    source: &str,
    container: &str,
) {
    let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
        return;
    };
    symbols.extend(hir.decl_arena.iter().filter_map(|(_, decl)| {
        let binding = hir.bindings.get(decl.binding.0 as usize)?;
        if binding.name.starts_with('$')
            || (!query.is_empty() && !binding.name.to_lowercase().contains(query))
        {
            return None;
        }
        let (start, end) = binding_range(source, binding)?;
        Some(json!({
            "name": binding.name,
            "kind": symbol_kind(binding.kind),
            "location": {
                "uri": uri,
                "range": range(source, start, end),
            },
            "containerName": container,
        }))
    }));
}

pub(super) fn workspace_symbol_key(symbol: &Value) -> (String, String, u64, u64) {
    (
        symbol
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        symbol
            .pointer("/location/uri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        symbol
            .pointer("/location/range/start/line")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        symbol
            .pointer("/location/range/start/character")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    )
}

pub(super) fn workspace_symbol_container(
    project: &ProjectAnalysis,
    analysis: &zutai_semantic::Analysis,
) -> String {
    if std::ptr::eq(analysis, &project.analysis) {
        return project
            .completion_packages()
            .root_package
            .as_ref()
            .and_then(|id| project.completion_packages().packages.get(id))
            .map(|package| package.name.clone())
            .unwrap_or_else(|| "root".to_owned());
    }
    if let Some((id, source)) = analysis
        .source_path
        .as_deref()
        .and_then(portable_package_path)
        && let Some(package) = project.completion_packages().packages.get(id)
    {
        let module = package
            .modules
            .iter()
            .find(|(_, path)| path.as_str() == source)
            .map(|(module, _)| module.as_str())
            .unwrap_or(source);
        return format!("{}.{}", package.name, module);
    }
    analysis
        .source_path
        .as_ref()
        .and_then(|path| path.file_stem())
        .and_then(|name| name.to_str())
        .unwrap_or("module")
        .to_owned()
}

pub(super) fn import_completion_context(
    source: &str,
    offset: usize,
) -> Option<ImportCompletionContext> {
    let before = source.get(..offset)?;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let line = before.get(line_start..)?;
    let import = line.rfind("import")?;
    if line[..import]
        .chars()
        .next_back()
        .is_some_and(zutai_syntax::ident::is_ident_continue)
    {
        return None;
    }
    let mut tail_start = line_start + import + "import".len();
    if source[tail_start..offset]
        .chars()
        .next()
        .is_some_and(|character| !character.is_whitespace())
    {
        return None;
    }
    while source[tail_start..offset]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        tail_start += source[tail_start..offset].chars().next()?.len_utf8();
    }
    let tail = source.get(tail_start..offset)?;
    if tail.starts_with('"')
        || tail.chars().any(|character| {
            !(zutai_syntax::ident::is_ident_continue(character) || character == '.')
        })
    {
        return None;
    }
    let last_dot = tail.rfind('.');
    let completed = last_dot
        .map(|index| &tail[..index])
        .unwrap_or_default()
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    let prefix_start = last_dot.map_or(tail_start, |index| tail_start + index + 1);
    Some(ImportCompletionContext {
        completed,
        prefix: source[prefix_start..offset].to_owned(),
        start: prefix_start,
    })
}

pub(super) fn package_import_candidates(
    project: &ProjectAnalysis,
    analysis: &zutai_semantic::Analysis,
    completed: &[String],
) -> Vec<CompletionCandidate> {
    let graph = project.completion_packages();
    let Some(owner) = project.owner_package(analysis) else {
        return Vec::new();
    };
    let Some(package) = graph.packages.get(owner) else {
        return Vec::new();
    };
    if completed.is_empty() {
        return package
            .dependencies
            .keys()
            .map(|alias| CompletionCandidate {
                name: alias.clone(),
                kind: 9,
                detail: "package alias".to_owned(),
            })
            .collect();
    }
    let Some(target) = package
        .dependencies
        .get(&completed[0])
        .and_then(|target| graph.packages.get(target))
    else {
        return Vec::new();
    };
    let module_prefix = completed.get(1..).unwrap_or_default().join(".");
    let prefix = (!module_prefix.is_empty()).then(|| format!("{module_prefix}."));
    target
        .modules
        .keys()
        .filter_map(|module| {
            let candidate = match prefix.as_deref() {
                Some(prefix) => module.strip_prefix(prefix)?,
                None => module.as_str(),
            };
            let segment = candidate.split('.').next()?;
            Some(CompletionCandidate {
                name: segment.to_owned(),
                kind: 9,
                detail: if candidate.contains('.') {
                    "module namespace".to_owned()
                } else {
                    "public module".to_owned()
                },
            })
        })
        .collect()
}

pub(super) fn member_completion_binding(
    source: &str,
    file: &zutai_hir::HirFile,
    prefix_start: usize,
) -> Option<zutai_hir::BindingId> {
    let dot = source.get(..prefix_start)?.strip_suffix('.')?;
    let mut start = dot.len();
    while let Some(character) = source[..start].chars().next_back() {
        if !zutai_syntax::ident::is_ident_continue(character) {
            break;
        }
        start -= character.len_utf8();
    }
    let receiver = source.get(start..dot.len())?;
    file.bindings
        .iter()
        .enumerate()
        .filter(|(_, binding)| {
            binding.name == receiver && completion_binding(binding, source, prefix_start)
        })
        .max_by_key(|(_, binding)| binding.span.start)
        .map(|(index, _)| zutai_hir::BindingId(index as u32))
}

pub(super) fn exported_member_candidates(
    analysis: &zutai_semantic::Analysis,
) -> Vec<CompletionCandidate> {
    let Some(file) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
        return Vec::new();
    };
    let zutai_hir::HirExprKind::Record(items) = &file.expr_arena[file.final_expr].kind else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let zutai_hir::HirRecordItem::Field(field) = item else {
                return None;
            };
            let (kind, detail) = match file.expr_arena[field.value].kind {
                zutai_hir::HirExprKind::BindingRef(binding) => file
                    .bindings
                    .get(binding.0 as usize)
                    .map(|binding| {
                        (
                            completion_kind(binding.kind),
                            binding_kind_label(binding.kind).to_owned(),
                        )
                    })
                    .unwrap_or((6, "exported member".to_owned())),
                zutai_hir::HirExprKind::TypeForm(_) => (7, "type".to_owned()),
                _ => (6, "exported member".to_owned()),
            };
            Some(CompletionCandidate {
                name: field.name.clone(),
                kind,
                detail,
            })
        })
        .collect()
}

pub(super) fn completion_items(
    mut candidates: Vec<CompletionCandidate>,
    prefix: &str,
    replacement: Value,
) -> Value {
    candidates.sort_by(|left, right| {
        (&left.name, left.kind, &left.detail).cmp(&(&right.name, right.kind, &right.detail))
    });
    candidates.dedup_by(|left, right| left.name == right.name);
    Value::Array(
        candidates
            .into_iter()
            .filter(|candidate| candidate.name.starts_with(prefix))
            .map(|candidate| {
                json!({
                    "label": candidate.name,
                    "kind": candidate.kind,
                    "detail": candidate.detail,
                    "sortText": candidate.name,
                    "textEdit": { "range": replacement, "newText": candidate.name },
                })
            })
            .collect(),
    )
}

pub(super) fn import_source_for_binding(
    file: &zutai_hir::HirFile,
    binding: zutai_hir::BindingId,
) -> Option<zutai_hir::HirImportSource> {
    file.decl_arena.iter().find_map(|(_, decl)| {
        if decl.binding != binding {
            return None;
        }
        let zutai_hir::HirDeclKind::Value { value, .. } = decl.kind else {
            return None;
        };
        match &file.expr_arena[value].kind {
            zutai_hir::HirExprKind::Import(source) => Some(source.clone()),
            _ => None,
        }
    })
}

pub(super) fn exported_member_range(
    source: &str,
    analysis: &zutai_semantic::Analysis,
    member: &str,
) -> Option<(usize, usize)> {
    let file = analysis.hir.as_ref().map(|lowered| &lowered.file)?;
    if let zutai_hir::HirExprKind::Record(items) = &file.expr_arena[file.final_expr].kind {
        for item in items {
            let zutai_hir::HirRecordItem::Field(field) = item else {
                continue;
            };
            if field.name != member {
                continue;
            }
            if let zutai_hir::HirExprKind::BindingRef(binding) = file.expr_arena[field.value].kind
                && let Some(binding) = file.bindings.get(binding.0 as usize)
                && let Some(range) = binding_range(source, binding)
            {
                return Some(range);
            }
            return name_range_in_span(source, field.span, member);
        }
    }
    file.decl_arena.iter().find_map(|(_, decl)| {
        let binding = file.bindings.get(decl.binding.0 as usize)?;
        (binding.name == member)
            .then(|| binding_range(source, binding))
            .flatten()
    })
}

pub(super) fn access_field_range(
    source: &str,
    span: zutai_syntax::Span,
    field: &str,
) -> Option<(usize, usize)> {
    let start = span.start as usize;
    let end = span.end as usize;
    let slice = source.get(start..end)?;
    let needle = format!(".{field}");
    let relative = slice.rfind(&needle)?;
    let field_start = start + relative + 1;
    Some((field_start, field_start + field.len()))
}

pub(super) fn name_range_in_span(
    source: &str,
    span: zutai_syntax::Span,
    name: &str,
) -> Option<(usize, usize)> {
    let start = span.start as usize;
    let end = span.end as usize;
    let relative = source.get(start..end)?.find(name)?;
    let name_start = start + relative;
    Some((name_start, name_start + name.len()))
}

pub(super) fn completion_binding(
    binding: &zutai_hir::Binding,
    source: &str,
    offset: usize,
) -> bool {
    if binding.name.starts_with('$') {
        return false;
    }
    match binding.kind {
        zutai_hir::BindingKind::BuiltinType
        | zutai_hir::BindingKind::BuiltinValue
        | zutai_hir::BindingKind::TopValue
        | zutai_hir::BindingKind::TopFunction
        | zutai_hir::BindingKind::TopType
        | zutai_hir::BindingKind::TopConstraint
        | zutai_hir::BindingKind::TopWitness => true,
        zutai_hir::BindingKind::ConstraintMethod
        | zutai_hir::BindingKind::TypeParam
        | zutai_hir::BindingKind::LevelParam
        | zutai_hir::BindingKind::Local
        | zutai_hir::BindingKind::Param => {
            (binding.span.start as usize) <= offset && binding_range(source, binding).is_some()
        }
    }
}

pub(super) fn binding_kind_label(kind: zutai_hir::BindingKind) -> &'static str {
    match kind {
        zutai_hir::BindingKind::BuiltinType => "builtin type",
        zutai_hir::BindingKind::BuiltinValue => "builtin value",
        zutai_hir::BindingKind::TopValue => "value",
        zutai_hir::BindingKind::TopFunction => "function",
        zutai_hir::BindingKind::TopType => "type",
        zutai_hir::BindingKind::TopConstraint => "constraint",
        zutai_hir::BindingKind::TopWitness => "witness",
        zutai_hir::BindingKind::ConstraintMethod => "constraint method",
        zutai_hir::BindingKind::TypeParam => "type parameter",
        zutai_hir::BindingKind::LevelParam => "universe level parameter",
        zutai_hir::BindingKind::Local => "local value",
        zutai_hir::BindingKind::Param => "parameter",
    }
}

pub(super) fn completion_kind(kind: zutai_hir::BindingKind) -> u8 {
    match kind {
        zutai_hir::BindingKind::TopFunction | zutai_hir::BindingKind::ConstraintMethod => 3,
        zutai_hir::BindingKind::TopType | zutai_hir::BindingKind::BuiltinType => 7,
        zutai_hir::BindingKind::TopConstraint => 8,
        zutai_hir::BindingKind::TypeParam | zutai_hir::BindingKind::LevelParam => 25,
        zutai_hir::BindingKind::Param => 6,
        _ => 6,
    }
}

pub(super) fn symbol_kind(kind: zutai_hir::BindingKind) -> u8 {
    match kind {
        zutai_hir::BindingKind::TopFunction => 12,
        zutai_hir::BindingKind::TopType => 5,
        zutai_hir::BindingKind::TopConstraint => 11,
        zutai_hir::BindingKind::TopWitness => 14,
        _ => 13,
    }
}

pub(super) fn renameable_binding(source: &str, binding: &zutai_hir::Binding) -> bool {
    !matches!(
        binding.kind,
        zutai_hir::BindingKind::BuiltinType | zutai_hir::BindingKind::BuiltinValue
    ) && !binding.name.starts_with('$')
        && binding_range(source, binding).is_some()
}

pub(super) fn render_type(file: &zutai_thir::ThirFile, id: zutai_thir::TypeId) -> String {
    fn go(
        file: &zutai_thir::ThirFile,
        id: zutai_thir::TypeId,
        seen: &mut Vec<zutai_thir::TypeId>,
    ) -> String {
        if seen.contains(&id) {
            return "…".to_string();
        }
        let Some(ty) = file.type_arena.get(id.0 as usize) else {
            return "<invalid type>".to_string();
        };
        seen.push(id);
        let result = match &ty.kind {
            zutai_thir::TypeKind::Type(_) => "Type".to_string(),
            zutai_thir::TypeKind::Bool => "Bool".to_string(),
            zutai_thir::TypeKind::Text => "Text".to_string(),
            zutai_thir::TypeKind::Int => "Int".to_string(),
            zutai_thir::TypeKind::Float => "Float".to_string(),
            zutai_thir::TypeKind::FixedNum(width) => width.name().to_string(),
            zutai_thir::TypeKind::Posit(spec) => format!("{spec:?}"),
            zutai_thir::TypeKind::Opaque(name) => name.clone(),
            zutai_thir::TypeKind::Atom(name) => format!("#{name}"),
            zutai_thir::TypeKind::True => "true".to_string(),
            zutai_thir::TypeKind::False => "false".to_string(),
            zutai_thir::TypeKind::List(inner) => format!("List {}", go(file, *inner, seen)),
            zutai_thir::TypeKind::Optional(inner) => format!("{}?", go(file, *inner, seen)),
            zutai_thir::TypeKind::Maybe(inner) => format!("Maybe {}", go(file, *inner, seen)),
            zutai_thir::TypeKind::Code(inner) => format!("Code {}", go(file, *inner, seen)),
            zutai_thir::TypeKind::Patch { target, deep } => {
                format!(
                    "{}Patch {}",
                    if *deep { "Deep" } else { "" },
                    go(file, *target, seen)
                )
            }
            zutai_thir::TypeKind::Record(fields, tail) => {
                let mut fields: Vec<_> = fields
                    .iter()
                    .map(|field| {
                        format!(
                            "{}{}: {}",
                            field.name,
                            if field.optional { "?" } else { "" },
                            go(file, field.ty, seen)
                        )
                    })
                    .collect();
                if !matches!(tail, zutai_thir::RowTail::Closed) {
                    fields.push("...".to_string());
                }
                format!("{{ {} }}", fields.join("; "))
            }
            zutai_thir::TypeKind::Union(variants, tail) => {
                let mut variants: Vec<_> = variants
                    .iter()
                    .map(|variant| match variant.payload {
                        Some(payload) => format!("#{} ({})", variant.name, go(file, payload, seen)),
                        None => format!("#{}", variant.name),
                    })
                    .collect();
                if !matches!(tail, zutai_thir::RowTail::Closed) {
                    variants.push("...".to_string());
                }
                format!("<{}>", variants.join(" | "))
            }
            zutai_thir::TypeKind::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(|item| match item {
                        zutai_thir::TypeTupleItem::Named { name, ty, .. } =>
                            format!("{name}: {}", go(file, *ty, seen)),
                        zutai_thir::TypeTupleItem::Positional(ty) => go(file, *ty, seen),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            zutai_thir::TypeKind::Function { from, to } => {
                format!("{} -> {}", go(file, *from, seen), go(file, *to, seen))
            }
            zutai_thir::TypeKind::Effect { base, row } => {
                let ops = row
                    .ops
                    .iter()
                    .map(|op| {
                        format!(
                            "{}: {} -> {}",
                            op.name,
                            go(file, op.param, seen),
                            go(file, op.result, seen)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("{} ! {{ {ops} }}", go(file, *base, seen))
            }
            zutai_thir::TypeKind::Never => "Never".to_string(),
            zutai_thir::TypeKind::TypeVar(binding)
            | zutai_thir::TypeKind::Alias(binding)
            | zutai_thir::TypeKind::Con(binding) => file
                .binding_names
                .get(binding.0 as usize)
                .cloned()
                .unwrap_or_else(|| format!("T{}", binding.0)),
            zutai_thir::TypeKind::InferVar(id) => format!("?{id}"),
            zutai_thir::TypeKind::AliasApply { binding, args } => format!(
                "{} {}",
                file.binding_names
                    .get(binding.0 as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("T{}", binding.0)),
                args.iter()
                    .map(|arg| go(file, *arg, seen))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            zutai_thir::TypeKind::Apply { func, arg } => {
                format!("{} {}", go(file, *func, seen), go(file, *arg, seen))
            }
            zutai_thir::TypeKind::ForAll { params, body, .. } => format!(
                "<{}> {}",
                params
                    .iter()
                    .map(|binding| file
                        .binding_names
                        .get(binding.0 as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("T{}", binding.0)))
                    .collect::<Vec<_>>()
                    .join(", "),
                go(file, *body, seen)
            ),
            zutai_thir::TypeKind::Error => "<type error>".to_string(),
        };
        seen.pop();
        result
    }
    go(file, id, &mut Vec::new())
}
