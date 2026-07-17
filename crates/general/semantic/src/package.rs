use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub use zutai_package::{PortablePackage, PortablePackageGraph};

pub(crate) const MANIFEST_NAME: &str = zutai_package::MANIFEST_NAME;

#[derive(Clone, Debug)]
struct FilesystemPackage {
    id: String,
    source: zutai_package::PortablePackageSource,
    name: String,
    root: PathBuf,
    dependencies: BTreeMap<String, String>,
    modules: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FilesystemPackageGraph {
    root_package: String,
    packages: BTreeMap<String, FilesystemPackage>,
    manifest_sources: BTreeMap<PathBuf, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct PackageSetupError {
    pub message: String,
    pub path: Option<PathBuf>,
    pub span: Option<zutai_syntax::Span>,
    pub related: Vec<crate::SourceLocation>,
}

impl PackageSetupError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: None,
            span: None,
            related: Vec::new(),
        }
    }

    fn at(path: PathBuf, span: zutai_im::ByteSpan, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: Some(path),
            span: Some(immediate_span(span)),
            related: Vec::new(),
        }
    }

    fn with_related(mut self, related: crate::SourceLocation) -> Self {
        self.related.insert(0, related);
        self
    }
}

impl From<zutai_package::PackageError> for PackageSetupError {
    fn from(error: zutai_package::PackageError) -> Self {
        Self {
            message: error.message,
            path: error.path,
            span: error.span.map(immediate_span),
            related: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PackageGraph {
    None,
    Filesystem(FilesystemPackageGraph),
    Memory(PortablePackageGraph),
    Invalid(PackageSetupError),
}

pub(crate) struct ResolvedPackageSource {
    pub key: PathBuf,
    pub contents: String,
    pub display: String,
    pub filesystem_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PackageResolveError {
    UnknownAlias { alias: String },
    UnknownModule { alias: String, module: String },
    Read { path: String, message: String },
}

impl fmt::Display for PackageResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAlias { alias } => write!(f, "unknown package dependency alias `{alias}`"),
            Self::UnknownModule { alias, module } => write!(
                f,
                "unknown module `{module}` in package dependency `{alias}`"
            ),
            Self::Read { path, message } => {
                write!(f, "cannot read package module {path}: {message}")
            }
        }
    }
}

impl PackageGraph {
    pub(crate) fn discover(path: &Path) -> Self {
        match FilesystemPackageGraph::discover(path) {
            Ok(Some(graph)) => Self::Filesystem(graph),
            Ok(None) => Self::None,
            Err(error) => Self::Invalid(error),
        }
    }

    pub(crate) fn from_memory(graph: PortablePackageGraph) -> Self {
        if graph.root_package.is_none() || graph.packages.is_empty() {
            return if graph.root_package.is_none() && graph.packages.is_empty() {
                Self::None
            } else {
                Self::Invalid(PackageSetupError::new(
                    "portable package graph has an incomplete root",
                ))
            };
        }
        match zutai_package::validate_portable(&graph) {
            Ok(()) => Self::Memory(graph),
            Err(error) => Self::Invalid(error.into()),
        }
    }

    pub(crate) fn invalid_error(&self) -> Option<&PackageSetupError> {
        match self {
            Self::Invalid(error) => Some(error),
            _ => None,
        }
    }

    pub(crate) fn manifest_fingerprint(&self) -> crate::cache::Fingerprint {
        let mut parts: Vec<&[u8]> = vec![zutai_package::COMPILER_VERSION.as_bytes()];
        match self {
            Self::None | Self::Memory(_) => parts.push(b"none"),
            Self::Invalid(error) => {
                parts.push(b"invalid");
                parts.push(error.message.as_bytes());
            }
            Self::Filesystem(graph) => {
                parts.push(b"filesystem");
                for (path, source) in &graph.manifest_sources {
                    parts.push(path.as_os_str().as_encoded_bytes());
                    parts.push(source.as_bytes());
                }
            }
        }
        crate::cache::fingerprint_parts(parts)
    }

