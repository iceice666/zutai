//! Module loader for imports.
//!
//! THIR lowering is pure, so all filesystem work happens here: walk the HIR for
//! import declarations (represented internally as `Import` expression nodes),
//! resolve each path relative to the importing file's directory, and produce
//! both a structural type (for THIR) and the data needed by the evaluator.
//!
//! - `.zti` (immediate data): parse the file and keep the parsed value; its type
//!   is derived structurally.
//! - `.zt` (module): recursively analyze the file, type the import by its final
//!   expression's exported type, and keep the analyzed sub-module so the
//!   evaluator can evaluate it.  Import cycles are detected and reported.
//!
//! Functions cross module boundaries via home-module handles stamped by the
//! evaluator.  Type-valued fields carry their denotation in `ImportedType::Type`
//! so annotation-position access (`x : serverLib.Server`) type-checks.

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use zutai_hir::{BindingId, HirExprKind, HirFile, HirImportSource};
use zutai_syntax::Span;
use zutai_thir::{
    ImportKey, ImportedField, ImportedFieldProvenance, ImportedProvenance,
    ImportedProvenanceChildren, ImportedRowTail, ImportedTupleItem, ImportedType, ThirDeclKind,
    ThirExprKind, ThirFile, WitnessPattern, export_witness_pattern,
};

use crate::package::{PackageGraph, PortablePackageGraph};
use crate::{Analysis, AnalysisOptions, StdlibSources};

/// Recursion state shared across a single top-level analysis: the stack of
/// modules currently being analyzed (for cycle detection) and a cache of
/// already-analyzed `.zt` modules keyed by canonical path.
enum SourceBackend<'a> {
    Filesystem {
        confinement_root: Option<PathBuf>,
        recording_root: Option<PathBuf>,
        recorded: BTreeMap<String, String>,
    },
    Memory(&'a BTreeMap<String, String>),
}

pub(crate) struct ImportContext<'a> {
    in_progress: Vec<PathBuf>,
    cache: FxHashMap<PathBuf, Rc<Analysis>>,
    source_backend: SourceBackend<'a>,
    stdlib: Rc<StdlibSources>,
    recorded_stdlib: BTreeMap<String, String>,
    packages: PackageGraph,
    recorded_packages: PortablePackageGraph,
    package_setup_reported: bool,
}

impl ImportContext<'_> {
    pub(crate) fn new(stdlib: Rc<StdlibSources>) -> Self {
        Self {
            in_progress: Vec::new(),
            cache: FxHashMap::default(),
            source_backend: SourceBackend::Filesystem {
                confinement_root: None,
                recording_root: None,
                recorded: BTreeMap::new(),
            },
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages: PackageGraph::None,
            recorded_packages: PortablePackageGraph::default(),
            package_setup_reported: false,
        }
    }
}

struct LoadedSource {
    key: PathBuf,
    contents: String,
}

enum LoadError {
    NotFound,
    Traversal,
    Read(std::io::Error),
}

impl<'a> ImportContext<'a> {
    /// Seed the in-progress stack with the root file's canonical path so that a
    /// descendant importing the root is detected as a cycle.
    pub(crate) fn with_root(path: &Path, stdlib: Rc<StdlibSources>) -> Self {
        let mut ctx = Self::new(stdlib);
        ctx.packages = PackageGraph::discover(path);
        if let Ok(canonical) = std::fs::canonicalize(path) {
            ctx.in_progress.push(canonical);
        }
        ctx
    }

    pub(crate) fn with_base(base: Option<&Path>, stdlib: Rc<StdlibSources>) -> Self {
        let mut ctx = Self::new(stdlib);
        if let Some(base) = base {
            ctx.packages = PackageGraph::discover(base);
        }
        ctx
    }

    pub(crate) fn with_recording_root(path: &Path, stdlib: Rc<StdlibSources>) -> Self {
        let canonical = std::fs::canonicalize(path).ok();
        let recording_root = canonical
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let packages = PackageGraph::discover(path);
        let recorded_packages = packages.portable_skeleton();
        Self {
            in_progress: canonical.into_iter().collect(),
            cache: FxHashMap::default(),
            source_backend: SourceBackend::Filesystem {
                confinement_root: None,
                recording_root,
                recorded: BTreeMap::new(),
            },
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages,
            recorded_packages,
            package_setup_reported: false,
        }
    }

