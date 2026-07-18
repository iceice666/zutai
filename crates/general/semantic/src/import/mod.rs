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

use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::cache::{
    AnalysisCache, CacheDependency, CacheDependencyKind, CacheDependencySource, CachedAnalysis,
    Fingerprint, ModuleCacheSlot, fingerprint_parts, fingerprint_text, module_fingerprint,
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
    fingerprints: FxHashMap<PathBuf, Fingerprint>,
    analysis_cache: Option<&'a AnalysisCache>,
    dependency_stack: Vec<Vec<CacheDependency>>,
    options: AnalysisOptions,
    source_backend: SourceBackend<'a>,
    stdlib: Rc<StdlibSources>,
    recorded_stdlib: BTreeMap<String, String>,
    packages: PackageGraph,
    recorded_packages: PortablePackageGraph,
    recorded_source_paths: BTreeMap<PathBuf, PathBuf>,
    package_setup_reported: bool,
    context_fingerprint: Fingerprint,
}

impl<'a> ImportContext<'a> {
    pub(crate) fn new(
        stdlib: Rc<StdlibSources>,
        analysis_cache: Option<&'a AnalysisCache>,
        options: AnalysisOptions,
    ) -> Self {
        let context_fingerprint = context_fingerprint(&stdlib, &PackageGraph::None);
        Self {
            in_progress: Vec::new(),
            cache: FxHashMap::default(),
            fingerprints: FxHashMap::default(),
            analysis_cache,
            dependency_stack: Vec::new(),
            options,
            source_backend: SourceBackend::Filesystem {
                confinement_root: None,
                recording_root: None,
                recorded: BTreeMap::new(),
            },
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages: PackageGraph::None,
            recorded_packages: PortablePackageGraph::default(),
            recorded_source_paths: BTreeMap::new(),
            package_setup_reported: false,
            context_fingerprint,
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
    pub(crate) fn with_root(
        path: &Path,
        stdlib: Rc<StdlibSources>,
        analysis_cache: Option<&'a AnalysisCache>,
        options: AnalysisOptions,
    ) -> Self {
        let mut ctx = Self::new(stdlib, analysis_cache, options);
        ctx.packages = PackageGraph::discover(path);
        ctx.refresh_context_fingerprint();
        if let Ok(canonical) = std::fs::canonicalize(path) {
            ctx.in_progress.push(canonical);
        }
        ctx
    }

    pub(crate) fn with_base(
        base: Option<&Path>,
        stdlib: Rc<StdlibSources>,
        analysis_cache: Option<&'a AnalysisCache>,
        options: AnalysisOptions,
    ) -> Self {
        let mut ctx = Self::new(stdlib, analysis_cache, options);
        if let Some(base) = base {
            ctx.packages = PackageGraph::discover(base);
            ctx.refresh_context_fingerprint();
        }
        ctx
    }
    pub(crate) fn with_recording_root(
        path: &Path,
        stdlib: Rc<StdlibSources>,
        analysis_cache: Option<&'a AnalysisCache>,
        options: AnalysisOptions,
    ) -> Self {
        let canonical = std::fs::canonicalize(path).ok();
        let recording_root = canonical
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let packages = PackageGraph::discover(path);
        let context_fingerprint = context_fingerprint(&stdlib, &packages);
        let mut recorded_packages = packages.portable_skeleton();
        let mut recorded_source_paths = BTreeMap::new();
        packages.record_public_modules(&mut recorded_packages, &mut recorded_source_paths);
        Self {
            in_progress: canonical.into_iter().collect(),
            cache: FxHashMap::default(),
            fingerprints: FxHashMap::default(),
            analysis_cache,
            dependency_stack: Vec::new(),
            options,
            source_backend: SourceBackend::Filesystem {
                confinement_root: None,
                recording_root,
                recorded: BTreeMap::new(),
            },
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages,
            recorded_packages,
            recorded_source_paths,
            package_setup_reported: false,
            context_fingerprint,
        }
    }
    pub(crate) fn with_explicit_recording_root(
        path: &Path,
        source_root: &Path,
        stdlib: Rc<StdlibSources>,
        analysis_cache: Option<&'a AnalysisCache>,
        options: AnalysisOptions,
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
        let context_fingerprint = context_fingerprint(&stdlib, &packages);
        let mut recorded_packages = packages.portable_skeleton();
        let mut recorded_source_paths = BTreeMap::new();
        packages.record_public_modules(&mut recorded_packages, &mut recorded_source_paths);
        Ok(Self {
            in_progress: vec![canonical],
            cache: FxHashMap::default(),
            fingerprints: FxHashMap::default(),
            analysis_cache,
            dependency_stack: Vec::new(),
            options,
            source_backend: SourceBackend::Filesystem {
                confinement_root: Some(source_root.clone()),
                recording_root: Some(source_root),
                recorded: BTreeMap::new(),
            },
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages,
            recorded_packages,
            recorded_source_paths,
            package_setup_reported: false,
            context_fingerprint,
        })
    }

    pub(crate) fn with_memory(
        sources: &'a BTreeMap<String, String>,
        entry: &Path,
        stdlib: Rc<StdlibSources>,
        packages: PortablePackageGraph,
        analysis_cache: Option<&'a AnalysisCache>,
        options: AnalysisOptions,
    ) -> Self {
        let packages = PackageGraph::from_memory(packages);
        let context_fingerprint = context_fingerprint(&stdlib, &packages);
        Self {
            in_progress: vec![entry.to_path_buf()],
            cache: FxHashMap::default(),
            fingerprints: FxHashMap::default(),
            analysis_cache,
            dependency_stack: Vec::new(),
            options,
            source_backend: SourceBackend::Memory(sources),
            stdlib,
            recorded_stdlib: BTreeMap::new(),
            packages,
            recorded_packages: PortablePackageGraph::default(),
            recorded_source_paths: BTreeMap::new(),
            package_setup_reported: false,
            context_fingerprint,
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

    pub(crate) fn take_recorded_source_paths(&mut self) -> BTreeMap<PathBuf, PathBuf> {
        std::mem::take(&mut self.recorded_source_paths)
    }

    pub(crate) fn stdlib(&self) -> &StdlibSources {
        &self.stdlib
    }

    fn refresh_context_fingerprint(&mut self) {
        self.context_fingerprint = context_fingerprint(&self.stdlib, &self.packages);
    }

    fn begin_module(&mut self) {
        self.dependency_stack.push(Vec::new());
    }

    fn finish_module(&mut self) -> Vec<CacheDependency> {
        self.dependency_stack
            .pop()
            .expect("module dependency collection is balanced")
    }

    fn record_dependency(&mut self, dependency: CacheDependency) {
        if let Some(dependencies) = self.dependency_stack.last_mut() {
            dependencies.push(dependency);
        }
    }

    fn try_cached_module(
        &mut self,
        key: &Path,
        contents: &str,
    ) -> Option<(Rc<Analysis>, Fingerprint)> {
        let cache = self.analysis_cache?;
        let slot = ModuleCacheSlot::new(key.to_path_buf(), self.options);
        let cached = match cache.get(&slot) {
            Some(cached) => cached,
            None => {
                cache.record_miss();
                return None;
            }
        };
        if cached.source != fingerprint_text(contents)
            || cached.context != self.context_fingerprint
            || cached
                .dependencies
                .iter()
                .any(|dependency| !self.dependency_is_current(dependency))
        {
            cache.record_miss();
            return None;
        }
        let dependencies = cached.dependencies.clone();
        self.replay_cached_dependencies(&dependencies)?;
        self.cache
            .insert(key.to_path_buf(), cached.analysis.clone());
        self.fingerprints
            .insert(key.to_path_buf(), cached.fingerprint);
        cache.record_hit();
        Some((cached.analysis, cached.fingerprint))
    }

    fn replay_cached_dependencies(&mut self, dependencies: &[CacheDependency]) -> Option<()> {
        for dependency in dependencies {
            let (contents, key) = self.load_dependency_source(&dependency.source)?;
            if let CacheDependencySource::Stdlib { name } = &dependency.source {
                self.recorded_stdlib.insert(name.clone(), contents.clone());
            } else {
                self.record_loaded_source(&key, &contents);
            }
            if dependency.kind == CacheDependencyKind::Module {
                let cache = self.analysis_cache?;
                let slot = ModuleCacheSlot::new(key, self.options);
                let cached = cache.get(&slot)?;
                self.replay_cached_dependencies(&cached.dependencies)?;
            }
        }
        Some(())
    }

    fn record_loaded_source(&mut self, key: &Path, contents: &str) {
        self.packages
            .record_source(&mut self.recorded_packages, key, contents);
        if let SourceBackend::Filesystem {
            recording_root: Some(root),
            recorded,
            ..
        } = &mut self.source_backend
            && let Ok(relative) = key.strip_prefix(root)
        {
            recorded.insert(path_to_bundle_key(relative), contents.to_owned());
        }
        if let Ok(path) = std::fs::canonicalize(key) {
            let stable = self
                .packages
                .stable_source_key(&path)
                .unwrap_or_else(|| key.to_path_buf());
            self.recorded_source_paths.insert(stable, path);
        }
    }

    fn dependency_is_current(&self, dependency: &CacheDependency) -> bool {
        let Some((contents, key)) = self.load_dependency_source(&dependency.source) else {
            return false;
        };
        match dependency.kind {
            CacheDependencyKind::Data => fingerprint_text(&contents) == dependency.fingerprint,
            CacheDependencyKind::Module => {
                let Some(cache) = self.analysis_cache else {
                    return false;
                };
                let slot = ModuleCacheSlot::new(key, self.options);
                let Some(cached) = cache.get(&slot) else {
                    return false;
                };
                cached.source == fingerprint_text(&contents)
                    && cached.context == self.context_fingerprint
                    && cached.fingerprint == dependency.fingerprint
                    && cached
                        .dependencies
                        .iter()
                        .all(|child| self.dependency_is_current(child))
            }
        }
    }

    fn load_dependency_source(&self, source: &CacheDependencySource) -> Option<(String, PathBuf)> {
        match source {
            CacheDependencySource::Relative { base, path } => {
                let loaded =
                    load_relative_source(&self.source_backend, &self.packages, base, path)?;
                Some((loaded.contents, loaded.key))
            }
            CacheDependencySource::Stdlib { name } => self.stdlib.source(name).map(|contents| {
                (
                    contents.to_owned(),
                    PathBuf::from("<stdlib>").join(format!("{name}.zt")),
                )
            }),
            CacheDependencySource::Package { importer, parts } => self
                .packages
                .resolve(importer.as_deref(), parts)
                .ok()
                .flatten()
                .map(|loaded| (loaded.contents, loaded.key)),
        }
    }

    fn store_cached_module(
        &mut self,
        key: &Path,
        contents: &str,
        dependencies: Vec<CacheDependency>,
        analysis: Rc<Analysis>,
    ) -> Fingerprint {
        let slot = ModuleCacheSlot::new(key.to_path_buf(), self.options);
        let source = fingerprint_text(contents);
        let fingerprint =
            module_fingerprint(&slot, source, self.context_fingerprint, &dependencies);
        self.cache.insert(key.to_path_buf(), analysis.clone());
        self.fingerprints.insert(key.to_path_buf(), fingerprint);
        if let Some(cache) = self.analysis_cache {
            cache.insert(
                slot,
                CachedAnalysis {
                    source,
                    context: self.context_fingerprint,
                    fingerprint,
                    dependencies,
                    analysis,
                },
            );
        }
        fingerprint
    }

    fn cached_fingerprint(&self, key: &Path) -> Option<Fingerprint> {
        self.fingerprints.get(key).copied()
    }
    fn take_package_setup_error(&mut self) -> Option<crate::package::PackageSetupError> {
        if self.package_setup_reported {
            return None;
        }
        let error = self.packages.invalid_error()?.clone();
        self.package_setup_reported = true;
        Some(error)
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

fn context_fingerprint(stdlib: &StdlibSources, packages: &PackageGraph) -> Fingerprint {
    let manifest = packages.manifest_fingerprint();
    let graph = packages.graph_fingerprint();
    let mut parts: Vec<&[u8]> = vec![
        env!("CARGO_PKG_VERSION").as_bytes(),
        stdlib.compiler_compatibility().as_bytes(),
        &manifest,
        &graph,
    ];
    for module in stdlib.modules() {
        parts.push(module.name().as_bytes());
        parts.push(module.source().as_bytes());
        parts.push(match module.visibility() {
            crate::stdlib::StdlibVisibility::Ambient => b"ambient",
            crate::stdlib::StdlibVisibility::Explicit => b"explicit",
        });
    }
    fingerprint_parts(parts)
}
fn load_relative_source(
    backend: &SourceBackend<'_>,
    packages: &PackageGraph,
    base: &Path,
    rel: &str,
) -> Option<LoadedSource> {
    if let Some(result) = packages.package_source(base, rel) {
        return result.ok().map(|source| LoadedSource {
            key: source.key,
            contents: source.contents,
        });
    }
    match backend {
        SourceBackend::Filesystem {
            confinement_root, ..
        } => {
            let base_dir = if base.as_os_str().is_empty() {
                Path::new(".")
            } else {
                base
            };
            let canonical_base = std::fs::canonicalize(base_dir).ok()?;
            let canonical = std::fs::canonicalize(base_dir.join(rel)).ok()?;
            let allowed_root = confinement_root.as_deref().unwrap_or(&canonical_base);
            if !canonical.starts_with(allowed_root) {
                return None;
            }
            let contents = std::fs::read_to_string(&canonical).ok()?;
            Some(LoadedSource {
                key: canonical,
                contents,
            })
        }
        SourceBackend::Memory(sources) => {
            let key = normalize_memory_join(base, rel)?;
            let contents = sources.get(&path_to_bundle_key(&key))?.clone();
            Some(LoadedSource { key, contents })
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

mod diagnostics;
mod imported_type;
mod resolve;

pub use diagnostics::{
    ConditionalWitnessShape, ImportDiagnostic, ImportDiagnosticKind, WitnessExport,
};
pub(crate) use diagnostics::{
    local_witness_exports, merge_witness_exports, witness_registry_covers,
};
pub(crate) use imported_type::{
    attach_type_only_exports, block_provenance, enrich_with_type_denotations, imported_type,
};
pub(crate) use resolve::{binding_name, resolve_imports};