    pub(crate) fn graph_fingerprint(&self) -> crate::cache::Fingerprint {
        let portable;
        let graph = match self {
            Self::Filesystem(graph) => {
                portable = graph.portable_skeleton();
                &portable
            }
            Self::Memory(graph) => graph,
            Self::None | Self::Invalid(_) => {
                return crate::cache::fingerprint_parts([b"none".as_slice()]);
            }
        };
        let mut parts = Vec::<Vec<u8>>::new();
        parts.push(
            graph
                .root_package
                .as_deref()
                .unwrap_or_default()
                .as_bytes()
                .to_vec(),
        );
        for (id, package) in &graph.packages {
            parts.push(id.as_bytes().to_vec());
            parts.push(package.name.as_bytes().to_vec());
            for (alias, target) in &package.dependencies {
                parts.push(alias.as_bytes().to_vec());
                parts.push(target.as_bytes().to_vec());
            }
            for (module, path) in &package.modules {
                parts.push(module.as_bytes().to_vec());
                parts.push(path.as_bytes().to_vec());
            }
        }
        crate::cache::fingerprint_parts(parts.iter().map(Vec::as_slice))
    }

    pub(crate) fn resolve(
        &self,
        importer: Option<&Path>,
        parts: &[String],
    ) -> Result<Option<ResolvedPackageSource>, PackageResolveError> {
        let [alias, module_parts @ ..] = parts else {
            return Ok(None);
        };
        if module_parts.is_empty() || alias == "stdlib" {
            return Ok(None);
        }
        let module = module_parts.join(".");
        match self {
            Self::None | Self::Invalid(_) => Ok(None),
            Self::Filesystem(graph) => graph.resolve(importer, alias, &module).map(Some),
            Self::Memory(graph) => resolve_memory(graph, importer, alias, &module).map(Some),
        }
    }

    pub(crate) fn portable_skeleton(&self) -> PortablePackageGraph {
        match self {
            Self::Filesystem(graph) => graph.portable_skeleton(),
            Self::Memory(graph) => graph.clone(),
            Self::None | Self::Invalid(_) => PortablePackageGraph::default(),
        }
    }

    pub(crate) fn package_source(
        &self,
        base: &Path,
        rel: &str,
    ) -> Option<Result<ResolvedPackageSource, PackageResolveError>> {
        match self {
            Self::Memory(graph) => load_memory_relative(graph, base, rel),
            _ => None,
        }
    }

    pub(crate) fn stable_source_key(&self, path: &Path) -> Option<PathBuf> {
        let Self::Filesystem(graph) = self else {
            return None;
        };
        let package = graph.owner(path)?;
        let relative = path.strip_prefix(&package.root).ok()?;
        Some(synthetic_key(&package.id, &path_key(relative)))
    }

    pub(crate) fn record_public_modules(
        &self,
        portable: &mut PortablePackageGraph,
        source_paths: &mut BTreeMap<PathBuf, PathBuf>,
    ) {
        let Self::Filesystem(graph) = self else {
            return;
        };
        for (id, package) in &graph.packages {
            let Some(target) = portable.packages.get_mut(id) else {
                continue;
            };
            for relative in package.modules.values() {
                let path = package.root.join(relative);
                let Ok(canonical) = fs::canonicalize(&path) else {
                    continue;
                };
                let Ok(contents) = fs::read_to_string(&canonical) else {
                    continue;
                };
                target
                    .sources
                    .insert(path_key(Path::new(relative)), contents);
                source_paths.insert(synthetic_key(id, relative), canonical);
            }
        }
    }

    pub(crate) fn record_source(
        &self,
        portable: &mut PortablePackageGraph,
        path: &Path,
        contents: &str,
    ) {
        let Self::Filesystem(graph) = self else {
            return;
        };
        let Some(package) = graph.owner(path) else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&package.root) else {
            return;
        };
        if let Some(target) = portable.packages.get_mut(&package.id) {
            target
                .sources
                .insert(path_key(relative), contents.to_owned());
        }
    }
}