    pub(crate) fn with_explicit_recording_root(
        path: &Path,
        source_root: &Path,
        stdlib: Rc<StdlibSources>,
    ) -> std::io::Result<Self> {
        let canonical = std::fs::canonicalize(path)?;
        let source_root = std::fs::canonicalize(if source_root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            source_root
        })?;
        if !source_root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "source root must be a directory",
            ));
        }
        if !canonical.starts_with(&source_root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "entry path must be inside the source root",
            ));
        }
        let packages = PackageGraph::discover(path);
        let recorded_packages = packages.portable_skeleton();
        Ok(Self {
            in_progress: vec![canonical],
            cache: FxHashMap::default(),
            source_backend: SourceBackend::Filesystem {
                confinement_root: Some(source_root.clone()),
                recording_root: Some(source_root),
                recorded: BTreeMap::new(),
            },
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages,
            recorded_packages,
            package_setup_reported: false,
        })
    }

    pub(crate) fn with_memory(
        sources: &'a BTreeMap<String, String>,
        entry: &Path,
        stdlib: Rc<StdlibSources>,
        packages: PortablePackageGraph,
    ) -> Self {
        let packages = PackageGraph::from_memory(packages);
        Self {
            in_progress: vec![entry.to_path_buf()],
            cache: FxHashMap::default(),
            source_backend: SourceBackend::Memory(sources),
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages,
            recorded_packages: PortablePackageGraph::default(),
            package_setup_reported: false,
        }
    }

    pub(crate) fn record_root_source(&mut self, key: &str, contents: &str) {
        if let SourceBackend::Filesystem { recorded, .. } = &mut self.source_backend {
            recorded.insert(key.to_string(), contents.to_string());
        }
    }

    pub(crate) fn take_recorded_sources(&mut self) -> BTreeMap<String, String> {
        match &mut self.source_backend {
            SourceBackend::Filesystem { recorded, .. } => std::mem::take(recorded),
            SourceBackend::Memory(_) => BTreeMap::new(),
        }
    }

    pub(crate) fn take_recorded_stdlib(&mut self) -> BTreeMap<String, String> {
        std::mem::take(&mut self.recorded_stdlib)
    }

    pub(crate) fn take_recorded_packages(&mut self) -> PortablePackageGraph {
        std::mem::take(&mut self.recorded_packages)
    }

    pub(crate) fn stdlib(&self) -> &StdlibSources {
        &self.stdlib
    }

    fn take_package_setup_error(&mut self) -> Option<String> {
        if self.package_setup_reported {
            return None;
        }
        let message = self.packages.invalid_message()?.to_owned();
        self.package_setup_reported = true;
        Some(message)
    }

    fn load(&mut self, base: &Path, rel: &str) -> Result<LoadedSource, LoadError> {
        if let Some(result) = self.packages.package_source(base, rel) {
            return result
                .map(|source| LoadedSource {
                    key: source.key,
                    contents: source.contents,
                })
                .map_err(|_| LoadError::NotFound);
        }
        match &mut self.source_backend {
            SourceBackend::Filesystem {
                confinement_root,
                recording_root,
                recorded,
            } => {
                let base_dir = if base.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    base
                };
                let canonical_base =
                    std::fs::canonicalize(base_dir).map_err(|_| LoadError::NotFound)?;
                let canonical =
                    std::fs::canonicalize(base_dir.join(rel)).map_err(|_| LoadError::NotFound)?;
                let allowed_root = confinement_root.as_deref().unwrap_or(&canonical_base);
                if !canonical.starts_with(allowed_root) {
                    return Err(LoadError::Traversal);
                }
                let contents = std::fs::read_to_string(&canonical).map_err(LoadError::Read)?;
                self.packages
                    .record_source(&mut self.recorded_packages, &canonical, &contents);
                if let Some(root) = recording_root
                    && let Ok(path) = canonical.strip_prefix(root)
                {
                    recorded.insert(path_to_bundle_key(path), contents.clone());
                }
                Ok(LoadedSource {
                    key: canonical,
                    contents,
                })
            }
            SourceBackend::Memory(sources) => {
                let key = normalize_memory_join(base, rel).ok_or(LoadError::Traversal)?;
                let bundle_key = path_to_bundle_key(&key);
                let contents = sources
                    .get(&bundle_key)
                    .cloned()
                    .ok_or(LoadError::NotFound)?;
                Ok(LoadedSource { key, contents })
            }
        }
    }
}

