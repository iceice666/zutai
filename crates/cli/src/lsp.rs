//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

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
    analysis_cache: zutai_semantic::AnalysisCache,
}

#[derive(Clone)]
struct Document {
    text: String,
    version: Option<i64>,
}

struct ProjectAnalysis {
    analysis: zutai_semantic::Analysis,
    root_path: PathBuf,
    source_paths: std::collections::BTreeMap<PathBuf, PathBuf>,
    sources: std::collections::BTreeMap<String, String>,
    packages: zutai_semantic::PortablePackageGraph,
}

#[derive(Clone)]
enum SymbolTarget {
    Binding {
        module: PathBuf,
        binding: zutai_hir::BindingId,
    },
    ExportedMember {
        module: PathBuf,
        member: String,
    },
}

struct SymbolPosition {
    project: ProjectAnalysis,
    target: SymbolTarget,
    selection: (usize, usize),
    source: String,
}

impl ProjectAnalysis {
    fn path_for(&self, analysis: &zutai_semantic::Analysis) -> Option<PathBuf> {
        if std::ptr::eq(analysis, &self.analysis) {
            return Some(self.root_path.clone());
        }
        let source = analysis.source_path.as_ref()?;
        self.source_paths
            .get(source)
            .cloned()
            .or_else(|| Some(self.filesystem_path(source)))
    }

    fn recorded_source(&self, analysis: &zutai_semantic::Analysis) -> Option<&str> {
        let path = analysis.source_path.as_ref()?;
        if let Some((package, source)) = portable_package_path(path) {
            return self
                .packages
                .packages
                .get(package)?
                .sources
                .get(source)
                .map(String::as_str);
        }
        self.sources.get(&path_key(path)).map(String::as_str)
    }

    fn package_source(
        &self,
        analysis: &zutai_semantic::Analysis,
    ) -> Option<zutai_package::PortablePackageSource> {
        let (package, _) = portable_package_path(analysis.source_path.as_ref()?)?;
        self.packages
            .packages
            .get(package)
            .map(|package| package.source)
    }

    fn module_identity(&self, analysis: &zutai_semantic::Analysis) -> PathBuf {
        if std::ptr::eq(analysis, &self.analysis) {
            return self.root_path.clone();
        }
        analysis
            .source_path
            .clone()
            .or_else(|| self.path_for(analysis))
            .unwrap_or_else(|| self.root_path.clone())
    }

