//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::diagnostics::format_import_diagnostic;

pub(crate) fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut server = Server::default();

    while let Some(message) = read_message(&mut input)? {
        let should_exit = server.handle(message, &mut output)?;
        if should_exit {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
    published_diagnostics: HashSet<String>,
}

#[derive(Clone)]
struct Document {
    text: String,
    version: Option<i64>,
}

impl Server {
    fn handle(&mut self, message: Value, output: &mut impl Write) -> io::Result<bool> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                if let Some(id) = id {
                    send(
                        output,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "capabilities": {
                                    "positionEncoding": "utf-16",
                                    "textDocumentSync": {
                                        "openClose": true,
                                        "change": 2,
                                        "save": { "includeText": false }
                                    },
                                    "hoverProvider": true,
                                    "definitionProvider": true,
                                    "referencesProvider": true,
                                    "documentSymbolProvider": true,
                                    "completionProvider": {
                                        "triggerCharacters": [".", ":"],
                                        "resolveProvider": false
                                    },
                                    "renameProvider": { "prepareProvider": true },
                                    "signatureHelpProvider": {
                                        "triggerCharacters": ["(", " "]
                                    },
                                    "codeActionProvider": {
                                        "codeActionKinds": ["quickfix"]
                                    }
                                },
                                "serverInfo": { "name": "zutai", "version": env!("CARGO_PKG_VERSION") }
                            }
                        }),
                    )?;
                }
            }
            "shutdown" => {
                if let Some(id) = id {
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": null }),
                    )?;
                }
            }
            "exit" => return Ok(true),
            "textDocument/didOpen" => {
                if let Some((uri, document)) = document_text(&params) {
                    self.documents.insert(uri.clone(), document);
                    self.publish_all_diagnostics(output)?;
                }
            }
            "textDocument/didChange" => {
                if self.apply_changes(&params).is_some() {
                    self.publish_all_diagnostics(output)?;
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    self.documents.remove(uri);
                    self.publish_all_diagnostics(output)?;
                }
            }
            "textDocument/hover" => {
                if let Some(id) = id {
                    let result = self.hover(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/definition" => {
                if let Some(id) = id {
                    let result = self.definition(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/references" => {
                if let Some(id) = id {
                    let result = self.references(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/documentSymbol" => {
                if let Some(id) = id {
                    let result = self.document_symbols(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/completion" => {
                if let Some(id) = id {
                    let result = self.completion(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/prepareRename" => {
                if let Some(id) = id {
                    let result = self.prepare_rename(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/rename" => {
                if let Some(id) = id {
                    let result = self.rename(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/signatureHelp" => {
                if let Some(id) = id {
                    let result = self.signature_help(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            "textDocument/codeAction" => {
                if let Some(id) = id {
                    let result = self.code_actions(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            _ => {
                if let Some(id) = id {
                    send(
                        output,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": format!("method not found: {method}") }
                        }),
                    )?;
                }
            }
        }
        Ok(false)
    }

    fn publish_all_diagnostics(&mut self, output: &mut impl Write) -> io::Result<()> {
        let roots: Vec<String> = self
            .documents
            .keys()
            .filter(|uri| {
                file_path(uri).and_then(|path| path.extension()?.to_str().map(str::to_owned))
                    == Some("zt".to_string())
            })
            .cloned()
            .collect();
        let mut routed: HashMap<String, Vec<Value>> = HashMap::new();
        for root_uri in roots {
            let Some(root_source) = self.source_for(&root_uri) else {
                continue;
            };
            let Some(analysis) = self.analyze_with_overlays(&root_uri, &root_source) else {
                continue;
            };
            for (uri, diagnostic) in self.routed_diagnostics(&root_uri, &root_source, &analysis) {
                routed.entry(uri).or_default().push(diagnostic);
            }
        }

        let routed_targets: HashSet<String> = routed.keys().cloned().collect();
        let mut targets: HashSet<String> = self.published_diagnostics.clone();
        targets.extend(self.documents.keys().cloned());
        targets.extend(routed.keys().cloned());
        for uri in &targets {
            publish(
                output,
                uri,
                self.documents
                    .get(uri)
                    .and_then(|document| document.version),
                routed.remove(uri).unwrap_or_default(),
            )?;
        }
        self.published_diagnostics = targets
            .into_iter()
            .filter(|uri| self.documents.contains_key(uri) || routed_targets.contains(uri))
            .collect();
        Ok(())
    }

    fn analyze_with_overlays(
        &self,
        root_uri: &str,
        root_source: &str,
    ) -> Option<zutai_semantic::Analysis> {
        let root_path = file_path(root_uri)?;
        let root_dir = root_path.parent()?;
        let Some(mut recorded) = zutai_semantic::analyze_path_recording(&root_path).ok() else {
            return analyze(root_source, root_uri);
        };
        for (uri, document) in &self.documents {
            let Some(path) = file_path(uri) else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(root_dir) else {
                continue;
            };
            let key = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            recorded.sources.insert(key, document.text.clone());
        }
        recorded
            .sources
            .insert(recorded.entry.clone(), root_source.to_string());
        zutai_semantic::analyze_sources(
            &recorded.entry,
            &recorded.sources,
            zutai_semantic::AnalysisOptions::default(),
        )
        .ok()
    }

    fn routed_diagnostics(
        &self,
        root_uri: &str,
        root_source: &str,
        analysis: &zutai_semantic::Analysis,
    ) -> Vec<(String, Value)> {
        let mut output = Vec::new();
        for diagnostic in &analysis.diagnostics {
            if let zutai_semantic::SemanticDiagnosticKind::Thir(thir) = &diagnostic.kind
                && let zutai_thir::ThirDiagnosticKind::ImportedDataTypeMismatch {
                    expected,
                    found,
                    origin,
                } = &thir.kind
                && let zutai_hir::HirImportSource::String(relative) = &origin.source
                && let Some(root_path) = file_path(root_uri)
            {
                let path = root_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .join(relative);
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                let uri = self.uri_for_path(&path);
                if let Some(source) = self.source_for(&uri) {
                    output.push((
                        uri.clone(),
                        json!({
                            "range": range(&source, origin.span.start as usize, origin.span.end as usize),
                            "severity": 1,
                            "source": "zutai",
                            "message": format!("type mismatch: expected {expected}, found {found}"),
                            "relatedInformation": [{
                                "location": {
                                    "uri": root_uri,
                                    "range": range(root_source, thir.span.start as usize, thir.span.end as usize),
                                },
                                "message": "required by this typed boundary",
                            }],
                        }),
                    ));
                    continue;
                }
            }
            output.push((
                root_uri.to_string(),
                diagnostic_value(root_source, diagnostic),
            ));
        }
        output
    }

    fn uri_for_path(&self, path: &std::path::Path) -> String {
        self.documents
            .keys()
            .find(|uri| file_path(uri).as_deref() == Some(path))
            .cloned()
            .unwrap_or_else(|| file_uri(path))
    }

    fn apply_changes(&mut self, params: &Value) -> Option<String> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?.to_owned();
        let version = params
            .pointer("/textDocument/version")
            .and_then(Value::as_i64);
        let document = self.documents.get_mut(&uri)?;
        for change in params.get("contentChanges")?.as_array()? {
            let text = change.get("text")?.as_str()?;
            if let Some(range) = change.get("range") {
                let start = offset_at(&document.text, range.get("start")?)?;
                let end = offset_at(&document.text, range.get("end")?)?;
                if start > end {
                    return None;
                }
                document.text.replace_range(start..end, text);
            } else {
                document.text = text.to_owned();
            }
        }
        document.version = version;
        Some(uri)
    }

    fn hover(&self, params: &Value) -> Value {
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
        let Some(analysis) = analyze(&source, uri) else {
            return Value::Null;
        };
        let Some(file) = analysis.thir.and_then(|lowered| lowered.file) else {
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
        let contents = format!("```zutai\n{}\n```", render_type(&file, expr.ty));
        json!({
            "contents": { "kind": "markdown", "value": contents },
            "range": range(&source, expr.span.start as usize, expr.span.end as usize),
        })
    }

    fn definition(&self, params: &Value) -> Value {
        let Some((uri, source, analysis, binding)) = self.binding_at_position(params) else {
            return Value::Null;
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
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

    fn references(&self, params: &Value) -> Value {
        let Some((uri, source, analysis, binding)) = self.binding_at_position(params) else {
            return Value::Null;
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Null;
        };
        let Some(binding_data) = hir.bindings.get(binding.0 as usize) else {
            return Value::Null;
        };
        if binding_range(&source, binding_data).is_none() {
            return Value::Null;
        }
        let include_declaration = params
            .pointer("/context/includeDeclaration")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Value::Array(
            binding_reference_ranges(&source, hir, binding, include_declaration)
                .into_iter()
                .map(|(start, end)| json!({ "uri": uri, "range": range(&source, start, end) }))
                .collect(),
        )
    }

    fn document_symbols(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(analysis) = analyze(&source, uri) else {
            return Value::Null;
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Array(Vec::new());
        };
        Value::Array(
            hir.decl_arena
                .iter()
                .filter_map(|(_, decl)| {
                    let binding = hir.bindings.get(decl.binding.0 as usize)?;
                    let (start, end) = binding_range(&source, binding)?;
                    Some(json!({
                        "name": binding.name,
                        "detail": binding_kind_label(binding.kind),
                        "kind": symbol_kind(binding.kind),
                        "range": range(&source, decl.span.start as usize, decl.span.end as usize),
                        "selectionRange": range(&source, start, end),
                    }))
                })
                .collect(),
        )
    }

    fn completion(&self, params: &Value) -> Value {
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
        let Some(analysis) = analyze(&source, uri) else {
            return Value::Null;
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Array(Vec::new());
        };
        let (start, prefix) = completion_prefix(&source, offset);
        let replacement = range(&source, start, offset);
        let mut candidates: Vec<_> = hir
            .bindings
            .iter()
            .filter(|binding| completion_binding(binding, &source, offset))
            .map(|binding| (binding.name.clone(), binding.kind))
            .collect();
        candidates.extend(
            KEYWORDS
                .iter()
                .map(|keyword| ((*keyword).to_owned(), zutai_hir::BindingKind::BuiltinValue)),
        );
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.dedup_by(|left, right| left.0 == right.0);
        Value::Array(
            candidates
                .into_iter()
                .filter(|(name, _)| name.starts_with(&prefix))
                .map(|(name, kind)| {
                    let is_keyword = KEYWORDS.contains(&name.as_str());
                    json!({
                        "label": name,
                        "kind": if is_keyword { 14 } else { completion_kind(kind) },
                        "detail": if is_keyword { "keyword" } else { binding_kind_label(kind) },
                        "sortText": name,
                        "textEdit": { "range": replacement, "newText": name },
                    })
                })
                .collect(),
        )
    }

    fn prepare_rename(&self, params: &Value) -> Value {
        let Some((_, source, analysis, binding)) = self.binding_at_position(params) else {
            return Value::Null;
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Null;
        };
        let Some(binding) = hir.bindings.get(binding.0 as usize) else {
            return Value::Null;
        };
        if !renameable_binding(&source, binding) {
            return Value::Null;
        }
        let (start, end) = binding_range(&source, binding).expect("renameable binding has a range");
        range(&source, start, end)
    }

    fn rename(&self, params: &Value) -> Value {
        let Some(new_name) = params.get("newName").and_then(Value::as_str) else {
            return Value::Null;
        };
        if !valid_identifier(new_name) {
            return Value::Null;
        }
        let Some((uri, source, analysis, binding)) = self.binding_at_position(params) else {
            return Value::Null;
        };
        let Some(hir) = analysis.hir.as_ref().map(|lowered| &lowered.file) else {
            return Value::Null;
        };
        let Some(binding_data) = hir.bindings.get(binding.0 as usize) else {
            return Value::Null;
        };
        if !renameable_binding(&source, binding_data) {
            return Value::Null;
        }
        let edits: Vec<_> = binding_reference_ranges(&source, hir, binding, true)
            .into_iter()
            .map(|(start, end)| json!({ "range": range(&source, start, end), "newText": new_name }))
            .collect();
        json!({ "changes": { uri: edits } })
    }

    fn signature_help(&self, params: &Value) -> Value {
        let Some((_, source, analysis, binding)) = self.binding_at_position(params) else {
            return Value::Null;
        };
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

    fn code_actions(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(analysis) = analyze(&source, uri) else {
            return Value::Null;
        };
        Value::Array(
            analysis
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

    fn binding_at_position(
        &self,
        params: &Value,
    ) -> Option<(
        String,
        String,
        zutai_semantic::Analysis,
        zutai_hir::BindingId,
    )> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?.to_owned();
        let source = self.source_for(&uri)?;
        let offset = offset_at(&source, params.get("position")?)?;
        let analysis = analyze(&source, &uri)?;
        let hir = analysis.hir.as_ref().map(|lowered| &lowered.file)?;
        let binding =
            binding_at(hir, offset).or_else(|| binding_declaration_at(&source, hir, offset))?;
        Some((uri, source, analysis, binding))
    }

    fn source_for(&self, uri: &str) -> Option<String> {
        self.documents
            .get(uri)
            .map(|document| document.text.clone())
            .or_else(|| file_path(uri).and_then(|path| std::fs::read_to_string(path).ok()))
    }
}

fn document_text(params: &Value) -> Option<(String, Document)> {
    let document = params.get("textDocument")?;
    Some((
        document.get("uri")?.as_str()?.to_owned(),
        Document {
            text: document.get("text")?.as_str()?.to_owned(),
            version: document.get("version").and_then(Value::as_i64),
        },
    ))
}

fn analyze(source: &str, uri: &str) -> Option<zutai_semantic::Analysis> {
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
fn diagnostics(source: &str, analysis: &zutai_semantic::Analysis) -> Vec<Value> {
    analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic_value(source, diagnostic))
        .collect()
}

fn diagnostic_value(source: &str, diagnostic: &zutai_semantic::SemanticDiagnostic) -> Value {
    match &diagnostic.kind {
        zutai_semantic::SemanticDiagnosticKind::Parse(parse) => json!({
            "range": range(source, parse.primary_span().start as usize, parse.primary_span().end as usize),
            "severity": severity(parse.severity),
            "code": parse.code,
            "source": "zutai",
            "message": parse.message,
        }),
        zutai_semantic::SemanticDiagnosticKind::Import(import) => json!({
            "range": range(source, 0, 0),
            "severity": 1,
            "source": "zutai",
            "message": format_import_diagnostic(import),
        }),
        _ => {
            let (message, start, end) = zutai_eval::describe_semantic_diagnostic(diagnostic)
                .expect("HIR and THIR diagnostics always have a source span");
            json!({
                "range": range(source, start as usize, end as usize),
                "severity": 1,
                "source": "zutai",
                "message": message,
            })
        }
    }
}

/// Find the narrowest resolved value or type reference at an LSP byte offset.
/// This deliberately consults HIR rather than THIR: name resolution survives
/// later type errors, so definition navigation remains useful while editing an
/// incomplete program.
fn binding_at(file: &zutai_hir::HirFile, offset: usize) -> Option<zutai_hir::BindingId> {
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
fn binding_range(source: &str, binding: &zutai_hir::Binding) -> Option<(usize, usize)> {
    let start = binding.span.start as usize;
    let end = start.checked_add(binding.name.len())?;
    (source.get(start..end) == Some(binding.name.as_str())).then_some((start, end))
}

fn binding_declaration_at(
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

fn binding_reference_ranges(
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

const KEYWORDS: &[&str] = &[
    "cond", "false", "handle", "if", "import", "match", "perform", "resume", "select", "then",
    "true", "type", "with",
];

fn completion_prefix(source: &str, offset: usize) -> (usize, String) {
    let mut start = floor_boundary(source, offset.min(source.len()));
    while let Some(character) = source[..start].chars().next_back() {
        if !zutai_syntax::ident::is_ident_continue(character) {
            break;
        }
        start -= character.len_utf8();
    }
    let prefix = &source[start..offset];
    if prefix
        .chars()
        .next()
        .is_some_and(zutai_syntax::ident::is_ident_start)
    {
        (start, prefix.to_owned())
    } else {
        (offset, String::new())
    }
}

fn completion_binding(binding: &zutai_hir::Binding, source: &str, offset: usize) -> bool {
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

fn binding_kind_label(kind: zutai_hir::BindingKind) -> &'static str {
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

fn completion_kind(kind: zutai_hir::BindingKind) -> u8 {
    match kind {
        zutai_hir::BindingKind::TopFunction | zutai_hir::BindingKind::ConstraintMethod => 3,
        zutai_hir::BindingKind::TopType | zutai_hir::BindingKind::BuiltinType => 7,
        zutai_hir::BindingKind::TopConstraint => 8,
        zutai_hir::BindingKind::TypeParam | zutai_hir::BindingKind::LevelParam => 25,
        zutai_hir::BindingKind::Param => 6,
        _ => 6,
    }
}

fn symbol_kind(kind: zutai_hir::BindingKind) -> u8 {
    match kind {
        zutai_hir::BindingKind::TopFunction => 12,
        zutai_hir::BindingKind::TopType => 5,
        zutai_hir::BindingKind::TopConstraint => 11,
        zutai_hir::BindingKind::TopWitness => 14,
        _ => 13,
    }
}

fn renameable_binding(source: &str, binding: &zutai_hir::Binding) -> bool {
    !matches!(
        binding.kind,
        zutai_hir::BindingKind::BuiltinType | zutai_hir::BindingKind::BuiltinValue
    ) && !binding.name.starts_with('$')
        && binding_range(source, binding).is_some()
}

fn valid_identifier(name: &str) -> bool {
    let tokens = zutai_syntax::tokenize(name);
    matches!(tokens.as_slice(), [token] if token.kind == zutai_syntax::SyntaxKind::Ident && token.text == name)
}

fn severity(severity: zutai_syntax::Severity) -> u8 {
    match severity {
        zutai_syntax::Severity::Error => 1,
        zutai_syntax::Severity::Warning => 2,
        zutai_syntax::Severity::Info => 3,
        zutai_syntax::Severity::Hint => 4,
    }
}

fn render_type(file: &zutai_thir::ThirFile, id: zutai_thir::TypeId) -> String {
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

fn contains(span: zutai_syntax::Span, offset: usize) -> bool {
    (span.start as usize) <= offset && offset <= span.end as usize
}

fn range(source: &str, start: usize, end: usize) -> Value {
    json!({ "start": position_at(source, start), "end": position_at(source, end) })
}

fn position_at(source: &str, offset: usize) -> Value {
    let offset = floor_boundary(source, offset.min(source.len()));
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let character = source[line_start..offset].encode_utf16().count();
    json!({ "line": line, "character": character })
}

fn offset_at(source: &str, position: &Value) -> Option<usize> {
    let line = position.get("line")?.as_u64()? as usize;
    let character = position.get("character")?.as_u64()? as usize;
    let line_start = if line == 0 {
        0
    } else {
        source
            .match_indices('\n')
            .nth(line - 1)
            .map(|(index, _)| index + 1)?
    };
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |index| line_start + index);
    let mut utf16 = 0;
    for (index, ch) in source[line_start..line_end].char_indices() {
        if utf16 >= character {
            return Some(line_start + index);
        }
        utf16 += ch.len_utf16();
        if utf16 >= character {
            return Some(line_start + index + ch.len_utf8());
        }
    }
    Some(line_end)
}

fn floor_boundary(source: &str, mut offset: usize) -> usize {
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn file_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let path = path.strip_prefix("localhost").unwrap_or(path);
    Some(PathBuf::from(percent_decode(path)))
}

fn file_uri(path: &std::path::Path) -> String {
    let mut encoded = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

fn percent_decode(input: &str) -> String {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            output.push(high * 16 + low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn publish(
    output: &mut impl Write,
    uri: &str,
    version: Option<i64>,
    diagnostics: Vec<Value>,
) -> io::Result<()> {
    send(
        output,
        json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": uri, "version": version, "diagnostics": diagnostics } }),
    )
}

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(content_length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };
    let mut body = vec![0; content_length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn send(output: &mut impl Write, message: Value) -> io::Result<()> {
    let body = serde_json::to_vec(&message).expect("JSON-RPC messages are serializable");
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_use_utf16_code_units() {
        let source = "x😀\n終";
        assert_eq!(position_at(source, 5), json!({ "line": 0, "character": 3 }));
        assert_eq!(
            offset_at(source, &json!({ "line": 0, "character": 3 })),
            Some(5)
        );
    }

    #[test]
    fn parse_hover_and_publish_diagnostics() {
        let uri = "file:///tmp/example.zt";
        let mut server = Server::default();
        let mut output = Vec::new();
        let open = json!({ "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "text": "x ::= 1;\nx" } } });
        server.handle(open, &mut output).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("publishDiagnostics"));

        let hover = server.hover(
            &json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 0 } }),
        );
        assert_eq!(
            hover.pointer("/contents/value").and_then(Value::as_str),
            Some("```zutai\nInt\n```")
        );
    }

    #[test]
    fn definition_resolves_value_and_type_bindings_with_utf16_ranges() {
        let uri = "file:///tmp/definition.zt";
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document {
                text: "名 ::= 1;\nCount :: type Int;\nvalue :: Count = 名;\nvalue".to_string(),
                version: None,
            },
        );

        let value = server.definition(
            &json!({ "textDocument": { "uri": uri }, "position": { "line": 3, "character": 1 } }),
        );
        assert_eq!(
            value,
            json!({
                "uri": uri,
                "range": {
                    "start": { "line": 2, "character": 0 },
                    "end": { "line": 2, "character": 5 }
                }
            })
        );

        let ty = server.definition(
            &json!({ "textDocument": { "uri": uri }, "position": { "line": 2, "character": 10 } }),
        );
        assert_eq!(
            ty.pointer("/range/start/line").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            ty.pointer("/range/end/character").and_then(Value::as_u64),
            Some(5)
        );

        let unicode = server.definition(
            &json!({ "textDocument": { "uri": uri }, "position": { "line": 2, "character": 17 } }),
        );
        assert_eq!(
            unicode.pointer("/range/start").cloned(),
            Some(json!({ "line": 0, "character": 0 }))
        );
        assert_eq!(
            unicode.pointer("/range/end").cloned(),
            Some(json!({ "line": 0, "character": 1 }))
        );
    }

    #[test]
    fn definition_works_when_later_type_checking_fails() {
        let uri = "file:///tmp/incomplete.zt";
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document {
                text: "answer ::= 42;\nanswer + \"bad\"".to_string(),
                version: None,
            },
        );

        let result = server.definition(
            &json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 2 } }),
        );
        assert_eq!(
            result.pointer("/range/start").cloned(),
            Some(json!({ "line": 0, "character": 0 }))
        );
        assert_eq!(
            result.pointer("/range/end").cloned(),
            Some(json!({ "line": 0, "character": 6 }))
        );
    }

    #[test]
    fn initialize_advertises_definition_support() {
        let mut server = Server::default();
        let mut output = Vec::new();
        server
            .handle(json!({ "id": 1, "method": "initialize" }), &mut output)
            .unwrap();
        let message = String::from_utf8(output).unwrap();
        assert!(message.contains("definitionProvider"));
        assert!(message.contains("referencesProvider"));
        assert!(message.contains("renameProvider"));
        assert!(message.contains("completionProvider"));
        assert!(message.contains("codeActionProvider"));
    }

    #[test]
    fn incremental_changes_preserve_utf16_positions_and_diagnostic_versions() {
        let uri = "file:///tmp/change.zt";
        let mut server = Server::default();
        let mut output = Vec::new();
        server
            .handle(
                json!({
                    "method": "textDocument/didOpen",
                    "params": { "textDocument": { "uri": uri, "version": 1, "text": "名 ::= 1;\n名" } }
                }),
                &mut output,
            )
            .unwrap();
        output.clear();
        server
            .handle(
                json!({
                    "method": "textDocument/didChange",
                    "params": {
                        "textDocument": { "uri": uri, "version": 2 },
                        "contentChanges": [{
                            "range": {
                                "start": { "line": 1, "character": 0 },
                                "end": { "line": 1, "character": 1 }
                            },
                            "text": "名 + \"bad\""
                        }]
                    }
                }),
                &mut output,
            )
            .unwrap();

        assert_eq!(
            server.source_for(uri).as_deref(),
            Some("名 ::= 1;\n名 + \"bad\"")
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"version\":2"), "{output}");
        assert!(output.contains("publishDiagnostics"), "{output}");
    }

    #[test]
    fn references_and_rename_are_binding_accurate() {
        let uri = "file:///tmp/rename.zt";
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document {
                text: "value ::= 1;\nuse ::= value;\nvalue".to_string(),
                version: Some(4),
            },
        );
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 8 },
            "context": { "includeDeclaration": true }
        });
        let references = server.references(&params);
        assert_eq!(references.as_array().map(Vec::len), Some(3));
        assert_eq!(
            server
                .prepare_rename(&params)
                .pointer("/start/line")
                .and_then(Value::as_u64),
            Some(0)
        );

        let rename = server.rename(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 8 },
            "newName": "renamed"
        }));
        assert_eq!(
            rename
                .get("changes")
                .and_then(|changes| changes.get(uri))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(3)
        );
        assert_eq!(
            server.rename(&json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 8 },
                "newName": "match"
            })),
            Value::Null
        );
    }

    #[test]
    fn symbols_completion_and_signature_help_use_semantic_information() {
        let uri = "file:///tmp/features.zt";
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document {
                text: "id :: Int -> Int\n  = x => x;\nvalue ::= id 1;\nvalue".to_string(),
                version: None,
            },
        );

        let symbols = server.document_symbols(&json!({ "textDocument": { "uri": uri } }));
        let names: Vec<_> = symbols
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|symbol| symbol.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, ["id", "value"]);

        let completions = server.completion(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 3 }
        }));
        assert!(
            completions
                .as_array()
                .unwrap()
                .iter()
                .any(|item| { item.get("label").and_then(Value::as_str) == Some("value") })
        );

        let signature = server.signature_help(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 10 }
        }));
        assert_eq!(
            signature
                .pointer("/signatures/0/label")
                .and_then(Value::as_str),
            Some("id : Int -> Int")
        );
    }

    #[test]
    fn parser_fixes_are_published_as_quick_fixes() {
        let uri = "file:///tmp/fix.zt";
        let mut server = Server::default();
        server.documents.insert(
            uri.to_string(),
            Document {
                text: "value : Int = 1;\nvalue".to_string(),
                version: None,
            },
        );

        let actions = server.code_actions(&json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 16 }
            },
            "context": { "diagnostics": [] }
        }));
        assert_eq!(actions.as_array().map(Vec::len), Some(1));
        assert_eq!(
            actions.pointer("/0/title").and_then(Value::as_str),
            Some("Use `::` for typed binding")
        );
        assert_eq!(
            actions
                .pointer("/0/edit/changes/file:~1~1~1tmp~1fix.zt/0/newText")
                .and_then(Value::as_str),
            Some("::")
        );
    }

    #[test]
    fn parser_diagnostic_includes_protocol_range() {
        let analysis = analyze("x ::= ;\nx", "file:///tmp/bad.zt").unwrap();
        let diagnostics = diagnostics("x ::= ;\nx", &analysis);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].get("range").is_some());
    }

    #[test]
    fn imported_zti_mismatch_is_published_to_data_uri_and_cleared_from_overlay() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "zutai-lsp-imported-data-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let b_path = dir.join("B.zt");
        let a_path = dir.join("A.zti");
        let c_path = dir.join("C.zt");
        let b_source = "Config :: type { port : Int; };\n{ Config = Config; }\n";
        let bad_a = "{\n  port = \"wrong\";\n}\n";
        let good_a = "{\n  port = 8080;\n}\n";
        let c_source =
            "b ::= import \"B.zt\";\na ::= import \"A.zti\";\nchecked :: b.Config = a;\nchecked\n";
        std::fs::write(&b_path, b_source).unwrap();
        std::fs::write(&a_path, bad_a).unwrap();
        std::fs::write(&c_path, c_source).unwrap();
        let a_uri = format!("file://{}", a_path.display());
        let c_uri = format!("file://{}", c_path.display());

        let mut server = Server::default();
        let mut output = Vec::new();
        server
            .handle(
                json!({ "method": "textDocument/didOpen", "params": { "textDocument": {
                    "uri": c_uri, "version": 1, "text": c_source
                } } }),
                &mut output,
            )
            .unwrap();
        server
            .handle(
                json!({ "method": "textDocument/didOpen", "params": { "textDocument": {
                    "uri": a_uri, "version": 1, "text": bad_a
                } } }),
                &mut output,
            )
            .unwrap();
        let published = String::from_utf8_lossy(&output);
        assert!(published.contains(&a_uri), "{published}");
        assert!(
            published.contains("expected Int, found Text"),
            "{published}"
        );
        assert!(published.contains("relatedInformation"), "{published}");

        output.clear();
        server
            .handle(
                json!({ "method": "textDocument/didChange", "params": {
                    "textDocument": { "uri": a_uri, "version": 2 },
                    "contentChanges": [{ "text": good_a }]
                } }),
                &mut output,
            )
            .unwrap();
        let cleared = String::from_utf8_lossy(&output);
        assert!(cleared.contains(&a_uri), "{cleared}");
        assert!(cleared.contains("\"diagnostics\":[]"), "{cleared}");
    }

    #[test]
    fn framing_round_trip() {
        let input = b"Content-Length: 17\r\n\r\n{\"method\":\"ping\"}";
        assert_eq!(
            read_message(&mut &input[..]).unwrap(),
            Some(json!({ "method": "ping" }))
        );
    }

    #[test]
    fn file_uris_preserve_absolute_paths() {
        assert_eq!(
            file_path("file:///tmp/example.zt"),
            Some(PathBuf::from("/tmp/example.zt"))
        );
        assert_eq!(
            file_path("file://localhost/tmp/example.zt"),
            Some(PathBuf::from("/tmp/example.zt"))
        );
        let spaced = PathBuf::from("/tmp/Zutai data/A.zti");
        assert_eq!(file_uri(&spaced), "file:///tmp/Zutai%20data/A.zti");
        assert_eq!(file_path(&file_uri(&spaced)), Some(spaced));
    }
}