fn normalize_memory_join(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.contains('\0')
        || rel.contains('\\')
        || Path::new(rel).is_absolute()
        || has_windows_drive_prefix(rel)
    {
        return None;
    }
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !out.pop() {
                    return None;
                }
            }
            part => out.push(part),
        }
    }
    Some(out)
}

pub(crate) fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

pub(crate) fn path_to_bundle_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Everything resolved for a single file's imports.
pub(crate) struct ResolvedImports {
    /// Structural types, keyed by import source — fed into THIR lowering.
    pub types: FxHashMap<ImportKey, ImportedType>,
    pub provenance: FxHashMap<ImportKey, ImportedProvenance>,
    /// Parsed `.zti` values, keyed by import source — consumed by the evaluator.
    pub values: FxHashMap<ImportKey, zutai_im::Value>,
    /// Analyzed `.zt` sub-modules, keyed by import source — evaluated recursively.
    pub modules: FxHashMap<ImportKey, Rc<Analysis>>,
    /// Witnesses exported by imported `.zt` modules, including re-exports.
    pub witnesses: Vec<WitnessExport>,
    pub diagnostics: Vec<ImportDiagnostic>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessExport {
    pub origin: PathBuf,
    pub constraint: String,
    pub target_key: String,
    pub target_display: String,
    /// HIR BindingId.0 of this witness instance's own binding.
    /// Used by the native backend to compute the dep-namespaced DC global name
    /// (`$dep{idx}${constraint}$w{binding_id}`) for cross-module witness dispatch.
    pub binding_id: u32,
    pub span: Span,
    /// For a parametric (conditional) witness such as `Eq @(List A) :: <A: Eq>`,
    /// the structural matcher plus per-parameter component-constraint names. `None`
    /// for a concrete witness (its `target_key` is `?`-free and matches directly).
    pub conditional: Option<ConditionalWitnessShape>,
}

/// Cross-module dispatch data for a conditional witness: the target shape with
/// parameter holes and, parallel to the holes, the component constraints each
/// hole's type must satisfy (`<A: Eq>` → `[["Eq"]]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalWitnessShape {
    pub pattern: WitnessPattern,
    pub param_bounds: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDiagnostic {
    pub kind: ImportDiagnosticKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportDiagnosticKind {
    /// An `import` appeared but the analysis has no base directory to resolve
    /// it against (e.g. `analyze(&str)` / REPL rather than `analyze_path`).
    NoBaseDirectory,
    /// Import path could not be turned into a supported file reference.
    UnsupportedImportForm {
        path: String,
    },
    /// The configured filesystem standard library could not be loaded.
    StdlibSetup {
        message: String,
    },
    /// The nearest `zutai.zti` package manifest or its local dependency graph
    /// is malformed.
    PackageSetup {
        message: String,
    },
    /// A dotted package import could not resolve through the importing
    /// package's declared dependency aliases and public module map.
    PackageResolution {
        path: String,
        message: String,
    },
    /// `import stdlib.<name>` named a module the configured standard library
    /// does not provide.
    UnknownStdlibModule {
        name: String,
    },
    FileNotFound {
        path: String,
    },
    ReadError {
        path: String,
        msg: String,
    },
    /// A `.zti` file failed to parse.
    ParseError {
        path: String,
        msg: String,
    },
    /// A `.zt` module imports (transitively) itself.
    ImportCycle {
        path: String,
    },
    /// A `.zt` module did not fully type-check, so it has no exportable value.
    ModuleHasErrors {
        path: String,
    },
    /// A `.zt` module's value cannot cross the import boundary (e.g. it is or
    /// contains a function or type value).
    UnsupportedExport {
        path: String,
        reason: &'static str,
    },
    /// Two distinct imported witnesses claim the same `(Constraint, Type)` pair.
    ConflictingWitness {
        constraint: String,
        target: String,
    },
    /// An import path is absolute or escapes the importing file's directory
    /// subtree (e.g. `"/tmp/x.zti"` or `"../../../etc/foo.zti"`).
    PathTraversal {
        path: String,
    },
}

enum Kind {
    Zti,
    Zt,
}

struct Resolver<'a> {
    base: Option<&'a Path>,
    types: FxHashMap<ImportKey, ImportedType>,
    provenance: FxHashMap<ImportKey, ImportedProvenance>,
    values: FxHashMap<ImportKey, zutai_im::Value>,
    modules: FxHashMap<ImportKey, Rc<Analysis>>,
    witnesses: Vec<WitnessExport>,
    witness_keys: FxHashMap<(String, String), PathBuf>,
    diagnostics: Vec<ImportDiagnostic>,
}

/// Resolve every distinct import declaration/internal import node in `hir` relative to `base`.
pub(crate) fn resolve_imports(
    hir: &HirFile,
    base: Option<&Path>,
    ctx: &mut ImportContext<'_>,
) -> ResolvedImports {
    let mut resolver = Resolver {
        base,
        types: FxHashMap::default(),
        provenance: FxHashMap::default(),
        values: FxHashMap::default(),
        modules: FxHashMap::default(),
        witnesses: Vec::new(),
        witness_keys: FxHashMap::default(),
        diagnostics: Vec::new(),
    };

    if let Some(message) = ctx.take_package_setup_error() {
        resolver.diag(
            ImportDiagnosticKind::PackageSetup { message },
            Span::default(),
        );
    }

    // Resolve each distinct source once, using the first span seen for diagnostics.
    let mut seen: FxHashSet<&HirImportSource> = FxHashSet::default();
    for (_, expr) in hir.expr_arena.iter() {
        let HirExprKind::Import(source) = &expr.kind else {
            continue;
        };
        if seen.insert(source) {
            resolver.resolve_one(source, expr.span, ctx);
        }
    }

    ResolvedImports {
        types: resolver.types,
        provenance: resolver.provenance,
        values: resolver.values,
        modules: resolver.modules,
        witnesses: resolver.witnesses,
        diagnostics: resolver.diagnostics,
    }
}

impl Resolver<'_> {
    fn resolve_one(&mut self, source: &HirImportSource, span: Span, ctx: &mut ImportContext) {
        // `import stdlib.<name>` resolves from the validated stdlib source set
        // (outside quoted-import subtree confinement). This is checked before
        // `relative_path` so `stdlib.stream` is not mistaken for `stem.ext`.
        if let HirImportSource::Path(parts) = source
            && parts.first().map(String::as_str) == Some("stdlib")
        {
            return self.resolve_stdlib(source, parts, span, ctx);
        }

        if ctx.packages.invalid_message().is_some() {
            return;
        }

        if let HirImportSource::Path(parts) = source
            && parts.len() >= 2
            && !matches!(parts.as_slice(), [_, extension] if matches!(extension.as_str(), "zt" | "zti"))
        {
            match ctx.packages.resolve(self.base, parts) {
                Ok(Some(loaded)) => {
                    ctx.packages.record_source(
                        &mut ctx.recorded_packages,
                        &loaded.key,
                        &loaded.contents,
                    );
                    let rel = loaded.display.clone();
                    return self.resolve_zt(
                        source,
                        LoadedSource {
                            key: loaded.key,
                            contents: loaded.contents,
                        },
                        &rel,
                        span,
                        ctx,
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    return self.diag(
                        ImportDiagnosticKind::PackageResolution {
                            path: parts.join("."),
                            message: error.to_string(),
                        },
                        span,
                    );
                }
            }
        }

        let rel = match relative_path(source) {
            Ok(rel) => rel,
            Err(kind) => return self.diag(kind, span),
        };

        let kind = match Path::new(&rel)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("zti") => Kind::Zti,
            Some("zt") => Kind::Zt,
            _ => {
                return self.diag(
                    ImportDiagnosticKind::UnsupportedImportForm { path: rel },
                    span,
                );
            }
        };

        let Some(base) = self.base else {
            return self.diag(ImportDiagnosticKind::NoBaseDirectory, span);
        };

        let loaded = match ctx.load(base, &rel) {
            Ok(loaded) => loaded,
            Err(LoadError::NotFound) => {
                return self.diag(ImportDiagnosticKind::FileNotFound { path: rel }, span);
            }
            Err(LoadError::Traversal) => {
                return self.diag(ImportDiagnosticKind::PathTraversal { path: rel }, span);
            }
            Err(LoadError::Read(err)) => return self.read_error(&rel, &err, span),
        };

        match kind {
            Kind::Zti => self.resolve_zti(source, &loaded.contents, &rel, span),
            Kind::Zt => self.resolve_zt(source, loaded, &rel, span, ctx),
        }
    }