    fn modules(&self) -> Vec<&zutai_semantic::Analysis> {
        fn visit<'a>(
            analysis: &'a zutai_semantic::Analysis,
            identity: PathBuf,
            seen: &mut HashSet<PathBuf>,
            modules: &mut Vec<&'a zutai_semantic::Analysis>,
        ) {
            if !seen.insert(identity) {
                return;
            }
            modules.push(analysis);
            let mut imported: Vec<_> = analysis.import_modules.values().collect();
            imported.sort_by_key(|module| module.source_path.clone());
            for module in imported {
                let identity = module
                    .source_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("<unknown>"));
                visit(module, identity, seen, modules);
            }
        }

        let mut modules = Vec::new();
        visit(
            &self.analysis,
            self.module_identity(&self.analysis),
            &mut HashSet::new(),
            &mut modules,
        );
        modules
    }

    fn filesystem_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }
        self.source_paths.get(path).cloned().unwrap_or_else(|| {
            self.root_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(path)
        })
    }

    fn module(&self, identity: &Path) -> Option<&zutai_semantic::Analysis> {
        self.modules()
            .into_iter()
            .find(|analysis| self.module_identity(analysis) == identity)
    }

    fn writable(&self, analysis: &zutai_semantic::Analysis) -> bool {
        if analysis
            .source_path
            .as_deref()
            .is_some_and(|path| path.starts_with("<stdlib>"))
        {
            return false;
        }
        self.package_source(analysis) != Some(zutai_package::PortablePackageSource::LockedGit)
    }
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
            let Some(project) = self.analyze_with_overlays(&root_uri, &root_source) else {
                continue;
            };
            for (uri, diagnostic) in self.routed_diagnostics(&root_uri, &root_source, &project) {
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

    fn analyze_with_overlays(&self, root_uri: &str, root_source: &str) -> Option<ProjectAnalysis> {
        let root_path = file_path(root_uri)?;
        let root_dir = root_path.parent()?;
        let Some(mut recorded) =
            zutai_semantic::analyze_path_recording_with_cache(&root_path, &self.analysis_cache)
                .ok()
        else {
            return analyze(root_source, root_uri).map(|analysis| ProjectAnalysis {
                analysis,
                root_path,
                source_paths: std::collections::BTreeMap::new(),
                sources: std::collections::BTreeMap::new(),
                packages: zutai_semantic::PortablePackageGraph::default(),
            });
        };
        let package_setup: Vec<_> = recorded
            .analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.kind,
                    zutai_semantic::SemanticDiagnosticKind::Import(
                        zutai_semantic::ImportDiagnostic {
                            kind: zutai_semantic::ImportDiagnosticKind::PackageSetup { .. },
                            ..
                        }
                    )
                )
            })
            .cloned()
            .collect();
        if !package_setup.is_empty() {
            let mut analysis = analyze(root_source, root_uri)?;
            analysis.diagnostics.extend(package_setup);
            return Some(ProjectAnalysis {
                analysis,
                root_path,
                source_paths: std::collections::BTreeMap::new(),
                sources: std::collections::BTreeMap::new(),
                packages: zutai_semantic::PortablePackageGraph::default(),
            });
        }
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
        for (analysis_path, filesystem_path) in &recorded.source_paths {
            let uri = self.uri_for_path(filesystem_path);
            let Some(document) = self.documents.get(&uri) else {
                continue;
            };
            if let Some((package, path)) = portable_package_path(analysis_path)
                && let Some(package) = recorded.packages.packages.get_mut(package)
            {
                package
                    .sources
                    .insert(path.to_owned(), document.text.clone());
            }
        }
        recorded
            .sources
            .insert(recorded.entry.clone(), root_source.to_string());
        let stdlib = zutai_semantic::StdlibSources::from_memory(
            recorded.stdlib_compiler_compatibility.clone(),
            recorded.stdlib_sources.clone(),
        )
        .ok()?;
        let packages = recorded.packages.clone();
        let sources = recorded.sources.clone();
        let analysis = zutai_semantic::analyze_sources_with_stdlib_packages_and_cache(
            &recorded.entry,
            &recorded.sources,
            zutai_semantic::AnalysisOptions::default(),
            &stdlib,
            recorded.packages,
            Some(&self.analysis_cache),
        )
        .ok()?;
        Some(ProjectAnalysis {
            analysis,
            root_path,
            source_paths: recorded.source_paths,
            sources,
            packages,
        })
    }

    fn routed_diagnostics(
        &self,
        root_uri: &str,
        root_source: &str,
        project: &ProjectAnalysis,
    ) -> Vec<(String, Value)> {
        let analysis = &project.analysis;
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
            if let zutai_semantic::SemanticDiagnosticKind::Import(import) = &diagnostic.kind {
                let primary_path = import
                    .path
                    .as_deref()
                    .map(|path| project.filesystem_path(path));
                let primary_uri = primary_path
                    .as_deref()
                    .map(|path| self.uri_for_path(path))
                    .unwrap_or_else(|| root_uri.to_string());
                let primary_source = primary_path
                    .as_deref()
                    .and_then(|path| {
                        self.source_for(&primary_uri)
                            .or_else(|| std::fs::read_to_string(path).ok())
                    })
                    .unwrap_or_else(|| root_source.to_string());
                let mut value = diagnostic_value(&primary_source, &primary_uri, diagnostic);
                let related: Vec<_> = import
                    .related
                    .iter()
                    .map(|location| {
                        let path = project.filesystem_path(&location.path);
                        json!({
                            "location": {
                                "uri": self.uri_for_path(&path),
                                "range": self.range_for_analysis_path(&path, location.span),
                            },
                            "message": location.label,
                        })
                    })
                    .collect();
                if !related.is_empty() {
                    value["relatedInformation"] = Value::Array(related);
                }
                output.push((primary_uri.to_owned(), value));
            } else {
                output.push((
                    root_uri.to_string(),
                    diagnostic_value(root_source, root_uri, diagnostic),
                ));
            }
        }
        for diagnostic in analysis.native_import_diagnostics() {
            let mut value = json!({
                "range": range(root_source, diagnostic.span.start as usize, diagnostic.span.end as usize),
                "severity": 2,
                "source": "zutai",
                "message": diagnostic.message,
            });
            let related: Vec<_> = diagnostic
                .related
                .iter()
                .map(|location| {
                    let path = project.filesystem_path(&location.path);
                    json!({
                        "location": {
                            "uri": self.uri_for_path(&path),
                            "range": self.range_for_analysis_path(&path, location.span),
                        },
                        "message": location.label,
                    })
                })
                .collect();
            if !related.is_empty() {
                value["relatedInformation"] = Value::Array(related);
            }
            output.push((root_uri.to_string(), value));
        }

        output
    }

    fn range_for_analysis_path(&self, path: &Path, span: zutai_syntax::Span) -> Value {
        let uri = self.uri_for_path(path);
        self.source_for(&uri)
            .map(|source| range(&source, span.start as usize, span.end as usize))
            .unwrap_or_else(|| range("", 0, 0))
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

    fn definition(&self, params: &Value) -> Value {
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

    fn references(&self, params: &Value) -> Value {
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

    fn document_symbols(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(project) = self.analyze_with_overlays(uri, &source) else {
            return Value::Null;
        };
        let Some(hir) = project.analysis.hir.as_ref().map(|lowered| &lowered.file) else {
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
        let Some(project) = self.analyze_with_overlays(uri, &source) else {
            return Value::Null;
        };
        let Some(hir) = project.analysis.hir.as_ref().map(|lowered| &lowered.file) else {
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
        let Some(position) = self.symbol_at_position(params) else {
            return Value::Null;
        };
        if !self.renameable_symbol(&position.project, &position.target) {
            return Value::Null;
        }
        range(&position.source, position.selection.0, position.selection.1)
    }

    fn rename(&self, params: &Value) -> Value {
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

    fn signature_help(&self, params: &Value) -> Value {
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

    fn code_actions(&self, params: &Value) -> Value {
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

    fn imported_member_target<'a>(
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

    fn project_for_document(&self, uri: &str, source: &str) -> Option<ProjectAnalysis> {
        let requested = file_path(uri)?;
        let requested = std::fs::canonicalize(&requested).unwrap_or(requested);
        let mut roots: Vec<_> = self.documents.keys().cloned().collect();
        if !roots.iter().any(|root| root == uri) {
            roots.push(uri.to_owned());
        }
        roots.sort();
        roots.dedup();

        let mut best = None;
        let mut best_size = 0;
        for root_uri in roots {
            let Some(root_path) = file_path(&root_uri) else {
                continue;
            };
            if root_path
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("zt")
            {
                continue;
            }
            let root_source = if root_uri == uri {
                source.to_owned()
            } else if let Some(source) = self.source_for(&root_uri) {
                source
            } else {
                continue;
            };
            let Some(project) = self.analyze_with_overlays(&root_uri, &root_source) else {
                continue;
            };
            let modules = project.modules();
            let contains = modules.iter().any(|analysis| {
                project
                    .path_for(analysis)
                    .is_some_and(|path| std::fs::canonicalize(&path).unwrap_or(path) == requested)
            });
            if contains && modules.len() > best_size {
                best_size = modules.len();
                best = Some(project);
            }
        }
        best.or_else(|| self.analyze_with_overlays(uri, source))
    }

    fn symbol_at_position(&self, params: &Value) -> Option<SymbolPosition> {
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

    fn renameable_symbol(&self, project: &ProjectAnalysis, target: &SymbolTarget) -> bool {
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

    fn symbol_references(
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

    fn symbol_reference_locations(
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

    fn source_for_analysis(
        &self,
        project: &ProjectAnalysis,
        analysis: &zutai_semantic::Analysis,
    ) -> Option<(String, String)> {
        let path = project.path_for(analysis)?;
        let uri = self.uri_for_path(&path);
        let source = self
            .source_for(&uri)
            .or_else(|| project.recorded_source(analysis).map(str::to_owned))?;
        Some((uri, source))
    }

    fn binding_at_position(
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

    fn source_for(&self, uri: &str) -> Option<String> {
        self.documents
            .get(uri)
            .map(|document| document.text.clone())
            .or_else(|| file_path(uri).and_then(|path| std::fs::read_to_string(path).ok()))
    }
}

fn imported_member_selection(
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

fn exported_member_origin(
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

fn resolve_exported_member_target(
    project: &ProjectAnalysis,
    analysis: &zutai_semantic::Analysis,
    member: &str,
) -> (PathBuf, String) {
    exported_member_origin(project, analysis, member)
        .unwrap_or_else(|| (project.module_identity(analysis), member.to_owned()))
}

fn exported_member_local_binding(
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

fn exported_member_for_binding(
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

fn exported_field_range(
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

fn imported_member_reference_ranges(
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

fn path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn portable_package_path(path: &std::path::Path) -> Option<(&str, &str)> {
    let mut components = path.components();
    (components.next()?.as_os_str() == "<package>").then_some(())?;
    let package = components.next()?.as_os_str().to_str()?;
    let source = path
        .strip_prefix(Path::new("<package>").join(package))
        .ok()?;
    source.to_str().map(|source| (package, source))
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
        .map(|diagnostic| diagnostic_value(source, "file:///test.zt", diagnostic))
        .collect()
}

fn diagnostic_value(
    source: &str,
    uri: &str,
    diagnostic: &zutai_semantic::SemanticDiagnostic,
) -> Value {
    match &diagnostic.kind {
        zutai_semantic::SemanticDiagnosticKind::Parse(parse) => json!({
            "range": range(source, parse.primary_span().start as usize, parse.primary_span().end as usize),
            "severity": severity(parse.severity),
            "code": parse.code,
            "source": "zutai",
            "message": parse.message,
        }),
        zutai_semantic::SemanticDiagnosticKind::Import(import) => json!({
            "range": range(source, import.span.start as usize, import.span.end as usize),
            "severity": 1,
            "source": "zutai",
            "message": format_import_diagnostic(import),
        }),
        _ => {
            let (message, start, end) = zutai_eval::describe_semantic_diagnostic(diagnostic)
                .expect("HIR and THIR diagnostics always have a source span");
            let mut value = json!({
                "range": range(source, start as usize, end as usize),
                "severity": 1,
                "source": "zutai",
                "message": message,
            });
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

fn imported_member_at(
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

fn import_source_for_binding(
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

fn exported_member_range(
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

fn access_field_range(
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

fn name_range_in_span(
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
    fn stdlib_html_and_css_type_errors_publish_lsp_diagnostics() {
        let cases = [
            (
                "file:///tmp/bad-html.zt",
                "html ::= import stdlib.html;\nMsg :: type { #save; };\nbad :: html.Html Msg = html.button { html.onClick 1; } { html.text \"save\"; };\nbad",
                "type mismatch",
            ),
            (
                "file:///tmp/bad-css.zt",
                "css ::= import stdlib.css;\nbad :: css.Stylesheet = css.stylesheet { css.rule { css.class \"card\"; } { css.padding (css.rem \"large\"); }; };\nbad",
                "expected Float, found Text",
            ),
        ];

        for (uri, source, expected) in cases {
            let analysis = analyze(source, uri).unwrap();
            let diagnostics = diagnostics(source, &analysis);
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic
                    .get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message.contains(expected))),
                "expected `{expected}` for {uri}, got {diagnostics:?}"
            );
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.get("range").is_some())
            );
        }
    }

    #[test]
    fn derive_diagnostic_carries_definition_related_information() {
        let source = "Ord :: <A> @A { compare :: A -> A -> Bool; } derive\nOrd @Int :: derive\n1";
        let analysis = analyze(source, "file:///tmp/derive.zt").unwrap();

        let diagnostics = diagnostics(source, &analysis);
        let derive = diagnostics
            .iter()
            .find(|d| {
                d.get("message")
                    .and_then(|m| m.as_str())
                    .is_some_and(|m| m.contains("cannot derive `Ord`"))
            })
            .expect("expected the derive diagnostic");
        let related = derive
            .get("relatedInformation")
            .and_then(|r| r.as_array())
            .expect("derive diagnostic should carry relatedInformation");
        assert_eq!(related.len(), 1);
        assert_eq!(
            related[0]["message"].as_str(),
            Some("constraint defined here")
        );
        // The related range starts on line 0 (the constraint declaration), while
        // the primary range is the derive request on line 1.
        assert_eq!(
            related[0]["location"]["range"]["start"]["line"].as_u64(),
            Some(0)
        );
        assert_eq!(
            derive["range"]["start"]["line"].as_u64(),
            Some(1),
            "primary range should sit at the derive request"
        );
    }

    #[test]
    fn native_gated_import_warning_retains_use_and_export_locations() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "zutai-lsp-native-import-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dep_path = dir.join("dep.zt");
        let root_path = dir.join("main.zt");
        let dep_source = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @List :: { map = \\f xs. xs; }\n1\n";
        let root_source = "m ::= import \"dep.zt\";\nm\n";
        std::fs::write(&dep_path, dep_source).unwrap();
        std::fs::write(&root_path, root_source).unwrap();
        let root_uri = file_uri(&root_path);
        let mut server = Server::default();
        server.documents.insert(
            root_uri.clone(),
            Document {
                text: root_source.to_owned(),
                version: Some(1),
            },
        );
        let project = server
            .analyze_with_overlays(&root_uri, root_source)
            .expect("project analysis");
        let diagnostics = server.routed_diagnostics(&root_uri, root_source, &project);
        let warning = diagnostics
            .iter()
            .map(|(_, diagnostic)| diagnostic)
            .find(|diagnostic| {
                diagnostic
                    .get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message.contains("non-matchable typeclass instances"))
            })
            .expect("native import warning");
        assert_eq!(warning["severity"], json!(2));
        assert_eq!(warning["range"]["start"]["line"], json!(0));
        let related = warning["relatedInformation"].as_array().unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0]["location"]["uri"], json!(file_uri(&dep_path)));
        assert_eq!(related[0]["location"]["range"]["start"]["line"], json!(1));
        assert_eq!(
            related[0]["message"],
            json!("non-matchable witness exported here")
        );

        let _ = std::fs::remove_dir_all(dir);
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

    fn package_manifest(name: &str, modules: &str, dependencies: &str) -> String {
        let modules = if modules.is_empty() {
            "[]".to_owned()
        } else {
            format!("[{modules};]")
        };
        let dependencies = if dependencies.is_empty() {
            "[]".to_owned()
        } else {
            format!("[{dependencies};]")
        };
        format!(
            "{{ formatVersion = 1; name = \"{name}\"; compilerCompatibility = \"{}\"; modules = {modules}; dependencies = {dependencies}; }}",
            env!("CARGO_PKG_VERSION")
        )
    }

    #[test]
    fn package_graph_navigation_overlays_and_diagnostics_match_cli_analysis() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("zutai-lsp-package-{}-{nonce}", std::process::id()));
        let app = root.join("app");
        let dep = root.join("dep");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(
            app.join("zutai.zti"),
            package_manifest("app", "", "{ alias = \"dep\"; path = \"../dep\"; }"),
        )
        .unwrap();
        std::fs::write(
            dep.join("zutai.zti"),
            package_manifest("dep", "{ name = \"api\"; path = \"src/api.zt\"; }", ""),
        )
        .unwrap();

        let app_path = app.join("src/main.zt");
        let dep_path = dep.join("src/api.zt");
        let app_source = "api ::= import dep.api;\nvalue :: api.Count = api.answer;\nvalue\n";
        let dep_source = "Count :: type Int;\nanswer :: Count = 42;\n{ answer = answer; }\n";
        std::fs::write(&app_path, app_source).unwrap();
        std::fs::write(&dep_path, dep_source).unwrap();
        let app_uri = file_uri(&app_path);
        let dep_uri = file_uri(&dep_path);

        let mut server = Server::default();
        server.documents.insert(
            app_uri.clone(),
            Document {
                text: app_source.to_owned(),
                version: Some(1),
            },
        );
        server.documents.insert(
            dep_uri.clone(),
            Document {
                text: dep_source.to_owned(),
                version: Some(1),
            },
        );

        let value = server.definition(&json!({
            "textDocument": { "uri": app_uri },
            "position": { "line": 1, "character": 27 }
        }));
        assert_eq!(
            value.get("uri").and_then(Value::as_str),
            Some(dep_uri.as_str())
        );
        assert_eq!(
            value.pointer("/range/start").cloned(),
            Some(json!({ "line": 1, "character": 0 }))
        );

        let ty = server.definition(&json!({
            "textDocument": { "uri": app_uri },
            "position": { "line": 1, "character": 14 }
        }));
        assert_eq!(
            ty.get("uri").and_then(Value::as_str),
            Some(dep_uri.as_str())
        );
        assert_eq!(
            ty.pointer("/range/start").cloned(),
            Some(json!({ "line": 0, "character": 0 }))
        );

        server.documents.get_mut(&dep_uri).unwrap().text =
            "Count :: type Bool;\nanswer :: Count = true;\n{ answer = answer; }\n".to_owned();
        let hover = server.hover(&json!({
            "textDocument": { "uri": app_uri },
            "position": { "line": 1, "character": 27 }
        }));
        assert_eq!(
            hover.pointer("/contents/value").and_then(Value::as_str),
            Some("```zutai\nBool\n```")
        );

        let bad_source = "api ::= import missing.api;\napi\n";
        std::fs::write(&app_path, bad_source).unwrap();
        server.documents.get_mut(&app_uri).unwrap().text = bad_source.to_owned();
        let cli = zutai_semantic::analyze_path(&app_path).unwrap();
        let cli_import = cli
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                zutai_semantic::SemanticDiagnosticKind::Import(import) => Some(import),
                _ => None,
            })
            .expect("CLI analysis should report the unresolved dependency");
        let project = server.analyze_with_overlays(&app_uri, bad_source).unwrap();
        let lsp_import = project
            .analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                zutai_semantic::SemanticDiagnosticKind::Import(import) => Some(import),
                _ => None,
            })
            .expect("LSP analysis should report the unresolved dependency");
        assert_eq!(lsp_import, cli_import);
        let diagnostic = diagnostic_value(
            bad_source,
            &app_uri,
            project
                .analysis
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    matches!(
                        diagnostic.kind,
                        zutai_semantic::SemanticDiagnosticKind::Import(_)
                    )
                })
                .unwrap(),
        );
        assert_eq!(
            diagnostic.pointer("/range/start").cloned(),
            Some(position_at(bad_source, cli_import.span.start as usize))
        );
        assert_eq!(
            diagnostic.pointer("/range/end").cloned(),
            Some(position_at(bad_source, cli_import.span.end as usize))
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn package_wide_references_and_safe_rename_respect_overlays_and_shadowing() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-lsp-package-references-{}-{nonce}",
            std::process::id()
        ));
        let app = root.join("app");
        let middle = root.join("middle");
        let leaf = root.join("leaf");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(middle.join("src")).unwrap();
        std::fs::create_dir_all(leaf.join("src")).unwrap();
        std::fs::write(
            app.join("zutai.zti"),
            package_manifest("app", "", "{ alias = \"middle\"; path = \"../middle\"; }"),
        )
        .unwrap();
        std::fs::write(
            middle.join("zutai.zti"),
            package_manifest(
                "middle",
                "{ name = \"api\"; path = \"src/api.zt\"; }; { name = \"other\"; path = \"src/other.zt\"; }",
                "{ alias = \"leaf\"; path = \"../leaf\"; }",
            ),
        )
        .unwrap();
        std::fs::write(
            leaf.join("zutai.zti"),
            package_manifest("leaf", "{ name = \"api\"; path = \"src/api.zt\"; }", ""),
        )
        .unwrap();

        let app_path = app.join("src/main.zt");
        let middle_path = middle.join("src/api.zt");
        let leaf_path = leaf.join("src/api.zt");
        let other_path = middle.join("src/other.zt");
        let app_source = "m ::= import middle.api;\no ::= import middle.other;\nvalue :: m.Count = m.answer + m.answer;\nunrelated ::= o.answer;\nvalue\n";
        let middle_source = "l ::= import leaf.api;\nvalue :: l.Count = l.answer;\n{ Count = l.Count; answer = l.answer; }\n";
        let other_source =
            "l ::= import leaf.api;\nown ::= 99;\n{ answer = own; retained = l.answer; }\n";
        let leaf_source = "Count :: type Int;\nanswer :: Count = 41;\nshadow ::= (\\answer. answer) 1;\n{ Count = Count; answer = answer; }\n";
        let leaf_overlay = "Count :: type Int;\nanswer :: Count = 42;\nshadow ::= (\\answer. answer) 1;\n{ Count = Count; answer = answer; }\n";
        std::fs::write(&other_path, other_source).unwrap();
        std::fs::write(&app_path, app_source).unwrap();
        std::fs::write(&middle_path, middle_source).unwrap();
        std::fs::write(&leaf_path, leaf_source).unwrap();
        let app_uri = file_uri(&app_path);
        let middle_uri = file_uri(&middle_path);
        let other_uri = file_uri(&other_path);
        let leaf_uri = file_uri(&leaf_path);

        let mut server = Server::default();
        for (uri, text) in [
            (&app_uri, app_source),
            (&middle_uri, middle_source),
            (&other_uri, other_source),
            (&leaf_uri, leaf_overlay),
        ] {
            server.documents.insert(
                uri.clone(),
                Document {
                    text: text.to_owned(),
                    version: Some(1),
                },
            );
        }

        let references = server.references(&json!({
            "textDocument": { "uri": app_uri },
            "position": { "line": 2, "character": 27 },
            "context": { "includeDeclaration": true }
        }));
        let locations = references.as_array().unwrap();
        assert_eq!(locations.len(), 9, "{references}");
        let by_uri = |uri: &str| {
            locations
                .iter()
                .filter(|location| location.get("uri").and_then(Value::as_str) == Some(uri))
                .count()
        };
        assert_eq!(by_uri(&app_uri), 2, "{references}");
        assert_eq!(by_uri(&middle_uri), 3, "{references}");
        assert_eq!(by_uri(&leaf_uri), 3, "{references}");
        assert_eq!(by_uri(&other_uri), 1, "{references}");
        assert!(!locations.iter().any(|location| {
            location.get("uri").and_then(Value::as_str) == Some(leaf_uri.as_str())
                && location
                    .pointer("/range/start/line")
                    .and_then(Value::as_u64)
                    == Some(2)
        }));

        let rename = server.rename(&json!({
            "textDocument": { "uri": leaf_uri },
            "position": { "line": 1, "character": 2 },
            "newName": "total"
        }));
        let changes = rename
            .pointer("/changes")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(changes.len(), 4, "{rename}");
        assert_eq!(changes[&app_uri].as_array().map(Vec::len), Some(2));
        assert_eq!(changes[&middle_uri].as_array().map(Vec::len), Some(3));
        assert_eq!(changes[&leaf_uri].as_array().map(Vec::len), Some(3));
        assert_eq!(changes[&other_uri].as_array().map(Vec::len), Some(1));
        assert!(
            !changes[&leaf_uri].as_array().unwrap().iter().any(|edit| {
                edit.pointer("/range/start/line").and_then(Value::as_u64) == Some(2)
            })
        );

        let apply_edits = |source: &str, edits: &Value| {
            let mut replacements: Vec<_> = edits
                .as_array()
                .unwrap()
                .iter()
                .map(|edit| {
                    let edit_range = edit.get("range").unwrap();
                    let start = offset_at(source, edit_range.get("start").unwrap()).unwrap();
                    let end = offset_at(source, edit_range.get("end").unwrap()).unwrap();
                    (start, end, edit.get("newText").unwrap().as_str().unwrap())
                })
                .collect();
            replacements.sort_unstable_by_key(|(start, _, _)| std::cmp::Reverse(*start));
            let mut renamed = source.to_owned();
            for (start, end, replacement) in replacements {
                renamed.replace_range(start..end, replacement);
            }
            renamed
        };
        let renamed_app = apply_edits(app_source, &changes[&app_uri]);
        let renamed_middle = apply_edits(middle_source, &changes[&middle_uri]);
        let renamed_leaf = apply_edits(leaf_overlay, &changes[&leaf_uri]);
        let renamed_other = apply_edits(other_source, &changes[&other_uri]);
        let mut renamed_server = Server::default();
        for (uri, text) in [
            (&app_uri, &renamed_app),
            (&middle_uri, &renamed_middle),
            (&other_uri, &renamed_other),
            (&leaf_uri, &renamed_leaf),
        ] {
            renamed_server.documents.insert(
                uri.clone(),
                Document {
                    text: text.clone(),
                    version: Some(2),
                },
            );
        }
        let renamed_project = renamed_server
            .project_for_document(&app_uri, &renamed_app)
            .expect("renamed project analysis");
        assert!(
            renamed_project
                .modules()
                .into_iter()
                .all(|analysis| analysis.diagnostics.is_empty()),
            "renamed sources must remain valid: {renamed_app}\n{renamed_middle}\n{renamed_leaf}"
        );

        let type_references = server.references(&json!({
            "textDocument": { "uri": app_uri },
            "position": { "line": 2, "character": 13 },
            "context": { "includeDeclaration": true }
        }));
        assert_eq!(type_references.as_array().map(Vec::len), Some(8));

        let mut locked_project = server
            .project_for_document(&app_uri, app_source)
            .expect("three-package project");
        let leaf_id = locked_project
            .packages
            .packages
            .iter()
            .find(|(_, package)| package.name == "leaf")
            .map(|(id, _)| id.clone())
            .unwrap();
        locked_project
            .packages
            .packages
            .get_mut(&leaf_id)
            .unwrap()
            .source = zutai_package::PortablePackageSource::LockedGit;
        let leaf_analysis = locked_project
            .modules()
            .into_iter()
            .find(|analysis| locked_project.path_for(analysis).as_deref() == Some(&leaf_path))
            .unwrap();
        let target = SymbolTarget::ExportedMember {
            module: locked_project.module_identity(leaf_analysis),
            member: "answer".to_owned(),
        };
        assert!(!server.renameable_symbol(&locked_project, &target));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn package_diagnostics_route_imports_and_manifest_provenance() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-lsp-package-diagnostics-{}-{nonce}",
            std::process::id()
        ));

        let unknown = root.join("unknown");
        std::fs::create_dir_all(unknown.join("src")).unwrap();
        std::fs::write(
            unknown.join("zutai.zti"),
            package_manifest("unknown", "", ""),
        )
        .unwrap();
        let unknown_path = unknown.join("src/main.zt");
        let unknown_source = "api ::= import missing.api;\napi\n";
        std::fs::write(&unknown_path, unknown_source).unwrap();
        let unknown_uri = file_uri(&unknown_path);
        let mut server = Server::default();
        server.documents.insert(
            unknown_uri.clone(),
            Document {
                text: unknown_source.to_owned(),
                version: Some(1),
            },
        );
        let project = server
            .analyze_with_overlays(&unknown_uri, unknown_source)
            .unwrap();
        let routed = server.routed_diagnostics(&unknown_uri, unknown_source, &project);
        let (uri, diagnostic) = routed
            .iter()
            .find(|(_, diagnostic)| {
                diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("unknown package dependency alias"))
            })
            .expect("unknown package alias diagnostic");
        assert_eq!(uri, &unknown_uri);
        assert_eq!(diagnostic["range"]["start"]["line"], json!(0));

        let duplicate = root.join("duplicate");
        let dep = root.join("dep");
        std::fs::create_dir_all(duplicate.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(dep.join("zutai.zti"), package_manifest("dep", "", "")).unwrap();
        let duplicate_manifest = package_manifest(
            "duplicate",
            "",
            "{ alias = \"dep\"; path = \"../dep\"; }; { alias = \"dep\"; path = \"../dep\"; }",
        );
        let duplicate_manifest_path = duplicate.join("zutai.zti");
        std::fs::write(&duplicate_manifest_path, &duplicate_manifest).unwrap();
        let duplicate_path = duplicate.join("src/main.zt");
        let duplicate_source = "api ::= import dep.api;\napi\n";
        std::fs::write(&duplicate_path, duplicate_source).unwrap();
        let duplicate_uri = file_uri(&duplicate_path);
        let duplicate_manifest_uri = file_uri(&duplicate_manifest_path);
        server.documents.insert(
            duplicate_uri.clone(),
            Document {
                text: duplicate_source.to_owned(),
                version: Some(1),
            },
        );
        let project = server
            .analyze_with_overlays(&duplicate_uri, duplicate_source)
            .unwrap();
        let routed = server.routed_diagnostics(&duplicate_uri, duplicate_source, &project);
        let (uri, diagnostic) = routed
            .iter()
            .find(|(_, diagnostic)| {
                diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("duplicate dependency alias"))
            })
            .expect("duplicate dependency alias diagnostic");
        assert_eq!(uri, &duplicate_manifest_uri);
        let second_alias = duplicate_manifest.rfind("\"dep\"").unwrap();
        assert_eq!(
            diagnostic["range"]["start"],
            position_at(&duplicate_manifest, second_alias)
        );

        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(a.join("src")).unwrap();
        std::fs::create_dir_all(b.join("src")).unwrap();
        std::fs::write(
            a.join("zutai.zti"),
            package_manifest("a", "", "{ alias = \"b\"; path = \"../b\"; }"),
        )
        .unwrap();
        std::fs::write(
            b.join("zutai.zti"),
            package_manifest("b", "", "{ alias = \"a\"; path = \"../a\"; }"),
        )
        .unwrap();
        let cycle_path = a.join("src/main.zt");
        let cycle_source = "api ::= import b.api;\napi\n";
        std::fs::write(&cycle_path, cycle_source).unwrap();
        let cycle_uri = file_uri(&cycle_path);
        server.documents.insert(
            cycle_uri.clone(),
            Document {
                text: cycle_source.to_owned(),
                version: Some(1),
            },
        );
        let project = server
            .analyze_with_overlays(&cycle_uri, cycle_source)
            .unwrap();
        let routed = server.routed_diagnostics(&cycle_uri, cycle_source, &project);
        let (uri, diagnostic) = routed
            .iter()
            .find(|(_, diagnostic)| {
                diagnostic["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("package dependency cycle"))
            })
            .expect("package dependency cycle diagnostic");
        assert_eq!(uri, &cycle_uri);
        let related = diagnostic["relatedInformation"].as_array().unwrap();
        assert_eq!(related.len(), 2);
        assert_eq!(
            related[0]["location"]["uri"],
            json!(file_uri(&a.join("zutai.zti")))
        );
        assert_eq!(
            related[1]["location"]["uri"],
            json!(file_uri(&b.join("zutai.zti")))
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_package_manifest_diagnostic_survives_overlay_analysis() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-lsp-package-setup-{}-{nonce}",
            std::process::id()
        ));
        let entry = root.join("src/main.zt");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(
            root.join("zutai.zti"),
            "{ formatVersion = \"bad\"; name = \"app\"; modules = []; dependencies = []; }\n",
        )
        .unwrap();
        let source = "1\n";
        std::fs::write(&entry, source).unwrap();

        let cli = zutai_semantic::analyze_path(&entry).unwrap();
        let cli_setup = cli
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                zutai_semantic::SemanticDiagnosticKind::Import(import)
                    if matches!(
                        import.kind,
                        zutai_semantic::ImportDiagnosticKind::PackageSetup { .. }
                    ) =>
                {
                    Some(import)
                }
                _ => None,
            })
            .expect("CLI analysis should report the malformed package manifest");

        let uri = file_uri(&entry);
        let mut server = Server::default();
        server.documents.insert(
            uri.clone(),
            Document {
                text: source.to_owned(),
                version: Some(1),
            },
        );
        let project = server.analyze_with_overlays(&uri, source).unwrap();
        let lsp_setup = project
            .analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                zutai_semantic::SemanticDiagnosticKind::Import(import)
                    if matches!(
                        import.kind,
                        zutai_semantic::ImportDiagnosticKind::PackageSetup { .. }
                    ) =>
                {
                    Some(import)
                }
                _ => None,
            })
            .expect("LSP analysis should preserve the malformed package manifest diagnostic");
        assert_eq!(lsp_setup, cli_setup);

        let _ = std::fs::remove_dir_all(root);
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
