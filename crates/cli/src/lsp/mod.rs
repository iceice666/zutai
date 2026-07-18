//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

mod analysis;
mod protocol;
mod requests;
#[cfg(test)]
mod tests;

use analysis::*;
use protocol::*;

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompletionCandidate {
    name: String,
    kind: u8,
    detail: String,
}

struct ImportCompletionContext {
    completed: Vec<String>,
    prefix: String,
    start: usize,
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

    fn completion_packages(&self) -> &zutai_semantic::PortablePackageGraph {
        &self.packages
    }

    fn package_for_analysis(
        &self,
        analysis: &zutai_semantic::Analysis,
    ) -> Option<(&str, &zutai_semantic::PortablePackage)> {
        let (id, _) = portable_package_path(analysis.source_path.as_ref()?)?;
        self.completion_packages()
            .packages
            .get_key_value(id)
            .map(|(id, package)| (id.as_str(), package))
    }

    fn owner_package(&self, analysis: &zutai_semantic::Analysis) -> Option<&str> {
        if std::ptr::eq(analysis, &self.analysis) {
            return self.completion_packages().root_package.as_deref();
        }
        self.package_for_analysis(analysis).map(|(id, _)| id)
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

    fn public_modules(
        &self,
        stdlib: &zutai_semantic::StdlibSources,
    ) -> Vec<(String, String, zutai_semantic::Analysis)> {
        let mut modules = Vec::new();
        for (id, package) in &self.packages.packages {
            for (name, path) in &package.modules {
                let entry = path_key(&Path::new("<package>").join(id).join(path));
                let mut graph = self.packages.clone();
                graph.root_package = Some(id.clone());
                let mut sources = package.sources.clone();
                let Some(entry_source) = package.sources.get(path) else {
                    continue;
                };
                sources.insert(entry.clone(), entry_source.clone());
                let Ok(analysis) = zutai_semantic::analyze_sources_with_stdlib_and_packages(
                    &entry,
                    &sources,
                    zutai_semantic::AnalysisOptions::default(),
                    stdlib,
                    graph,
                ) else {
                    continue;
                };
                modules.push((package.name.clone(), name.clone(), analysis));
            }
        }
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
                                    "workspaceSymbolProvider": true,
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
                                    },
                                    "documentFormattingProvider": true,
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
            "workspace/symbol" => {
                if let Some(id) = id {
                    let result = self.workspace_symbols(&params);
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
            "textDocument/formatting" => {
                if let Some(id) = id {
                    let result = self.formatting(&params);
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
                source_paths: recorded.source_paths,
                sources: recorded.sources,
                packages: recorded.packages,
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
                            "severity": severity(diagnostic.metadata().severity),
                            "code": diagnostic.metadata().code,
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
        for diagnostic in analysis.backend_diagnostics() {
            let mut value = json!({
                "range": range(root_source, diagnostic.span.start as usize, diagnostic.span.end as usize),
                "severity": severity(diagnostic.severity),
                "code": diagnostic.code,
                "source": "zutai",
                "message": diagnostic.message.clone(),
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

    fn source_for(&self, uri: &str) -> Option<String> {
        self.documents
            .get(uri)
            .map(|document| document.text.clone())
            .or_else(|| file_path(uri).and_then(|path| std::fs::read_to_string(path).ok()))
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
}