    fn resolve_zti(&mut self, source: &HirImportSource, contents: &str, rel: &str, span: Span) {
        match zutai_im::parse_located(contents) {
            Ok(block) => {
                let provenance = block_provenance(&block);
                let value = zutai_im::Value::Block(block.value);
                let ty = imported_type(&value);
                self.types.insert(source.clone(), ty);
                self.provenance.insert(source.clone(), provenance);
                self.values.insert(source.clone(), value);
            }
            Err(err) => self.diag(
                ImportDiagnosticKind::ParseError {
                    path: rel.to_string(),
                    msg: err.to_string(),
                },
                span,
            ),
        }
    }

    fn resolve_zt(
        &mut self,
        source: &HirImportSource,
        loaded: LoadedSource,
        rel: &str,
        span: Span,
        ctx: &mut ImportContext,
    ) {
        if ctx.in_progress.iter().any(|p| p == &loaded.key) {
            return self.diag(
                ImportDiagnosticKind::ImportCycle {
                    path: rel.to_string(),
                },
                span,
            );
        }

        let module = match ctx.cache.get(&loaded.key) {
            Some(module) => module.clone(),
            None => {
                match self.analyze_zt(
                    &loaded.key,
                    loaded.key.parent(),
                    &loaded.contents,
                    rel,
                    span,
                    ctx,
                ) {
                    Some(module) => module,
                    None => return,
                }
            }
        };

        self.register_zt_module(source, module, rel, span);
    }