impl FilesystemPackageGraph {
    fn discover(path: &Path) -> Result<Option<Self>, PackageSetupError> {
        let start = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let Some(root) = find_manifest_root(start) else {
            return Ok(None);
        };
        let root = fs::canonicalize(&root).map_err(|error| {
            PackageSetupError::new(format!(
                "cannot resolve package root {}: {error}",
                root.display()
            ))
        })?;
        let root_manifest_path = root.join(MANIFEST_NAME);
        let root_manifest_source = fs::read_to_string(&root_manifest_path).map_err(|error| {
            PackageSetupError::new(format!(
                "cannot read package manifest {}: {error}",
                root_manifest_path.display()
            ))
        })?;
        let root_manifest =
            zutai_package::parse_manifest(&root_manifest_path, &root_manifest_source)?;
        if root_manifest.format_version == 2 {
            #[cfg(target_arch = "wasm32")]
            return Err(PackageSetupError::new(
                "manifest format 2 requires a portable prepared package graph in Wasm",
            ));
            #[cfg(not(target_arch = "wasm32"))]
            {
                let prepared = zutai_package::acquire::prepare_graph(&root, None)?;
                let packages = prepared
                    .graph
                    .packages
                    .iter()
                    .map(|(id, package)| {
                        let root = prepared.roots[id].clone();
                        (
                            id.clone(),
                            FilesystemPackage {
                                id: id.clone(),
                                source: package.source,
                                name: package.name.clone(),
                                root,
                                dependencies: package.dependencies.clone(),
                                modules: package.modules.clone(),
                            },
                        )
                    })
                    .collect();
                return Ok(Some(Self {
                    root_package: prepared
                        .graph
                        .root_package
                        .expect("prepared graph has root package"),
                    packages,
                    manifest_sources: prepared.manifests,
                }));
            }
        }

        let mut packages = BTreeMap::new();
        let mut roots = BTreeMap::new();
        let mut active = Vec::new();
        let mut manifest_sources = BTreeMap::new();
        let root_id = load_v1_package(
            &root,
            &root,
            &mut packages,
            &mut roots,
            &mut active,
            &mut manifest_sources,
        )?;
        Ok(Some(Self {
            root_package: root_id,
            packages,
            manifest_sources,
        }))
    }

    fn owner(&self, path: &Path) -> Option<&FilesystemPackage> {
        self.packages
            .values()
            .filter(|package| path.starts_with(&package.root))
            .max_by_key(|package| package.root.components().count())
    }

