//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

impl Server {
    pub(super) fn hover(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(offset) = params
            .get("position")
            .and_then(|position| offset_at(&source, position))
        else {
            return Value::Null;
        };
        let Some(project) = self.analyze_with_overlays(uri, &source) else {
            return Value::Null;
        };
        let Some(file) = project
            .analysis
            .thir
            .as_ref()
            .and_then(|lowered| lowered.file.as_ref())
        else {
            return Value::Null;
        };
        let expr = file
            .expr_arena
            .iter()
            .filter(|(_, expr)| contains(expr.span, offset))
            .min_by_key(|(_, expr)| expr.span.end.saturating_sub(expr.span.start))
            .map(|(_, expr)| expr);
        let Some(expr) = expr else {
            return Value::Null;
        };
        let contents = format!("```zutai\n{}\n```", render_type(file, expr.ty));
        json!({
            "contents": { "kind": "markdown", "value": contents },
            "range": range(&source, expr.span.start as usize, expr.span.end as usize),
        })
    }

    pub(super) fn definition(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(offset) = params
            .get("position")
            .and_then(|position| offset_at(&source, position))
        else {
            return Value::Null;
        };
        let Some(project) = self.analyze_with_overlays(uri, &source) else {
            return Value::Null;
        };

        if let Some((module, path, member)) = self.imported_member_target(&source, offset, &project)
        {
            let target_uri = self.uri_for_path(&path);
            let Some(target_source) = self.source_for(&target_uri) else {
                return Value::Null;
            };
            let Some((start, end)) = exported_member_range(&target_source, module, &member) else {
                return Value::Null;
            };
            return json!({
                "uri": target_uri,
                "range": range(&target_source, start, end),
            });
        }

        let Some(hir) = project.analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Null;
        };
        let Some(binding) =
            binding_at(hir, offset).or_else(|| binding_declaration_at(&source, hir, offset))
        else {
            return Value::Null;
        };
        let Some(binding) = hir.bindings.get(binding.0 as usize) else {
            return Value::Null;
        };
        let Some((start, end)) = binding_range(&source, binding) else {
            return Value::Null;
        };
        json!({ "uri": uri, "range": range(&source, start, end) })
    }

    pub(super) fn references(&self, params: &Value) -> Value {
        let Some(position) = self.symbol_at_position(params) else {
            return Value::Null;
        };
        let include_declaration = params
            .pointer("/context/includeDeclaration")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Value::Array(
            self.symbol_references(&position.project, &position.target, include_declaration)
                .into_iter()
                .map(|(uri, source, start, end)| {
                    json!({ "uri": uri, "range": range(&source, start, end) })
                })
                .collect(),
        )
    }

    pub(super) fn document_symbols(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(project) = self.project_for_document(uri, &source) else {
            return Value::Null;
        };
        let requested = file_path(uri).map(|path| std::fs::canonicalize(&path).unwrap_or(path));
        let analysis = requested.as_ref().and_then(|requested| {
            project.modules().into_iter().find(|analysis| {
                project
                    .path_for(analysis)
                    .is_some_and(|path| std::fs::canonicalize(&path).unwrap_or(path) == *requested)
            })
        });
        let Some(analysis) = analysis else {
            return Value::Array(Vec::new());
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Array(Vec::new());
        };
        Value::Array(document_symbol_values(&source, hir))
    }

    pub(super) fn workspace_symbols(&self, params: &Value) -> Value {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let Ok(stdlib) = zutai_semantic::StdlibSources::load_configured(None) else {
            return Value::Array(Vec::new());
        };
        let mut roots: Vec<_> = self
            .documents
            .iter()
            .filter(|(uri, _)| {
                file_path(uri).and_then(|path| path.extension()?.to_str().map(str::to_owned))
                    == Some("zt".to_owned())
            })
            .map(|(uri, document)| (uri.clone(), document.text.clone()))
            .collect();
        roots.sort_by(|left, right| left.0.cmp(&right.0));
        let mut seen = HashSet::new();
        let mut symbols = Vec::new();
        for (uri, source) in roots {
            let Some(project) = self.analyze_with_overlays(&uri, &source) else {
                continue;
            };
            let public = project.public_modules(&stdlib);
            for (package, module, analysis) in &public {
                let Some(source_path) = analysis.source_path.as_ref() else {
                    continue;
                };
                if !seen.insert(source_path.clone()) {
                    continue;
                }
                let Some(path) = project.source_paths.get(source_path) else {
                    continue;
                };
                let module_uri = self.uri_for_path(path);
                let module_source = self
                    .source_for(&module_uri)
                    .or_else(|| project.recorded_source(analysis).map(str::to_owned));
                let Some(module_source) = module_source else {
                    continue;
                };
                append_workspace_symbols(
                    &mut symbols,
                    &query,
                    analysis,
                    &module_uri,
                    &module_source,
                    &format!("{package}.{module}"),
                );
            }
            for analysis in project.modules() {
                let identity = project.module_identity(analysis);
                if !seen.insert(identity) {
                    continue;
                }
                let Some((module_uri, module_source)) =
                    self.source_for_analysis(&project, analysis)
                else {
                    continue;
                };
                append_workspace_symbols(
                    &mut symbols,
                    &query,
                    analysis,
                    &module_uri,
                    &module_source,
                    &workspace_symbol_container(&project, analysis),
                );
            }
        }
        symbols.sort_by_key(workspace_symbol_key);
        Value::Array(symbols)
    }

    pub(super) fn completion(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(offset) = params
            .get("position")
            .and_then(|position| offset_at(&source, position))
        else {
            return Value::Null;
        };
        let Some(project) = self.project_for_document(uri, &source) else {
            return Value::Null;
        };
        let requested = file_path(uri).map(|path| std::fs::canonicalize(&path).unwrap_or(path));
        let analysis = requested.as_ref().and_then(|requested| {
            project.modules().into_iter().find(|analysis| {
                project
                    .path_for(analysis)
                    .is_some_and(|path| std::fs::canonicalize(&path).unwrap_or(path) == *requested)
            })
        });
        let (start, prefix) = completion_prefix(&source, offset);
        let replacement = range(&source, start, offset);

        if let Some(import_context) = import_completion_context(&source, offset) {
            let analysis = analysis.unwrap_or(&project.analysis);
            return completion_items(
                package_import_candidates(&project, analysis, &import_context.completed),
                &import_context.prefix,
                range(&source, import_context.start, offset),
            );
        }

        let Some(analysis) = analysis else {
            return Value::Array(Vec::new());
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Array(Vec::new());
        };
        if let Some(binding) = member_completion_binding(&source, hir, start) {
            let Some(import) = import_source_for_binding(hir, binding) else {
                return Value::Array(Vec::new());
            };
            let Some(target) = analysis.import_modules.get(&import) else {
                return Value::Array(Vec::new());
            };
            return completion_items(exported_member_candidates(target), &prefix, replacement);
        }

        let mut candidates: Vec<_> = hir
            .bindings
            .iter()
            .filter(|binding| completion_binding(binding, &source, offset))
            .map(|binding| CompletionCandidate {
                name: binding.name.clone(),
                kind: completion_kind(binding.kind),
                detail: binding_kind_label(binding.kind).to_owned(),
            })
            .collect();
        candidates.extend(KEYWORDS.iter().map(|keyword| CompletionCandidate {
            name: (*keyword).to_owned(),
            kind: 14,
            detail: "keyword".to_owned(),
        }));
        completion_items(candidates, &prefix, replacement)
    }

    pub(super) fn prepare_rename(&self, params: &Value) -> Value {
        let Some(position) = self.symbol_at_position(params) else {
            return Value::Null;
        };
        if !self.renameable_symbol(&position.project, &position.target) {
            return Value::Null;
        }
        range(&position.source, position.selection.0, position.selection.1)
    }

    pub(super) fn rename(&self, params: &Value) -> Value {
        let Some(new_name) = params.get("newName").and_then(Value::as_str) else {
            return Value::Null;
        };
        if !valid_identifier(new_name) {
            return Value::Null;
        }
        let Some(position) = self.symbol_at_position(params) else {
            return Value::Null;
        };
        if !self.renameable_symbol(&position.project, &position.target) {
            return Value::Null;
        }
        let mut changes = serde_json::Map::new();
        for (uri, source, start, end) in
            self.symbol_references(&position.project, &position.target, true)
        {
            let edits = changes
                .entry(uri)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("rename change entry is an array");
            edits.push(json!({ "range": range(&source, start, end), "newText": new_name }));
        }
        json!({ "changes": changes })
    }

    pub(super) fn signature_help(&self, params: &Value) -> Value {
        let Some((_, source, project, binding)) = self.binding_at_position(params) else {
            return Value::Null;
        };
        let analysis = &project.analysis;
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Null;
        };
        let Some(binding_data) = hir.bindings.get(binding.0 as usize) else {
            return Value::Null;
        };
        let Some(file) = analysis
            .thir
            .as_ref()
            .and_then(|lowered| lowered.file.as_ref())
        else {
            return Value::Null;
        };
        let ty = file
            .expr_arena
            .iter()
            .filter_map(|(_, expr)| match expr.kind {
                zutai_thir::ThirExprKind::BindingRef {
                    binding: candidate, ..
                } if candidate == binding
                    && source.get(expr.span.start as usize..expr.span.end as usize)
                        == Some(binding_data.name.as_str()) =>
                {
                    Some((expr.ty, expr.span))
                }
                _ => None,
            })
            .min_by_key(|(_, span)| span.end.saturating_sub(span.start))
            .map(|(ty, _)| ty);
        let Some(ty) = ty else {
            return Value::Null;
        };
        json!({
            "signatures": [{
                "label": format!("{} : {}", binding_data.name, render_type(file, ty)),
                "documentation": { "kind": "markdown", "value": binding_kind_label(binding_data.kind) }
            }],
            "activeSignature": 0,
            "activeParameter": 0,
        })
    }

    pub(super) fn code_actions(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(project) = self.analyze_with_overlays(uri, &source) else {
            return Value::Null;
        };
        Value::Array(
            project
                .analysis
                .diagnostics
                .iter()
                .filter_map(|diagnostic| match &diagnostic.kind {
                    zutai_semantic::SemanticDiagnosticKind::Parse(parse) => Some(parse),
                    _ => None,
                })
                .flat_map(|diagnostic| {
                    diagnostic.fixes.iter().map(|fix| {
                        let edits: Vec<_> = fix
                            .edits
                            .iter()
                            .map(|edit| {
                                json!({
                                    "range": range(&source, edit.span.start as usize, edit.span.end as usize),
                                    "newText": edit.replacement,
                                })
                            })
                            .collect();
                        json!({
                            "title": fix.title,
                            "kind": "quickfix",
                            "isPreferred": matches!(fix.applicability, zutai_syntax::Applicability::MachineApplicable),
                            "edit": { "changes": { uri: edits } },
                        })
                    })
                })
                .collect(),
        )
    }

    pub(super) fn formatting(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(path) = file_path(uri) else {
            return Value::Null;
        };
        let formatted = match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("zt") => zutai_syntax::format_source(&source).ok(),
            Some("zti") => zutai_im::format_source(&source).ok(),
            _ => None,
        };
        let Some(formatted) = formatted else {
            return Value::Null;
        };
        if formatted == source {
            return Value::Array(Vec::new());
        }
        json!([{
            "range": range(&source, 0, source.len()),
            "newText": formatted,
        }])
    }

    pub(super) fn imported_member_target<'a>(
        &self,
        source: &str,
        offset: usize,
        project: &'a ProjectAnalysis,
    ) -> Option<(&'a zutai_semantic::Analysis, PathBuf, String)> {
        let hir = project.analysis.hir.as_ref().map(|lowered| &lowered.file)?;
        let (binding, member) = imported_member_at(source, hir, offset)?;
        let import = import_source_for_binding(hir, binding)?;
        let module = project.analysis.import_modules.get(&import)?;
        let path = project.path_for(module)?;
        Some((module.as_ref(), path, member))
    }

    pub(super) fn symbol_at_position(&self, params: &Value) -> Option<SymbolPosition> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?;
        let source = self.source_for(uri)?;
        let offset = offset_at(&source, params.get("position")?)?;
        let requested = file_path(uri)?;
        let requested = std::fs::canonicalize(&requested).unwrap_or(requested);
        let project = self.project_for_document(uri, &source)?;
        let analysis = project.modules().into_iter().find(|analysis| {
            project
                .path_for(analysis)
                .is_some_and(|path| std::fs::canonicalize(&path).unwrap_or(path) == requested)
        })?;
        let module = project.module_identity(analysis);
        let hir = analysis.hir.as_ref().map(|lowered| &lowered.file)?;

        if let Some((import_binding, member)) = imported_member_at(&source, hir, offset) {
            let selection = imported_member_selection(&source, hir, offset)?;
            let import = import_source_for_binding(hir, import_binding)?;
            let target = analysis.import_modules.get(&import)?;
            return Some(SymbolPosition {
                target: SymbolTarget::ExportedMember {
                    module: project.module_identity(target),
                    member,
                },
                project,
                selection,
                source,
            });
        }

        let binding =
            binding_at(hir, offset).or_else(|| binding_declaration_at(&source, hir, offset))?;
        let binding_data = hir.bindings.get(binding.0 as usize)?;
        let selection = binding_range(&source, binding_data)?;
        Some(SymbolPosition {
            target: SymbolTarget::Binding { module, binding },
            project,
            selection,
            source,
        })
    }

    pub(super) fn binding_at_position(
        &self,
        params: &Value,
    ) -> Option<(String, String, ProjectAnalysis, zutai_hir::BindingId)> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?.to_owned();
        let source = self.source_for(&uri)?;
        let offset = offset_at(&source, params.get("position")?)?;
        let project = self.analyze_with_overlays(&uri, &source)?;
        let hir = project.analysis.hir.as_ref().map(|lowered| &lowered.file)?;
        let binding =
            binding_at(hir, offset).or_else(|| binding_declaration_at(&source, hir, offset))?;
        Some((uri, source, project, binding))
    }

    pub(super) fn renameable_symbol(
        &self,
        project: &ProjectAnalysis,
        target: &SymbolTarget,
    ) -> bool {
        let (analysis, binding) = match target {
            SymbolTarget::Binding { module, binding } => {
                let Some(analysis) = project.module(module) else {
                    return false;
                };
                (analysis, Some(*binding))
            }
            SymbolTarget::ExportedMember { module, member } => {
                let Some(analysis) = project.module(module) else {
                    return false;
                };
                let Some(source) = self
                    .source_for_analysis(project, analysis)
                    .map(|(_, source)| source)
                else {
                    return false;
                };
                let binding = exported_member_local_binding(analysis, member);
                if exported_field_range(&source, analysis, member).is_none() {
                    return false;
                }
                (analysis, binding)
            }
        };
        if !project.writable(analysis) {
            return false;
        }
        if let Some(binding) = binding {
            let Some(source) = self
                .source_for_analysis(project, analysis)
                .map(|(_, source)| source)
            else {
                return false;
            };
            let Some(binding) = analysis
                .hir
                .as_ref()
                .and_then(|lowered| lowered.file.bindings.get(binding.0 as usize))
            else {
                return false;
            };
            if !renameable_binding(&source, binding) {
                return false;
            }
        }
        self.symbol_reference_locations(project, target, true)
            .into_iter()
            .all(|(module, _, _, _, _)| {
                project
                    .module(&module)
                    .is_some_and(|analysis| project.writable(analysis))
            })
    }

    pub(super) fn symbol_references(
        &self,
        project: &ProjectAnalysis,
        target: &SymbolTarget,
        include_declaration: bool,
    ) -> Vec<(String, String, usize, usize)> {
        self.symbol_reference_locations(project, target, include_declaration)
            .into_iter()
            .map(|(_, uri, source, start, end)| (uri, source, start, end))
            .collect()
    }

    pub(super) fn symbol_reference_locations(
        &self,
        project: &ProjectAnalysis,
        target: &SymbolTarget,
        include_declaration: bool,
    ) -> Vec<(PathBuf, String, String, usize, usize)> {
        let (target_module, member, binding) = match target {
            SymbolTarget::Binding { module, binding } => {
                let Some(analysis) = project.module(module) else {
                    return Vec::new();
                };
                (
                    module.clone(),
                    exported_member_for_binding(analysis, *binding),
                    Some(*binding),
                )
            }
            SymbolTarget::ExportedMember { module, member } => {
                let Some(analysis) = project.module(module) else {
                    return Vec::new();
                };
                let (module, member) = resolve_exported_member_target(project, analysis, member);
                let binding = project
                    .module(&module)
                    .and_then(|analysis| exported_member_local_binding(analysis, &member));
                (module, Some(member), binding)
            }
        };

        let mut locations = Vec::new();
        if let Some(analysis) = project.module(&target_module)
            && let Some((uri, source)) = self.source_for_analysis(project, analysis)
            && let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file)
        {
            if let Some(binding) = binding {
                locations.extend(
                    binding_reference_ranges(&source, hir, binding, include_declaration)
                        .into_iter()
                        .map(|(start, end)| {
                            (
                                target_module.clone(),
                                uri.clone(),
                                source.clone(),
                                start,
                                end,
                            )
                        }),
                );
            }
            if include_declaration
                && let Some(member) = member.as_deref()
                && let Some((start, end)) = exported_field_range(&source, analysis, member)
            {
                locations.push((target_module.clone(), uri, source, start, end));
            }
        }

        if let Some(member) = member.as_deref() {
            for analysis in project.modules() {
                let module = project.module_identity(analysis);
                let Some((uri, source)) = self.source_for_analysis(project, analysis) else {
                    continue;
                };
                locations.extend(
                    imported_member_reference_ranges(
                        &source,
                        analysis,
                        project,
                        &target_module,
                        member,
                    )
                    .into_iter()
                    .map(|(start, end)| (module.clone(), uri.clone(), source.clone(), start, end)),
                );
                if include_declaration
                    && module != target_module
                    && let Some((origin, origin_member)) =
                        exported_member_origin(project, analysis, member)
                    && origin == target_module
                    && origin_member == member
                    && let Some((start, end)) = exported_field_range(&source, analysis, member)
                {
                    locations.push((module, uri, source, start, end));
                }
            }
        }

        locations
            .sort_by(|left, right| (&left.1, left.3, left.4).cmp(&(&right.1, right.3, right.4)));
        locations
            .dedup_by(|left, right| left.1 == right.1 && left.3 == right.3 && left.4 == right.4);
        locations
    }
}