    /// `import stdlib.<name>` — resolve `<name>` against the configured standard
    /// library source set. Uses a synthetic cache key
    /// (`<stdlib>/<name>.zt`) so cycle detection and caching still apply without
    /// touching the filesystem.
    fn resolve_stdlib(
        &mut self,
        source: &HirImportSource,
        parts: &[String],
        span: Span,
        ctx: &mut ImportContext<'_>,
    ) {
        let name = match parts {
            [_, name] => name.as_str(),
            _ => {
                return self.diag(
                    ImportDiagnosticKind::UnsupportedImportForm {
                        path: parts.join("."),
                    },
                    span,
                );
            }
        };
        let Some(contents) = ctx.stdlib.source(name).map(str::to_owned) else {
            return self.diag(
                ImportDiagnosticKind::UnknownStdlibModule {
                    name: name.to_string(),
                },
                span,
            );
        };

        ctx.recorded_stdlib
            .insert(name.to_owned(), contents.clone());
        let key = PathBuf::from("<stdlib>").join(format!("{name}.zt"));
        let rel = format!("stdlib.{name}");
        if ctx.in_progress.iter().any(|p| p == &key) {
            return self.diag(ImportDiagnosticKind::ImportCycle { path: rel }, span);
        }
        let module = match ctx.cache.get(&key) {
            Some(module) => module.clone(),
            None => match self.analyze_zt(&key, key.parent(), &contents, &rel, span, ctx) {
                Some(module) => module,
                None => return,
            },
        };

        self.register_zt_module(source, module, &rel, span);
    }