    fn resolve(
        &self,
        importer: Option<&Path>,
        alias: &str,
        module: &str,
    ) -> Result<ResolvedPackageSource, PackageResolveError> {
        let owner = importer
            .and_then(|path| fs::canonicalize(path).ok())
            .and_then(|path| self.owner(&path))
            .or_else(|| self.packages.get(&self.root_package))
            .expect("filesystem package graph always contains its root package");
        let target_id =
            owner
                .dependencies
                .get(alias)
                .ok_or_else(|| PackageResolveError::UnknownAlias {
                    alias: alias.to_owned(),
                })?;
        let target = &self.packages[target_id];
        let relative =
            target
                .modules
                .get(module)
                .ok_or_else(|| PackageResolveError::UnknownModule {
                    alias: alias.to_owned(),
                    module: module.to_owned(),
                })?;
        let unresolved = target.root.join(relative);
        let path = fs::canonicalize(&unresolved).map_err(|error| PackageResolveError::Read {
            path: unresolved.display().to_string(),
            message: error.to_string(),
        })?;
        if !path.starts_with(&target.root) {
            return Err(PackageResolveError::Read {
                path: path.display().to_string(),
                message: "module escapes package root".to_owned(),
            });
        }
        let contents = fs::read_to_string(&path).map_err(|error| PackageResolveError::Read {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        Ok(ResolvedPackageSource {
            key: path.clone(),
            contents,
            display: format!("{alias}.{module}"),
            filesystem_path: Some(path),
        })
    }

    fn portable_skeleton(&self) -> PortablePackageGraph {
        PortablePackageGraph {
            root_package: Some(self.root_package.clone()),
            packages: self
                .packages
                .iter()
                .map(|(id, package)| {
                    (
                        id.clone(),
                        PortablePackage {
                            source: package.source,
                            name: package.name.clone(),
                            dependencies: package.dependencies.clone(),
                            modules: package.modules.clone(),
                            sources: BTreeMap::new(),
                        },
                    )
                })
                .collect(),
        }
    }
}

fn load_v1_package(
    root_package: &Path,
    root: &Path,
    packages: &mut BTreeMap<String, FilesystemPackage>,
    roots: &mut BTreeMap<PathBuf, String>,
    active: &mut Vec<PathBuf>,
    manifest_sources: &mut BTreeMap<PathBuf, String>,
) -> Result<String, PackageSetupError> {
    let root = fs::canonicalize(root).map_err(|error| {
        PackageSetupError::new(format!(
            "cannot resolve package path {}: {error}",
            root.display()
        ))
    })?;
    if let Some(id) = roots.get(&root) {
        return Ok(id.clone());
    }
    if active.contains(&root) {
        let mut cycle = active
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        cycle.push(root.display().to_string());
        return Err(PackageSetupError::new(format!(
            "package dependency cycle: {}",
            cycle.join(" -> ")
        )));
    }
    active.push(root.clone());
    let manifest_path = root.join(MANIFEST_NAME);
    let source = fs::read_to_string(&manifest_path).map_err(|error| {
        PackageSetupError::new(format!(
            "cannot read package manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    manifest_sources.insert(manifest_path.clone(), source.clone());
    let manifest = zutai_package::parse_manifest(&manifest_path, &source)?;
    if manifest.format_version != 1 {
        return Err(PackageSetupError::new(format!(
            "manifest format changed below format-1 package root {}",
            root_package.display()
        )));
    }
    let relative = pathdiff::diff_paths(&root, root_package)
        .ok_or_else(|| PackageSetupError::new("cannot derive root-relative package identity"))?;
    let identity = if relative.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        path_key(&relative)
    };
    let source_identity = zutai_package::LockedSource::Path { path: identity };
    let id = zutai_package::package_id(&source_identity);
    let mut dependencies = BTreeMap::new();
    for dependency in manifest.dependencies {
        if dependencies.contains_key(&dependency.alias) {
            return Err(PackageSetupError::at(
                manifest_path.clone(),
                dependency.alias_span,
                format!("duplicate dependency alias {:?}", dependency.alias),
            ));
        }
        if dependency.alias == "stdlib" {
            return Err(PackageSetupError::new(
                "dependency alias `stdlib` is reserved by the toolchain",
            ));
        }
        let zutai_package::ManifestSource::Path { path } = dependency.source else {
            return Err(PackageSetupError::new(
                "manifest format 1 cannot contain Git dependencies",
            ));
        };
        let dependency_root = fs::canonicalize(root.join(&path)).map_err(|error| {
            PackageSetupError::new(format!(
                "cannot resolve dependency {:?} at {}: {error}",
                dependency.alias,
                root.join(&path).display()
            ))
        })?;
        if !dependency_root.join(MANIFEST_NAME).is_file() {
            return Err(PackageSetupError::new(format!(
                "dependency {:?} has no {MANIFEST_NAME} at {}",
                dependency.alias,
                dependency_root.display()
            )));
        }
        let target = load_v1_package(
            root_package,
            &dependency_root,
            packages,
            roots,
            active,
            manifest_sources,
        )
        .map_err(|error| {
            let label = if error.message.contains("package dependency cycle") {
                "package dependency cycle continues here"
            } else {
                "dependency declared here"
            };
            error.with_related(crate::SourceLocation {
                path: manifest_path.clone(),
                span: immediate_span(dependency.alias_span),
                label: label.to_owned(),
            })
        })?;
        dependencies.insert(dependency.alias, target);
    }
    active.pop();
    roots.insert(root.clone(), id.clone());
    packages.insert(
        id.clone(),
        FilesystemPackage {
            id: id.clone(),
            source: zutai_package::PortablePackageSource::Path,
            name: manifest.name,
            root,
            dependencies,
            modules: manifest.modules,
        },
    );
    Ok(id)
}

fn find_manifest_root(start: &Path) -> Option<PathBuf> {
    let mut current = fs::canonicalize(start).ok()?;
    loop {
        if current.join(MANIFEST_NAME).is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn immediate_span(span: zutai_im::ByteSpan) -> zutai_syntax::Span {
    zutai_syntax::Span {
        start: span.start as u32,
        end: span.end as u32,
    }
}

fn path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn synthetic_key(package: &str, path: &str) -> PathBuf {
    PathBuf::from("<package>").join(package).join(path)
}

fn resolve_memory(
    graph: &PortablePackageGraph,
    importer: Option<&Path>,
    alias: &str,
    module: &str,
) -> Result<ResolvedPackageSource, PackageResolveError> {
    let owner = memory_owner(graph, importer)
        .or(graph.root_package.as_deref())
        .expect("validated portable graph has root");
    let owner_package = &graph.packages[owner];
    let target_id =
        owner_package
            .dependencies
            .get(alias)
            .ok_or_else(|| PackageResolveError::UnknownAlias {
                alias: alias.to_owned(),
            })?;
    let target = &graph.packages[target_id];
    let path = target
        .modules
        .get(module)
        .ok_or_else(|| PackageResolveError::UnknownModule {
            alias: alias.to_owned(),
            module: module.to_owned(),
        })?;
    let contents = target
        .sources
        .get(path)
        .cloned()
        .ok_or_else(|| PackageResolveError::Read {
            path: format!("{target_id}/{path}"),
            message: "source is missing from the portable package graph".to_owned(),
        })?;
    Ok(ResolvedPackageSource {
        key: synthetic_key(target_id, path),
        contents,
        display: format!("{alias}.{module}"),
        filesystem_path: None,
    })
}

fn memory_owner<'a>(graph: &'a PortablePackageGraph, importer: Option<&Path>) -> Option<&'a str> {
    let importer = importer?;
    let mut components = importer.components();
    if components.next()?.as_os_str() != "<package>" {
        return graph.root_package.as_deref();
    }
    let id = components.next()?.as_os_str().to_str()?;
    graph.packages.get_key_value(id).map(|(id, _)| id.as_str())
}

fn load_memory_relative(
    graph: &PortablePackageGraph,
    base: &Path,
    rel: &str,
) -> Option<Result<ResolvedPackageSource, PackageResolveError>> {
    let owner = memory_owner(graph, Some(base))?;
    let prefix = Path::new("<package>").join(owner);
    let relative_base = base.strip_prefix(&prefix).ok()?;
    let normalized = normalize_join(relative_base, rel).ok_or_else(|| PackageResolveError::Read {
        path: rel.to_owned(),
        message: "relative import escapes package root".to_owned(),
    });
    Some(normalized.and_then(|path| {
        let key = path_key(&path);
        let contents = graph.packages[owner]
            .sources
            .get(&key)
            .cloned()
            .ok_or_else(|| PackageResolveError::Read {
                path: format!("{owner}/{key}"),
                message: "source is missing from the portable package graph".to_owned(),
            })?;
        Ok(ResolvedPackageSource {
            key: synthetic_key(owner, &key),
            contents,
            display: rel.to_owned(),
            filesystem_path: None,
        })
    }))
}

fn normalize_join(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.contains('\0') || rel.contains('\\') || Path::new(rel).is_absolute() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("zutai-package-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(name: &str, modules: &str, dependencies: &str) -> String {
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
            zutai_package::COMPILER_VERSION
        )
    }

    #[test]
    fn resolves_local_dependency_module() {
        let temp = Temp::new();
        let app = temp.0.join("app");
        let math = temp.0.join("math");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::create_dir_all(math.join("src")).unwrap();
        fs::write(
            app.join(MANIFEST_NAME),
            manifest("app", "", "{ alias = \"math\"; path = \"../math\"; }"),
        )
        .unwrap();
        fs::write(
            math.join(MANIFEST_NAME),
            manifest(
                "math",
                "{ name = \"vector\"; path = \"src/vector.zt\"; }",
                "",
            ),
        )
        .unwrap();
        fs::write(app.join("src/main.zt"), "import math.vector").unwrap();
        fs::write(math.join("src/vector.zt"), "{ answer = 42; }").unwrap();

        let graph = FilesystemPackageGraph::discover(&app.join("src/main.zt"))
            .unwrap()
            .unwrap();
        let resolved = graph
            .resolve(Some(&app.join("src")), "math", "vector")
            .unwrap();
        assert!(resolved.contents.contains("42"));
        assert_eq!(resolved.display, "math.vector");
    }

    #[test]
    fn rejects_incomplete_portable_dependency_graph() {
        let root_source = zutai_package::LockedSource::Path {
            path: ".".to_owned(),
        };
        let root = zutai_package::package_id(&root_source);
        let graph = PortablePackageGraph {
            root_package: Some(root.clone()),
            packages: BTreeMap::from([(
                root,
                PortablePackage {
                    source: zutai_package::PortablePackageSource::Path,
                    name: "app".to_owned(),
                    dependencies: BTreeMap::from([("missing".to_owned(), "nope".to_owned())]),
                    modules: BTreeMap::new(),
                    sources: BTreeMap::new(),
                },
            )]),
        };
        let PackageGraph::Invalid(error) = PackageGraph::from_memory(graph) else {
            panic!("invalid portable graph was accepted");
        };
        assert!(
            error.message.contains("missing package"),
            "{}",
            error.message
        );
    }
}
