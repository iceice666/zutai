use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use zutai_im::{Block, LocatedChildren, Value};

pub(crate) const MANIFEST_NAME: &str = "zutai.zti";
const FORMAT_VERSION: i64 = 1;
const COMPILER_COMPATIBILITY: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePackageGraph {
    pub root_package: Option<String>,
    pub packages: BTreeMap<String, PortablePackage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePackage {
    pub dependencies: BTreeMap<String, String>,
    pub modules: BTreeMap<String, String>,
    pub sources: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct FilesystemPackage {
    name: String,
    root: PathBuf,
    dependencies: BTreeMap<String, PathBuf>,
    modules: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FilesystemPackageGraph {
    root_package: String,
    packages: BTreeMap<PathBuf, FilesystemPackage>,
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
    fn new(message: String) -> Self {
        Self {
            message,
            path: None,
            span: None,
            related: Vec::new(),
        }
    }

    fn at(path: PathBuf, span: zutai_im::ByteSpan, message: String) -> Self {
        Self {
            message,
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
            Self::UnknownModule { alias, module } => {
                write!(
                    f,
                    "unknown module `{module}` in package dependency `{alias}`"
                )
            }
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
                    "portable package graph has an incomplete root".to_owned(),
                ))
            };
        }
        match validate_portable(&graph) {
            Ok(()) => Self::Memory(graph),
            Err(error) => Self::Invalid(PackageSetupError::new(error)),
        }
    }

    pub(crate) fn invalid_error(&self) -> Option<&PackageSetupError> {
        match self {
            Self::Invalid(error) => Some(error),
            _ => None,
        }
    }

    pub(crate) fn manifest_fingerprint(&self) -> crate::cache::Fingerprint {
        let mut parts: Vec<&[u8]> = vec![COMPILER_COMPATIBILITY.as_bytes()];
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
        let parts = graph.fingerprint_parts();
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
        Some(synthetic_key(&package.name, &path_key(relative)))
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
        if let Some(target) = portable.packages.get_mut(&package.name) {
            target
                .sources
                .insert(path_key(relative), contents.to_owned());
        }
    }
}