    /// Recursively analyze a `.zt` module's source into a cached `Analysis`,
    /// pushing a diagnostic and returning `None` on cycle or module errors.
    /// `key` is the cache/cycle identity (a real canonical path or a synthetic
    /// stdlib key); `parent` is the directory used to resolve the module's own
    /// relative imports.
    fn analyze_zt(
        &mut self,
        key: &Path,
        parent: Option<&Path>,
        contents: &str,
        rel: &str,
        span: Span,
        ctx: &mut ImportContext<'_>,
    ) -> Option<Rc<Analysis>> {
        ctx.in_progress.push(key.to_path_buf());
        let analysis =
            crate::analyze_inner(contents, parent, Some(key), AnalysisOptions::default(), ctx);
        ctx.in_progress.pop();

        if analysis.blocking_diagnostics().next().is_some() || !analysis.is_thir_complete() {
            // A cycle is first detected on the back-edge, one module deeper;
            // propagate it so every level on the chain reports the cycle rather
            // than a vague "module has errors".
            let kind = if contains_cycle(&analysis) {
                ImportDiagnosticKind::ImportCycle {
                    path: rel.to_string(),
                }
            } else {
                ImportDiagnosticKind::ModuleHasErrors {
                    path: rel.to_string(),
                }
            };
            self.diag(kind, span);
            return None;
        }
        let module = Rc::new(analysis);
        ctx.cache.insert(key.to_path_buf(), module.clone());
        Some(module)
    }

    /// Type a resolved `.zt` module by its exported (final-expression) type and
    /// register it under `source` for THIR lowering and evaluation.
    fn register_zt_module(
        &mut self,
        source: &HirImportSource,
        module: Rc<Analysis>,
        rel: &str,
        span: Span,
    ) {
        // Type the import by exporting the module's final-expression type,
        // then enrich type-valued record fields with their denotations.
        let exported = {
            let Some(file) = module.thir.as_ref().and_then(|thir| thir.file.as_ref()) else {
                return self.diag(
                    ImportDiagnosticKind::ModuleHasErrors {
                        path: rel.to_string(),
                    },
                    span,
                );
            };
            let final_ty = file.expr_arena[file.final_expr].ty;
            zutai_thir::export_type(file, final_ty)
                .map(|ty| enrich_with_type_denotations(ty, file))
                .map(|ty| attach_type_only_exports(ty, file))
        };

        match exported {
            Ok(ty) => {
                self.merge_witnesses(&module.witness_exports, span);
                self.types.insert(source.clone(), ty);
                self.modules.insert(source.clone(), module);
            }
            Err(unsupported) => self.diag(
                ImportDiagnosticKind::UnsupportedExport {
                    path: rel.to_string(),
                    reason: unsupported.reason,
                },
                span,
            ),
        }
    }

    fn read_error(&mut self, rel: &str, err: &std::io::Error, span: Span) {
        self.diag(
            ImportDiagnosticKind::ReadError {
                path: rel.to_string(),
                msg: err.to_string(),
            },
            span,
        );
    }

    fn diag(&mut self, kind: ImportDiagnosticKind, span: Span) {
        self.diagnostics.push(ImportDiagnostic { kind, span });
    }

    fn merge_witnesses(&mut self, witnesses: &[WitnessExport], span: Span) {
        for witness in witnesses {
            let key = (witness.constraint.clone(), witness.target_key.clone());
            match self.witness_keys.get(&key) {
                Some(origin) if origin != &witness.origin => {
                    self.diag(
                        ImportDiagnosticKind::ConflictingWitness {
                            constraint: witness.constraint.clone(),
                            target: witness.target_display.clone(),
                        },
                        span,
                    );
                }
                Some(_) => {}
                None => {
                    self.witness_keys.insert(key, witness.origin.clone());
                    self.witnesses.push(witness.clone());
                }
            }
        }
    }
}

/// Whether `analysis` failed (at least in part) because of an import cycle.
fn contains_cycle(analysis: &Analysis) -> bool {
    analysis.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            crate::SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. })
        )
    })
}

pub(crate) fn merge_witness_exports(
    imported: Vec<WitnessExport>,
    local: Vec<WitnessExport>,
) -> (Vec<WitnessExport>, Vec<ImportDiagnostic>) {
    let mut merged = Vec::new();
    let mut diagnostics = Vec::new();
    let mut keys: FxHashMap<(String, String), PathBuf> = FxHashMap::default();
    for witness in imported.into_iter().chain(local) {
        let key = (witness.constraint.clone(), witness.target_key.clone());
        match keys.get(&key) {
            Some(origin) if origin != &witness.origin => {
                diagnostics.push(ImportDiagnostic {
                    kind: ImportDiagnosticKind::ConflictingWitness {
                        constraint: witness.constraint.clone(),
                        target: witness.target_display.clone(),
                    },
                    span: witness.span,
                });
            }
            Some(_) => {}
            None => {
                keys.insert(key, witness.origin.clone());
                merged.push(witness);
            }
        }
    }
    (merged, diagnostics)
}

pub(crate) fn local_witness_exports(
    hir: &HirFile,
    file: &ThirFile,
    origin: &Path,
) -> Vec<WitnessExport> {
    let mut out = Vec::new();
    for (_, decl) in file.decl_arena.iter() {
        let ThirDeclKind::Witness {
            constraint: Some(constraint),
            target,
            params,
            param_bounds,
            ..
        } = &decl.kind
        else {
            continue;
        };
        let Ok(exported_target) = zutai_thir::export_type(file, *target) else {
            continue;
        };
        let constraint = binding_name(hir, *constraint).to_string();
        let target_key = imported_type_key(&exported_target);
        // A parametric witness carries its type params as holes; record the
        // structural matcher and per-param component constraints so an importer
        // can dispatch it at a concrete call site (Phase B). A concrete witness
        // (`params` empty) needs none — its `target_key` matches directly.
        let conditional = if params.is_empty() {
            None
        } else {
            export_witness_pattern(file, *target, params).map(|pattern| {
                let param_bounds = param_bounds
                    .iter()
                    .map(|bounds| {
                        bounds
                            .iter()
                            .map(|b| binding_name(hir, *b).to_string())
                            .collect()
                    })
                    .collect();
                ConditionalWitnessShape {
                    pattern,
                    param_bounds,
                }
            })
        };
        out.push(WitnessExport {
            origin: origin.to_path_buf(),
            constraint,
            target_display: target_key.clone(),
            target_key,
            binding_id: decl.binding.0,
            span: decl.span,
            conditional,
        });
    }
    out
}

fn binding_name(hir: &HirFile, binding: BindingId) -> &str {
    &hir.bindings[binding.0 as usize].name
}