impl PortablePackageGraph {
    fn fingerprint_parts(&self) -> Vec<Vec<u8>> {
        let mut parts = Vec::new();
        parts.push(
            self.root_package
                .as_deref()
                .unwrap_or_default()
                .as_bytes()
                .to_vec(),
        );
        for (name, package) in &self.packages {
            parts.push(name.as_bytes().to_vec());
            for (alias, target) in &package.dependencies {
                parts.push(alias.as_bytes().to_vec());
                parts.push(target.as_bytes().to_vec());
            }
            for (module, path) in &package.modules {
                parts.push(module.as_bytes().to_vec());
                parts.push(path.as_bytes().to_vec());
            }
        }
        parts
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
        let mut packages = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut stack = Vec::new();
        let mut manifest_sources = BTreeMap::new();
        let root_name = load_package(
            &root,
            &mut packages,
            &mut names,
            &mut stack,
            &mut manifest_sources,
        )?;
        Ok(Some(Self {
            root_package: root_name,
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
            .or_else(|| {
                self.packages
                    .values()
                    .find(|package| package.name == self.root_package)
            })
            .expect("filesystem package graph always contains its root package");
        let target_root =
            owner
                .dependencies
                .get(alias)
                .ok_or_else(|| PackageResolveError::UnknownAlias {
                    alias: alias.to_owned(),
                })?;
        let target = self
            .packages
            .get(target_root)
            .expect("validated dependency root");
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
        let mut packages = BTreeMap::new();
        for package in self.packages.values() {
            let dependencies = package
                .dependencies
                .iter()
                .map(|(alias, root)| {
                    let name = self.packages[root].name.clone();
                    (alias.clone(), name)
                })
                .collect();
            packages.insert(
                package.name.clone(),
                PortablePackage {
                    dependencies,
                    modules: package.modules.clone(),
                    sources: BTreeMap::new(),
                },
            );
        }
        PortablePackageGraph {
            root_package: Some(self.root_package.clone()),
            packages,
        }
    }
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

fn load_package(
    root: &Path,
    packages: &mut BTreeMap<PathBuf, FilesystemPackage>,
    names: &mut BTreeMap<String, PathBuf>,
    stack: &mut Vec<PathBuf>,
    manifest_sources: &mut BTreeMap<PathBuf, String>,
) -> Result<String, PackageSetupError> {
    let root = fs::canonicalize(root).map_err(|error| {
        PackageSetupError::new(format!(
            "cannot resolve package path {}: {error}",
            root.display()
        ))
    })?;
    if let Some(package) = packages.get(&root) {
        return Ok(package.name.clone());
    }
    if stack.contains(&root) {
        let mut cycle = stack
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        cycle.push(root.display().to_string());
        return Err(PackageSetupError::new(format!(
            "package dependency cycle: {}",
            cycle.join(" -> ")
        )));
    }
    stack.push(root.clone());
    let manifest_path = root.join(MANIFEST_NAME);
    let source = fs::read_to_string(&manifest_path).map_err(|error| {
        PackageSetupError::new(format!(
            "cannot read package manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    manifest_sources.insert(manifest_path.clone(), source.clone());
    let manifest = parse_manifest(&manifest_path, &source).map_err(PackageSetupError::new)?;
    if let Some(existing) = names.get(&manifest.name)
        && existing != &root
    {
        return Err(PackageSetupError::new(format!(
            "duplicate package name {:?} at {} and {}",
            manifest.name,
            existing.display(),
            root.display()
        )));
    }
    names.insert(manifest.name.clone(), root.clone());

    let mut modules = BTreeMap::new();
    for (name, relative) in manifest.modules {
        validate_module_name(&name).map_err(PackageSetupError::new)?;
        let relative_path = Path::new(&relative);
        if relative.contains('\0')
            || relative.contains('\\')
            || !safe_relative_path(relative_path, "zt")
        {
            return Err(PackageSetupError::new(format!(
                "unsafe module path {relative:?} in {}",
                manifest_path.display()
            )));
        }
        let canonical = fs::canonicalize(root.join(relative_path)).map_err(|error| {
            PackageSetupError::new(format!(
                "cannot resolve module {name:?} at {}: {error}",
                root.join(relative_path).display()
            ))
        })?;
        if !canonical.starts_with(&root) {
            return Err(PackageSetupError::new(format!(
                "module {name:?} escapes package root {}",
                root.display()
            )));
        }
        if modules
            .insert(name.clone(), path_key(relative_path))
            .is_some()
        {
            return Err(PackageSetupError::new(format!(
                "duplicate module {name:?} in {}",
                manifest_path.display()
            )));
        }
    }

    let mut dependencies = BTreeMap::new();
    for dependency in manifest.dependencies {
        let alias = dependency.alias;
        let relative = dependency.path;
        validate_name("dependency alias", &alias).map_err(PackageSetupError::new)?;
        if alias == "stdlib" {
            return Err(PackageSetupError::new(
                "dependency alias `stdlib` is reserved by the toolchain".to_owned(),
            ));
        }
        if dependencies.contains_key(&alias) {
            return Err(PackageSetupError::at(
                manifest_path.clone(),
                dependency.alias_span,
                format!(
                    "duplicate dependency alias {alias:?} in {}",
                    manifest_path.display()
                ),
            ));
        }
        let relative_path = Path::new(&relative);
        if relative_path.is_absolute()
            || relative_path.as_os_str().is_empty()
            || relative.contains('\0')
            || relative.contains('\\')
            || has_windows_drive_prefix(&relative)
            || relative_path
                .components()
                .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        {
            return Err(PackageSetupError::new(format!(
                "invalid local dependency path {relative:?}"
            )));
        }
        let dependency_root = fs::canonicalize(root.join(relative_path)).map_err(|error| {
            PackageSetupError::new(format!(
                "cannot resolve dependency {alias:?} at {}: {error}",
                root.join(relative_path).display()
            ))
        })?;
        if !dependency_root.join(MANIFEST_NAME).is_file() {
            return Err(PackageSetupError::new(format!(
                "dependency {alias:?} has no {MANIFEST_NAME} at {}",
                dependency_root.display()
            )));
        }
        if let Err(error) = load_package(&dependency_root, packages, names, stack, manifest_sources)
        {
            let label = if error.message.contains("package dependency cycle") {
                "package dependency cycle continues here"
            } else {
                "dependency declared here"
            };
            return Err(error.with_related(crate::SourceLocation {
                path: manifest_path.clone(),
                span: immediate_span(dependency.alias_span),
                label: label.to_owned(),
            }));
        }
        dependencies.insert(alias, dependency_root);
    }
    stack.pop();
    let name = manifest.name.clone();
    packages.insert(
        root.clone(),
        FilesystemPackage {
            name: manifest.name,
            root,
            dependencies,
            modules,
        },
    );
    Ok(name)
}

#[derive(Debug)]
struct Manifest {
    name: String,
    modules: Vec<(String, String)>,
    dependencies: Vec<ManifestDependency>,
}

#[derive(Debug)]
struct ManifestDependency {
    alias: String,
    path: String,
    alias_span: zutai_im::ByteSpan,
}

fn parse_manifest(path: &Path, source: &str) -> Result<Manifest, String> {
    let block = zutai_im::parse_located(source)
        .map_err(|error| format!("invalid package manifest {}: {error}", path.display()))?;
    reject_duplicate_fields(path, &block.value)?;
    let format_version = integer_field(path, &block.value, "formatVersion")?;
    if format_version != FORMAT_VERSION {
        return Err(format!(
            "unsupported package manifest format {format_version} in {}; expected {FORMAT_VERSION}",
            path.display()
        ));
    }
    let name = string_field(path, &block.value, "name")?.to_owned();
    validate_name("package", &name)?;
    let compatibility = string_field(path, &block.value, "compilerCompatibility")?;
    if compatibility != COMPILER_COMPATIBILITY {
        return Err(format!(
            "package compatibility {compatibility:?} in {} does not match compiler {COMPILER_COMPATIBILITY:?}",
            path.display()
        ));
    }
    let modules = entry_list(path, &block.value, "modules", "name", "path")?;
    let dependencies = located_dependencies(path, &block)?;
    let known = [
        "formatVersion",
        "name",
        "compilerCompatibility",
        "modules",
        "dependencies",
    ];
    if let Some(field) = block
        .value
        .iter()
        .find(|pair| !known.contains(&pair.field_name.as_str()))
    {
        return Err(format!(
            "unknown package manifest field {:?} in {}",
            field.field_name,
            path.display()
        ));
    }
    Ok(Manifest {
        name,
        modules,
        dependencies,
    })
}

fn reject_duplicate_fields(path: &Path, block: &Block) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for pair in block.iter() {
        if !seen.insert(&pair.field_name) {
            return Err(format!(
                "duplicate field {:?} in {}",
                pair.field_name,
                path.display()
            ));
        }
    }
    Ok(())
}

fn value_field<'a>(path: &Path, block: &'a Block, name: &str) -> Result<&'a Value, String> {
    block
        .iter()
        .find(|pair| pair.field_name == name)
        .map(|pair| &pair.value)
        .ok_or_else(|| format!("missing field {name:?} in {}", path.display()))
}

fn string_field<'a>(path: &Path, block: &'a Block, name: &str) -> Result<&'a str, String> {
    match value_field(path, block, name)? {
        Value::String(value) => Ok(value),
        _ => Err(format!(
            "field {name:?} must be a string in {}",
            path.display()
        )),
    }
}

fn integer_field(path: &Path, block: &Block, name: &str) -> Result<i64, String> {
    match value_field(path, block, name)? {
        Value::Integer(value) => Ok(*value),
        _ => Err(format!(
            "field {name:?} must be an integer in {}",
            path.display()
        )),
    }
}

fn entry_list(
    path: &Path,
    block: &Block,
    field: &str,
    key_field: &str,
    value_field_name: &str,
) -> Result<Vec<(String, String)>, String> {
    let value = value_field(path, block, field)?;
    let Value::Array(entries) = value else {
        return Err(format!(
            "field {field:?} must be an array in {}",
            path.display()
        ));
    };
    entries
        .iter()
        .map(|entry| {
            let Value::Block(entry) = entry else {
                return Err(format!(
                    "entries in {field:?} must be blocks in {}",
                    path.display()
                ));
            };
            reject_duplicate_fields(path, entry)?;
            if entry.iter().any(|pair| {
                pair.field_name != key_field
                    && pair.field_name != value_field_name
                    && pair.field_name != "visibility"
            }) {
                return Err(format!(
                    "entries in {field:?} contain an unknown field in {}",
                    path.display()
                ));
            }
            Ok((
                string_field(path, entry, key_field)?.to_owned(),
                string_field(path, entry, value_field_name)?.to_owned(),
            ))
        })
        .collect()
}

fn located_dependencies(
    path: &Path,
    block: &zutai_im::LocatedBlock,
) -> Result<Vec<ManifestDependency>, String> {
    let Some(field) = block
        .fields
        .iter()
        .find(|field| field.field_name == "dependencies")
    else {
        return Ok(Vec::new());
    };
    let LocatedChildren::Array(entries) = &field.value.children else {
        return Err(format!(
            "field \"dependencies\" must be an array in {}",
            path.display()
        ));
    };
    entries
        .iter()
        .map(|entry| {
            let LocatedChildren::Block(fields) = &entry.children else {
                return Err(format!(
                    "entries in \"dependencies\" must be blocks in {}",
                    path.display()
                ));
            };
            let Value::Block(value) = &entry.value else {
                unreachable!("located block children match their value")
            };
            reject_duplicate_fields(path, value)?;
            if fields.iter().any(|field| {
                field.field_name != "alias"
                    && field.field_name != "path"
                    && field.field_name != "visibility"
            }) {
                return Err(format!(
                    "entries in \"dependencies\" contain an unknown field in {}",
                    path.display()
                ));
            }
            let alias = fields
                .iter()
                .find(|field| field.field_name == "alias")
                .ok_or_else(|| format!("missing field \"alias\" in {}", path.display()))?;
            let relative = fields
                .iter()
                .find(|field| field.field_name == "path")
                .ok_or_else(|| format!("missing field \"path\" in {}", path.display()))?;
            let Value::String(alias_value) = &alias.value.value else {
                return Err(format!(
                    "field \"alias\" must be a string in {}",
                    path.display()
                ));
            };
            let Value::String(relative_value) = &relative.value.value else {
                return Err(format!(
                    "field \"path\" must be a string in {}",
                    path.display()
                ));
            };
            Ok(ManifestDependency {
                alias: alias_value.clone(),
                path: relative_value.clone(),
                alias_span: alias.value.span,
            })
        })
        .collect()
}

fn immediate_span(span: zutai_im::ByteSpan) -> zutai_syntax::Span {
    zutai_syntax::Span {
        start: span.start as u32,
        end: span.end as u32,
    }
}

fn validate_name(kind: &str, name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid {kind} name {name:?}; use lowercase ASCII letters, digits, and `_`"
        ))
    }
}

fn validate_module_name(name: &str) -> Result<(), String> {
    if !name.is_empty()
        && name
            .split('.')
            .all(|segment| validate_name("module", segment).is_ok())
    {
        Ok(())
    } else {
        Err(format!(
            "invalid module name {name:?}; use dot-separated lowercase ASCII segments"
        ))
    }
}

fn safe_relative_path(path: &Path, extension: &str) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.extension().is_some_and(|found| found == extension)
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validate_portable(graph: &PortablePackageGraph) -> Result<(), String> {
    let root = graph
        .root_package
        .as_deref()
        .ok_or_else(|| "portable package graph has no root package".to_owned())?;
    if !graph.packages.contains_key(root) {
        return Err(format!(
            "portable package graph is missing root package {root:?}"
        ));
    }
    for (name, package) in &graph.packages {
        validate_name("package", name)?;
        for (alias, target) in &package.dependencies {
            validate_name("dependency alias", alias)?;
            if alias == "stdlib" {
                return Err("portable package graph uses reserved alias `stdlib`".to_owned());
            }
            if !graph.packages.contains_key(target) {
                return Err(format!(
                    "package {name:?} dependency {alias:?} names missing package {target:?}"
                ));
            }
        }
        for (module, path) in &package.modules {
            validate_module_name(module)?;
            if path.contains('\0')
                || path.contains('\\')
                || !safe_relative_path(Path::new(path), "zt")
            {
                return Err(format!("package {name:?} has unsafe module path {path:?}"));
            }
        }
        for path in package.sources.keys() {
            let extension = Path::new(path).extension().and_then(|value| value.to_str());
            if path.contains('\0')
                || path.contains('\\')
                || !matches!(extension, Some("zt" | "zti"))
                || Path::new(path).is_absolute()
                || Path::new(path)
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(format!("package {name:?} has unsafe source path {path:?}"));
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for name in graph.packages.keys() {
        validate_portable_acyclic(graph, name, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn validate_portable_acyclic(
    graph: &PortablePackageGraph,
    name: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), String> {
    if visited.contains(name) {
        return Ok(());
    }
    if !visiting.insert(name.to_owned()) {
        return Err(format!(
            "portable package dependency cycle through {name:?}"
        ));
    }
    for target in graph.packages[name].dependencies.values() {
        validate_portable_acyclic(graph, target, visiting, visited)?;
    }
    visiting.remove(name);
    visited.insert(name.to_owned());
    Ok(())
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
        .unwrap();
    let owner_package = &graph.packages[owner];
    let target_name =
        owner_package
            .dependencies
            .get(alias)
            .ok_or_else(|| PackageResolveError::UnknownAlias {
                alias: alias.to_owned(),
            })?;
    let target = &graph.packages[target_name];
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
            path: format!("{target_name}/{path}"),
            message: "source is missing from the portable package graph".to_owned(),
        })?;
    Ok(ResolvedPackageSource {
        key: synthetic_key(target_name, path),
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
    let name = components.next()?.as_os_str().to_str()?;
    graph
        .packages
        .get_key_value(name)
        .map(|(name, _)| name.as_str())
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
            "{{ formatVersion = 1; name = \"{name}\"; compilerCompatibility = \"{COMPILER_COMPATIBILITY}\"; modules = {modules}; dependencies = {dependencies}; }}"
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
    fn rejects_executable_or_wrongly_typed_manifest_fields() {
        let temp = Temp::new();
        let source =
            "{ formatVersion = 1; name = #app; compilerCompatibility = \"0.1.0\"; modules = []; }";
        let error = parse_manifest(&temp.0.join(MANIFEST_NAME), source).unwrap_err();
        assert!(error.contains("must be a string"), "{error}");
    }

    #[test]
    fn split_stdlib_units_are_valid_package_roots() {
        let root = Path::new(env!("ZUTAI_STDLIB_ROOT"));
        for unit in ["base", "data", "system", "web"] {
            let graph = FilesystemPackageGraph::discover(
                &root.join("packages").join(unit).join(MANIFEST_NAME),
            )
            .unwrap();
            assert!(graph.is_some(), "missing package graph for {unit}");
        }
    }

    #[test]
    fn rejects_incomplete_portable_dependency_graph() {
        let graph = PortablePackageGraph {
            root_package: Some("app".to_owned()),
            packages: BTreeMap::from([(
                "app".to_owned(),
                PortablePackage {
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

    #[test]
    fn analyzes_and_replays_transitive_local_packages() {
        let temp = Temp::new();
        let app = temp.0.join("app");
        let facade = temp.0.join("facade");
        let math = temp.0.join("math");
        for root in [&app, &facade, &math] {
            fs::create_dir_all(root.join("src")).unwrap();
        }
        fs::write(
            app.join(MANIFEST_NAME),
            manifest("app", "", "{ alias = \"facade\"; path = \"../facade\"; }"),
        )
        .unwrap();
        fs::write(
            facade.join(MANIFEST_NAME),
            manifest(
                "facade",
                "{ name = \"api\"; path = \"src/api.zt\"; }",
                "{ alias = \"math\"; path = \"../math\"; }",
            ),
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
        let entry = app.join("src/main.zt");
        fs::write(&entry, "api ::= import facade.api; api.answer").unwrap();
        fs::write(
            facade.join("src/api.zt"),
            "internal ::= import \"internal.zt\"; vector ::= import math.vector; { answer = internal.pick vector.answer; }",
        )
        .unwrap();
        fs::write(facade.join("src/internal.zt"), "{ pick = \\value. value; }").unwrap();
        fs::write(math.join("src/vector.zt"), "{ answer = 42; }").unwrap();

        let stdlib = crate::StdlibSources::load(env!("ZUTAI_STDLIB_ROOT")).unwrap();
        let recorded = crate::analyze_path_recording_with_stdlib(&entry, &stdlib).unwrap();
        assert!(recorded.analysis.blocking_diagnostics().next().is_none());
        assert_eq!(recorded.packages.packages["facade"].sources.len(), 2);
        assert_eq!(recorded.packages.packages["math"].sources.len(), 1);

        let replayed = crate::analyze_sources_with_stdlib_and_packages(
            &recorded.entry,
            &recorded.sources,
            crate::AnalysisOptions::default(),
            &stdlib,
            recorded.packages,
        )
        .unwrap();
        assert!(replayed.blocking_diagnostics().next().is_none());
        assert!(replayed.is_thir_complete());
    }

    #[test]
    fn cache_invalidates_when_package_manifest_changes() {
        let temp = Temp::new();
        let app = temp.0.join("app");
        let dep = temp.0.join("dep");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::create_dir_all(dep.join("src")).unwrap();
        fs::write(
            app.join(MANIFEST_NAME),
            manifest("app", "", "{ alias = \"dep\"; path = \"../dep\"; }"),
        )
        .unwrap();
        fs::write(
            dep.join(MANIFEST_NAME),
            manifest("dep", "{ name = \"value\"; path = \"src/value.zt\"; }", ""),
        )
        .unwrap();
        let entry = app.join("src/main.zt");
        fs::write(&entry, "v ::= import dep.value; v.answer").unwrap();
        fs::write(dep.join("src/value.zt"), "{ answer = 42; }").unwrap();

        let cache = crate::AnalysisCache::default();
        let first = crate::analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
        let before = cache.stats();
        assert_eq!(before.module_misses, 1);

        fs::write(dep.join("src/unused.zt"), "0").unwrap();
        fs::write(
            dep.join(MANIFEST_NAME),
            manifest(
                "dep",
                "{ name = \"value\"; path = \"src/value.zt\"; }; { name = \"unused\"; path = \"src/unused.zt\"; }",
                "",
            ),
        )
        .unwrap();
        let second = crate::analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
        assert_eq!(cache.stats().module_misses, before.module_misses + 1);
    }

    #[test]
    fn reports_dependency_cycles_as_package_setup_errors() {
        let temp = Temp::new();
        let a = temp.0.join("a");
        let b = temp.0.join("b");
        fs::create_dir_all(a.join("src")).unwrap();
        fs::create_dir_all(b.join("src")).unwrap();
        fs::write(
            a.join(MANIFEST_NAME),
            manifest("a", "", "{ alias = \"b\"; path = \"../b\"; }"),
        )
        .unwrap();
        fs::write(
            b.join(MANIFEST_NAME),
            manifest("b", "", "{ alias = \"a\"; path = \"../a\"; }"),
        )
        .unwrap();
        let entry = a.join("src/main.zt");
        fs::write(&entry, "x ::= import b.missing; x").unwrap();
        let stdlib = crate::StdlibSources::load(env!("ZUTAI_STDLIB_ROOT")).unwrap();
        let analysis = crate::analyze_path_with_stdlib(&entry, &stdlib).unwrap();
        let diagnostic = analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                crate::SemanticDiagnosticKind::Import(import)
                    if matches!(
                        &import.kind,
                        crate::ImportDiagnosticKind::PackageSetup { message }
                            if message.contains("package dependency cycle")
                    ) =>
                {
                    Some(import)
                }
                _ => None,
            })
            .expect("package dependency cycle diagnostic");
        assert_eq!(diagnostic.related.len(), 2);
        assert_eq!(diagnostic.related[0].path, a.join(MANIFEST_NAME));
        assert_eq!(diagnostic.related[1].path, b.join(MANIFEST_NAME));
        assert_eq!(
            diagnostic.related[0].label,
            "package dependency cycle continues here"
        );
        assert_eq!(
            diagnostic.related[1].label,
            "package dependency cycle continues here"
        );
    }

    #[test]
    fn duplicate_dependency_alias_points_at_manifest_entry() {
        let temp = Temp::new();
        let app = temp.0.join("app");
        let dep = temp.0.join("dep");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::create_dir_all(dep.join("src")).unwrap();
        fs::write(dep.join(MANIFEST_NAME), manifest("dep", "", "")).unwrap();
        let manifest_source = manifest(
            "app",
            "",
            "{ alias = \"dep\"; path = \"../dep\"; }; { alias = \"dep\"; path = \"../dep\"; }",
        );
        fs::write(app.join(MANIFEST_NAME), &manifest_source).unwrap();
        let entry = app.join("src/main.zt");
        fs::write(&entry, "x ::= import dep.api; x").unwrap();
        let stdlib = crate::StdlibSources::load(env!("ZUTAI_STDLIB_ROOT")).unwrap();
        let analysis = crate::analyze_path_with_stdlib(&entry, &stdlib).unwrap();
        let diagnostic = analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                crate::SemanticDiagnosticKind::Import(import)
                    if matches!(
                        &import.kind,
                        crate::ImportDiagnosticKind::PackageSetup { message }
                            if message.contains("duplicate dependency alias")
                    ) =>
                {
                    Some(import)
                }
                _ => None,
            })
            .expect("duplicate dependency alias diagnostic");
        assert_eq!(
            diagnostic.path.as_deref(),
            Some(app.join(MANIFEST_NAME).as_path())
        );
        assert_eq!(
            &manifest_source[diagnostic.span.start as usize..diagnostic.span.end as usize],
            "\"dep\""
        );
    }

    #[test]
    fn package_setup_error_points_at_first_import() {
        let temp = Temp::new();
        let app = temp.0.join("app");
        fs::create_dir_all(app.join("src")).unwrap();
        fs::write(
            app.join(MANIFEST_NAME),
            manifest("app", "", "{ alias = \"dep\"; path = \"../missing\"; }"),
        )
        .unwrap();
        let source = "x ::= import dep.api;\nx\n";
        let entry = app.join("src/main.zt");
        fs::write(&entry, source).unwrap();
        let stdlib = crate::StdlibSources::load(env!("ZUTAI_STDLIB_ROOT")).unwrap();
        let analysis = crate::analyze_path_with_stdlib(&entry, &stdlib).unwrap();
        let diagnostic = analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                crate::SemanticDiagnosticKind::Import(import)
                    if matches!(
                        import.kind,
                        crate::ImportDiagnosticKind::PackageSetup { .. }
                    ) =>
                {
                    Some(import)
                }
                _ => None,
            })
            .expect("invalid dependency should produce a package setup diagnostic");
        assert_eq!(
            &source[diagnostic.span.start as usize..diagnostic.span.end as usize],
            "import dep.api"
        );
    }
}