fn imported_type_key(ty: &ImportedType) -> String {
    match ty {
        ImportedType::Bool => "Bool".to_string(),
        ImportedType::Int => "Int".to_string(),
        ImportedType::Float => "Float".to_string(),
        ImportedType::FixedNum(fw) => fw.name().to_string(),
        ImportedType::Posit(spec) => spec.type_name(),
        ImportedType::Text => "Text".to_string(),
        ImportedType::Opaque(name) => format!("opaque:{name}"),
        ImportedType::Atom(name) => format!("#{name}"),
        ImportedType::List(inner) => format!("[{}]", imported_type_key(inner)),
        ImportedType::Optional(inner) => format!("{}?", imported_type_key(inner)),
        ImportedType::Maybe(inner) => format!("Maybe[{}]", imported_type_key(inner)),
        ImportedType::Record(fields) => {
            let mut parts: Vec<String> = fields
                .iter()
                .map(|field| {
                    let marker = if field.optional { "?:" } else { ":" };
                    format!("{}{}{}", field.name, marker, imported_type_key(&field.ty))
                })
                .collect();
            parts.sort();
            format!("{{{}}}", parts.join(","))
        }
        ImportedType::WithTypeExports { value, types } => {
            let mut parts: Vec<String> = types
                .iter()
                .map(|field| format!("{}:{}", field.name, imported_type_key(&field.ty)))
                .collect();
            parts.sort();
            format!("{}+types{{{}}}", imported_type_key(value), parts.join(","))
        }
        ImportedType::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    ImportedTupleItem::Named { name, ty } => {
                        format!("{}:{}", name, imported_type_key(ty))
                    }
                    ImportedTupleItem::Positional(ty) => imported_type_key(ty),
                })
                .collect();
            format!("({})", parts.join(","))
        }
        ImportedType::Union(variants) => {
            let parts: Vec<String> = variants
                .iter()
                .map(|variant| match &variant.payload {
                    Some(payload) => format!("{}({})", variant.name, imported_type_key(payload)),
                    None => variant.name.clone(),
                })
                .collect();
            format!("<{}>", parts.join("|"))
        }
        ImportedType::Function { from, to } => {
            format!("({}->{})", imported_type_key(from), imported_type_key(to))
        }
        ImportedType::Effect { base, ops, tail } => {
            let ops = ops
                .iter()
                .map(|op| {
                    format!(
                        "{}:{}->{}",
                        op.name,
                        imported_type_key(&op.param),
                        imported_type_key(&op.result)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let tail = match tail {
                ImportedRowTail::Closed => String::new(),
                ImportedRowTail::Open => "...".to_string(),
                ImportedRowTail::Param(id) => format!("...#{id}"),
            };
            format!("{}!{{{}{tail}}}", imported_type_key(base), ops)
        }
        ImportedType::Type(inner) => format!("Type({})", imported_type_key(inner)),
        ImportedType::TypeCon { params, body } => {
            let ps: Vec<String> = params.iter().map(|id| format!("'{id}")).collect();
            format!("\\<{}>{}", ps.join(","), imported_type_key(body))
        }
        ImportedType::ConApply { ctor, args } => {
            let parts: Vec<String> = args.iter().map(imported_type_key).collect();
            format!("{ctor}[{}]", parts.join(","))
        }
        ImportedType::TyVar(id) => format!("'{id}"),
        ImportedType::Unknown => "?".to_string(),
    }
}

/// Turn an import source into a relative path string.
fn relative_path(source: &HirImportSource) -> Result<String, ImportDiagnosticKind> {
    match source {
        HirImportSource::String(value) => {
            if Path::new(value).is_absolute() {
                return Err(ImportDiagnosticKind::PathTraversal {
                    path: value.clone(),
                });
            }
            Ok(value.clone())
        }
        // Bare shorthand `import config.zti` lexes to `["config", "zti"]`; only
        // the simple `stem.ext` form is resolved.  Anything else falls back to
        // the canonical quoted string form.
        HirImportSource::Path(parts) if parts.len() == 2 => {
            Ok(format!("{}.{}", parts[0], parts[1]))
        }
        HirImportSource::Path(parts) => Err(ImportDiagnosticKind::UnsupportedImportForm {
            path: parts.join("."),
        }),
    }
}

/// Derive the structural type of an imported `.zti` data value.
///
/// Blocks become records (all fields required), arrays become lists, atoms keep
/// their spelling.  A heterogeneous array yields a union of its distinct element
/// types; an empty array yields `Unknown` (a fresh inference variable in THIR).
fn imported_type(value: &zutai_im::Value) -> ImportedType {
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

fn immediate_span(span: zutai_im::ByteSpan) -> Span {
    Span::new(span.start, span.end)
}

fn block_provenance(block: &zutai_im::LocatedBlock) -> ImportedProvenance {
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

fn value_provenance(
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

fn array_element_type(items: &[zutai_im::Value]) -> ImportedType {
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
fn enrich_with_type_denotations(ty: ImportedType, file: &ThirFile) -> ImportedType {
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
fn attach_type_only_exports(ty: ImportedType, file: &ThirFile) -> ImportedType {
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
