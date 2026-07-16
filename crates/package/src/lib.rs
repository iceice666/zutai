//! Portable Zutai package manifests and locked package graphs.
//!
//! Parsing and validation compile on every target. Network acquisition and
//! content-addressed cache mutation live behind the native `acquire` module.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use url::Url;
use zutai_im::{Block, LocatedChildren, Value};

pub const MANIFEST_NAME: &str = "zutai.zti";
pub const LOCK_NAME: &str = "zutai.lock.zti";
pub const LOCK_FORMAT_VERSION: i64 = 1;
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub format_version: i64,
    pub name: String,
    pub compiler_compatibility: String,
    pub modules: BTreeMap<String, String>,
    pub dependencies: Vec<ManifestDependency>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestDependency {
    pub alias: String,
    pub source: ManifestSource,
    pub alias_span: zutai_im::ByteSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManifestSource {
    Path {
        path: String,
    },
    Git {
        url: String,
        rev: String,
        subdir: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePackageGraph {
    pub root_package: Option<String>,
    pub packages: BTreeMap<String, PortablePackage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePackage {
    pub name: String,
    pub dependencies: BTreeMap<String, String>,
    pub modules: BTreeMap<String, String>,
    pub sources: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lockfile {
    pub generated_by: String,
    pub root: String,
    pub packages: BTreeMap<String, LockedPackage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockedPackage {
    pub id: String,
    pub name: String,
    pub source: LockedSource,
    pub manifest_hash: String,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockedSource {
    Path {
        path: String,
    },
    Git {
        url: String,
        resolved_url: String,
        requested_rev: String,
        commit: String,
        object_format: ObjectFormat,
        subdir: String,
        tree_hash: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}

impl ObjectFormat {
    pub fn atom(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }

    pub fn hex_len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageError {
    pub message: String,
    pub path: Option<PathBuf>,
    pub span: Option<zutai_im::ByteSpan>,
}

impl PackageError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: None,
            span: None,
        }
    }

    pub fn at(
        path: impl Into<PathBuf>,
        span: zutai_im::ByteSpan,
        message: impl Into<String>,
    ) -> Self {
        Self {
            message: message.into(),
            path: Some(path.into()),
            span: Some(span),
        }
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PackageError {}

pub fn parse_manifest(path: &Path, source: &str) -> Result<Manifest, PackageError> {
    let block = zutai_im::parse_located(source).map_err(|error| {
        PackageError::new(format!(
            "invalid package manifest {}: {error}",
            path.display()
        ))
    })?;
    reject_duplicate_fields(path, &block.value)?;
    let format_version = integer_field(path, &block.value, "formatVersion")?;
    if !matches!(format_version, 1 | 2) {
        return Err(PackageError::new(format!(
            "unsupported package manifest format {format_version} in {}; expected 1 or 2",
            path.display()
        )));
    }
    let name = string_field(path, &block.value, "name")?.to_owned();
    validate_name("package", &name).map_err(PackageError::new)?;
    let compiler_compatibility =
        string_field(path, &block.value, "compilerCompatibility")?.to_owned();
    validate_compiler_compatibility(path, format_version, &compiler_compatibility)?;
    let mut modules = BTreeMap::new();
    for (module, module_path) in entry_list(path, &block.value, "modules", "name", "path")? {
        validate_module_name(&module).map_err(PackageError::new)?;
        if !safe_relative_path(Path::new(&module_path), "zt")
            || module_path.contains('\\')
            || module_path.contains('\0')
        {
            return Err(PackageError::new(format!(
                "package module {module:?} has unsafe path {module_path:?} in {}",
                path.display()
            )));
        }
        if modules.insert(module.clone(), module_path).is_some() {
            return Err(PackageError::new(format!(
                "duplicate package module {module:?} in {}",
                path.display()
            )));
        }
    }
    let dependencies = parse_dependencies(path, &block, format_version)?;
    let mut aliases = BTreeSet::new();
    for dependency in &dependencies {
        validate_name("dependency alias", &dependency.alias).map_err(PackageError::new)?;
        if dependency.alias == "stdlib" {
            return Err(PackageError::at(
                path,
                dependency.alias_span,
                "dependency alias `stdlib` is reserved by the toolchain",
            ));
        }
        if !aliases.insert(&dependency.alias) {
            return Err(PackageError::at(
                path,
                dependency.alias_span,
                format!("duplicate dependency alias {:?}", dependency.alias),
            ));
        }
    }
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
        return Err(PackageError::new(format!(
            "unknown package manifest field {:?} in {}",
            field.field_name,
            path.display()
        )));
    }
    Ok(Manifest {
        format_version,
        name,
        compiler_compatibility,
        modules,
        dependencies,
    })
}

fn validate_compiler_compatibility(
    path: &Path,
    format_version: i64,
    compatibility: &str,
) -> Result<(), PackageError> {
    if format_version == 1 {
        if compatibility == COMPILER_VERSION {
            return Ok(());
        }
        return Err(PackageError::new(format!(
            "package compatibility {compatibility:?} in {} does not match compiler {COMPILER_VERSION:?}",
            path.display()
        )));
    }
    let requirement = semver::VersionReq::parse(compatibility).map_err(|error| {
        PackageError::new(format!(
            "invalid package compatibility requirement {compatibility:?} in {}: {error}",
            path.display()
        ))
    })?;
    let version = semver::Version::parse(COMPILER_VERSION).expect("workspace version is SemVer");
    if requirement.matches(&version) {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "package compatibility {compatibility:?} in {} does not accept compiler {COMPILER_VERSION:?}",
            path.display()
        )))
    }
}

fn parse_dependencies(
    path: &Path,
    block: &zutai_im::LocatedBlock,
    format_version: i64,
) -> Result<Vec<ManifestDependency>, PackageError> {
    let Some(field) = block
        .fields
        .iter()
        .find(|field| field.field_name == "dependencies")
    else {
        return Ok(Vec::new());
    };
    let LocatedChildren::Array(entries) = &field.value.children else {
        return Err(PackageError::new(format!(
            "field \"dependencies\" must be an array in {}",
            path.display()
        )));
    };
    entries
        .iter()
        .map(|entry| {
            let LocatedChildren::Block(fields) = &entry.children else {
                return Err(PackageError::new(format!(
                    "entries in \"dependencies\" must be blocks in {}",
                    path.display()
                )));
            };
            let Value::Block(value) = &entry.value else {
                unreachable!("located block children match their value")
            };
            reject_duplicate_fields(path, value)?;
            let alias = located_field(path, fields, "alias")?;
            let Value::String(alias_value) = &alias.value.value else {
                return Err(PackageError::new(format!(
                    "field \"alias\" must be a string in {}",
                    path.display()
                )));
            };
            let source = if format_version == 1 {
                if fields.iter().any(|field| {
                    field.field_name != "alias"
                        && field.field_name != "path"
                        && field.field_name != "visibility"
                }) {
                    return Err(PackageError::new(format!(
                        "entries in \"dependencies\" contain an unknown field in {}",
                        path.display()
                    )));
                }
                let path_field = located_field(path, fields, "path")?;
                let Value::String(relative) = &path_field.value.value else {
                    return Err(PackageError::new(format!(
                        "field \"path\" must be a string in {}",
                        path.display()
                    )));
                };
                ManifestSource::Path {
                    path: relative.clone(),
                }
            } else {
                if fields.iter().any(|field| {
                    field.field_name != "alias"
                        && field.field_name != "source"
                        && field.field_name != "visibility"
                }) {
                    return Err(PackageError::new(format!(
                        "entries in \"dependencies\" contain an unknown field in {}",
                        path.display()
                    )));
                }
                let source = located_field(path, fields, "source")?;
                parse_manifest_source(path, &source.value.value)?
            };
            Ok(ManifestDependency {
                alias: alias_value.clone(),
                source,
                alias_span: alias.value.span,
            })
        })
        .collect()
}

fn parse_manifest_source(path: &Path, value: &Value) -> Result<ManifestSource, PackageError> {
    let Value::Block(source) = value else {
        return Err(PackageError::new(format!(
            "field \"source\" must be a block in {}",
            path.display()
        )));
    };
    reject_duplicate_fields(path, source)?;
    let kind = atom_field(path, source, "kind")?;
    match kind {
        "path" => {
            reject_unknown_fields(path, source, &["kind", "path"])?;
            Ok(ManifestSource::Path {
                path: string_field(path, source, "path")?.to_owned(),
            })
        }
        "git" => {
            reject_unknown_fields(path, source, &["kind", "url", "rev", "subdir"])?;
            let url = normalize_git_url(string_field(path, source, "url")?)?;
            let rev = string_field(path, source, "rev")?.to_owned();
            validate_requested_rev(&rev)?;
            let subdir = optional_string_field(path, source, "subdir")?.unwrap_or(".");
            let subdir = normalize_subdir(subdir)?;
            Ok(ManifestSource::Git { url, rev, subdir })
        }
        other => Err(PackageError::new(format!(
            "unknown package source kind #{other} in {}",
            path.display()
        ))),
    }
}

pub fn normalize_git_url(raw: &str) -> Result<String, PackageError> {
    let mut url = Url::parse(raw)
        .map_err(|error| PackageError::new(format!("invalid Git URL {raw:?}: {error}")))?;
    if url.scheme() != "https" {
        return Err(PackageError::new(
            "Git package URLs must use https and may not use SSH, file, or scp syntax",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PackageError::new(
            "Git package URLs may not contain credentials",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(PackageError::new(
            "Git package URLs may not contain a query string or fragment",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| PackageError::new("Git package URL has no DNS host"))?
        .to_ascii_lowercase();
    url.set_host(Some(&host))
        .map_err(|_| PackageError::new("Git package URL has an invalid host"))?;
    if url.port() == Some(443) {
        url.set_port(None)
            .map_err(|_| PackageError::new("Git package URL has an invalid port"))?;
    }
    let path = normalize_url_path(url.path())?;
    url.set_path(&path);
    Ok(url.into())
}

fn normalize_url_path(path: &str) -> Result<String, PackageError> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(PackageError::new("Git URL path escapes its root"));
                }
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(PackageError::new("Git package URL has no repository path"));
    }
    Ok(format!("/{}", parts.join("/")))
}

pub fn validate_requested_rev(rev: &str) -> Result<(), PackageError> {
    let full_hex = matches!(rev.len(), 40 | 64)
        && rev.bytes().all(|byte| byte.is_ascii_hexdigit())
        && rev.bytes().all(|byte| !byte.is_ascii_uppercase());
    let reference = rev
        .strip_prefix("refs/heads/")
        .or_else(|| rev.strip_prefix("refs/tags/"));
    let fully_qualified = reference.is_some_and(valid_reference_tail);
    if fully_qualified || full_hex {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "invalid Git revision {rev:?}; use a full lowercase object ID or refs/heads/... / refs/tags/..."
        )))
    }
}

fn valid_reference_tail(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.ends_with(".lock")
        && !value.contains("..")
        && !value.contains("@{")
        && !value.contains("//")
        && !value.bytes().any(|byte| {
            byte <= b' '
                || byte == 0x7f
                || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        })
        && value.split('/').all(|part| !part.is_empty() && part != ".")
}

pub fn normalize_subdir(raw: &str) -> Result<String, PackageError> {
    normalize_relative(raw, true).ok_or_else(|| {
        PackageError::new(format!(
            "invalid Git package subdir {raw:?}; use a relative portable path"
        ))
    })
}

pub fn normalize_path_dependency(raw: &str) -> Result<String, PackageError> {
    normalize_relative(raw, false).ok_or_else(|| {
        PackageError::new(format!(
            "invalid local dependency path {raw:?}; use a relative portable path"
        ))
    })
}

fn normalize_relative(raw: &str, confine: bool) -> Option<String> {
    if raw.is_empty()
        || raw.contains('\0')
        || raw.contains('\\')
        || Path::new(raw).is_absolute()
        || has_windows_drive_prefix(raw)
    {
        return None;
    }
    let mut out = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if confine && out.pop().is_none() {
                    return None;
                }
                if !confine {
                    out.push("..");
                }
            }
            ".git" => return None,
            part => out.push(part),
        }
    }
    if out.is_empty() {
        Some(".".to_owned())
    } else {
        Some(out.join("/"))
    }
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn package_id(source: &LockedSource) -> String {
    let mut digest = Sha256::new();
    digest.update(b"zutai-package-id-v1");
    match source {
        LockedSource::Path { path } => {
            update_part(&mut digest, b"path");
            update_part(&mut digest, path.as_bytes());
        }
        LockedSource::Git {
            url,
            commit,
            object_format,
            subdir,
            ..
        } => {
            update_part(&mut digest, b"git");
            update_part(&mut digest, url.as_bytes());
            update_part(&mut digest, object_format.atom().as_bytes());
            update_part(&mut digest, commit.as_bytes());
            update_part(&mut digest, subdir.as_bytes());
        }
    }
    format!("pkg:{}", base32_lower(&digest.finalize()))
}

fn update_part(digest: &mut Sha256, part: &[u8]) {
    digest.update((part.len() as u64).to_le_bytes());
    digest.update(part);
}

fn base32_lower(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut acc = 0_u32;
    let mut bits = 0_u8;
    for byte in bytes {
        acc = (acc << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        output.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    output
}

pub fn parse_lockfile(path: &Path, source: &str) -> Result<Lockfile, PackageError> {
    let block = zutai_im::parse(source).map_err(|error| {
        PackageError::new(format!("invalid package lock {}: {error}", path.display()))
    })?;
    reject_duplicate_fields(path, &block)?;
    let version = integer_field(path, &block, "formatVersion")?;
    if version != LOCK_FORMAT_VERSION {
        return Err(PackageError::new(format!(
            "unsupported package lock format {version} in {}; expected {LOCK_FORMAT_VERSION}",
            path.display()
        )));
    }
    reject_unknown_fields(
        path,
        &block,
        &["formatVersion", "generatedBy", "root", "packages"],
    )?;
    let generated_by = string_field(path, &block, "generatedBy")?.to_owned();
    let root = string_field(path, &block, "root")?.to_owned();
    validate_package_id(&root)?;
    let Value::Array(entries) = value_field(path, &block, "packages")? else {
        return Err(PackageError::new(format!(
            "field \"packages\" must be an array in {}",
            path.display()
        )));
    };
    let mut packages = BTreeMap::new();
    for entry in entries {
        let Value::Block(entry) = entry else {
            return Err(PackageError::new(format!(
                "entries in \"packages\" must be blocks in {}",
                path.display()
            )));
        };
        reject_duplicate_fields(path, entry)?;
        reject_unknown_fields(
            path,
            entry,
            &["id", "name", "source", "manifestHash", "dependencies"],
        )?;
        let id = string_field(path, entry, "id")?.to_owned();
        validate_package_id(&id)?;
        let name = string_field(path, entry, "name")?.to_owned();
        validate_name("package", &name).map_err(PackageError::new)?;
        let source = parse_locked_source(path, value_field(path, entry, "source")?)?;
        if package_id(&source) != id {
            return Err(PackageError::new(format!(
                "package lock node {id:?} does not match its source identity"
            )));
        }
        let manifest_hash = string_field(path, entry, "manifestHash")?.to_owned();
        validate_digest(&manifest_hash)?;
        let dependencies = dependency_edges(path, value_field(path, entry, "dependencies")?)?;
        if packages
            .insert(
                id.clone(),
                LockedPackage {
                    id: id.clone(),
                    name,
                    source,
                    manifest_hash,
                    dependencies,
                },
            )
            .is_some()
        {
            return Err(PackageError::new(format!(
                "duplicate package lock node {id:?}"
            )));
        }
    }
    let lock = Lockfile {
        generated_by,
        root,
        packages,
    };
    validate_lockfile(&lock)?;
    Ok(lock)
}

fn parse_locked_source(path: &Path, value: &Value) -> Result<LockedSource, PackageError> {
    let Value::Block(source) = value else {
        return Err(PackageError::new(format!(
            "lock source must be a block in {}",
            path.display()
        )));
    };
    reject_duplicate_fields(path, source)?;
    match atom_field(path, source, "kind")? {
        "path" => {
            reject_unknown_fields(path, source, &["kind", "path"])?;
            Ok(LockedSource::Path {
                path: normalize_path_dependency(string_field(path, source, "path")?)?,
            })
        }
        "git" => {
            reject_unknown_fields(
                path,
                source,
                &[
                    "kind",
                    "url",
                    "resolvedUrl",
                    "requestedRev",
                    "commit",
                    "objectFormat",
                    "subdir",
                    "treeHash",
                ],
            )?;
            let url = normalize_git_url(string_field(path, source, "url")?)?;
            let resolved_url = normalize_git_url(string_field(path, source, "resolvedUrl")?)?;
            let requested_rev = string_field(path, source, "requestedRev")?.to_owned();
            validate_requested_rev(&requested_rev)?;
            let object_format = match atom_field(path, source, "objectFormat")? {
                "sha1" => ObjectFormat::Sha1,
                "sha256" => ObjectFormat::Sha256,
                other => {
                    return Err(PackageError::new(format!(
                        "unsupported Git object format #{other}"
                    )));
                }
            };
            let commit = string_field(path, source, "commit")?.to_ascii_lowercase();
            validate_object_id(&commit, object_format)?;
            let subdir = normalize_subdir(string_field(path, source, "subdir")?)?;
            let tree_hash = string_field(path, source, "treeHash")?.to_owned();
            validate_digest(&tree_hash)?;
            Ok(LockedSource::Git {
                url,
                resolved_url,
                requested_rev,
                commit,
                object_format,
                subdir,
                tree_hash,
            })
        }
        other => Err(PackageError::new(format!(
            "unknown package lock source kind #{other}"
        ))),
    }
}

fn dependency_edges(path: &Path, value: &Value) -> Result<BTreeMap<String, String>, PackageError> {
    let Value::Array(entries) = value else {
        return Err(PackageError::new(format!(
            "lock dependencies must be an array in {}",
            path.display()
        )));
    };
    let mut edges = BTreeMap::new();
    for entry in entries {
        let Value::Block(entry) = entry else {
            return Err(PackageError::new("lock dependency entries must be blocks"));
        };
        reject_duplicate_fields(path, entry)?;
        reject_unknown_fields(path, entry, &["alias", "package"])?;
        let alias = string_field(path, entry, "alias")?.to_owned();
        validate_name("dependency alias", &alias).map_err(PackageError::new)?;
        if alias == "stdlib" {
            return Err(PackageError::new(
                "dependency alias `stdlib` is reserved by the toolchain",
            ));
        }
        let package = string_field(path, entry, "package")?.to_owned();
        validate_package_id(&package)?;
        if edges.insert(alias.clone(), package).is_some() {
            return Err(PackageError::new(format!(
                "duplicate package lock dependency alias {alias:?}"
            )));
        }
    }
    Ok(edges)
}

pub fn validate_lockfile(lock: &Lockfile) -> Result<(), PackageError> {
    if !lock.packages.contains_key(&lock.root) {
        return Err(PackageError::new(format!(
            "package lock is missing root node {:?}",
            lock.root
        )));
    }
    for package in lock.packages.values() {
        for (alias, target) in &package.dependencies {
            if !lock.packages.contains_key(target) {
                return Err(PackageError::new(format!(
                    "package lock node {:?} dependency {alias:?} names missing node {target:?}",
                    package.id
                )));
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    validate_lock_acyclic(lock, &lock.root, &mut visiting, &mut visited)
}

fn validate_lock_acyclic(
    lock: &Lockfile,
    id: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), PackageError> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(PackageError::new(format!(
            "package lock dependency cycle through {id:?}"
        )));
    }
    for target in lock.packages[id].dependencies.values() {
        validate_lock_acyclic(lock, target, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    Ok(())
}

pub fn render_lockfile(lock: &Lockfile) -> String {
    let mut out = String::new();
    out.push_str("{\n  formatVersion = 1;\n  generatedBy = ");
    push_string(&mut out, &lock.generated_by);
    out.push_str(";\n  root = ");
    push_string(&mut out, &lock.root);
    out.push_str(";\n  packages = [\n");
    for package in lock.packages.values() {
        out.push_str("    {\n      id = ");
        push_string(&mut out, &package.id);
        out.push_str(";\n      name = ");
        push_string(&mut out, &package.name);
        out.push_str(";\n      source = ");
        render_locked_source(&mut out, &package.source);
        out.push_str(";\n      manifestHash = ");
        push_string(&mut out, &package.manifest_hash);
        out.push_str(";\n      dependencies = [\n");
        for (alias, target) in &package.dependencies {
            out.push_str("        { alias = ");
            push_string(&mut out, alias);
            out.push_str("; package = ");
            push_string(&mut out, target);
            out.push_str("; };\n");
        }
        out.push_str("      ];\n    };\n");
    }
    out.push_str("  ];\n}\n");
    out
}

fn render_locked_source(out: &mut String, source: &LockedSource) {
    match source {
        LockedSource::Path { path } => {
            out.push_str("{ kind = #path; path = ");
            push_string(out, path);
            out.push_str("; }");
        }
        LockedSource::Git {
            url,
            resolved_url,
            requested_rev,
            commit,
            object_format,
            subdir,
            tree_hash,
        } => {
            out.push_str("{ kind = #git; url = ");
            push_string(out, url);
            out.push_str("; resolvedUrl = ");
            push_string(out, resolved_url);
            out.push_str("; requestedRev = ");
            push_string(out, requested_rev);
            out.push_str("; commit = ");
            push_string(out, commit);
            out.push_str("; objectFormat = #");
            out.push_str(object_format.atom());
            out.push_str("; subdir = ");
            push_string(out, subdir);
            out.push_str("; treeHash = ");
            push_string(out, tree_hash);
            out.push_str("; }");
        }
    }
}

fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn validate_object_id(value: &str, format: ObjectFormat) -> Result<(), PackageError> {
    if value.len() == format.hex_len()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value.bytes().all(|byte| !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "invalid {} Git object ID {value:?}",
            format.atom()
        )))
    }
}

fn validate_digest(value: &str) -> Result<(), PackageError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(PackageError::new(format!(
            "invalid digest {value:?}; expected sha256:<64 lowercase hex digits>"
        )));
    };
    if hex.len() == 64
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        && hex.bytes().all(|byte| !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "invalid digest {value:?}; expected sha256:<64 lowercase hex digits>"
        )))
    }
}

fn validate_package_id(value: &str) -> Result<(), PackageError> {
    let Some(body) = value.strip_prefix("pkg:") else {
        return Err(PackageError::new(format!(
            "invalid package node ID {value:?}"
        )));
    };
    if body.len() == 52
        && body
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'2'..=b'7'))
    {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "invalid package node ID {value:?}"
        )))
    }
}

pub fn validate_portable(graph: &PortablePackageGraph) -> Result<(), PackageError> {
    let root = graph
        .root_package
        .as_deref()
        .ok_or_else(|| PackageError::new("portable package graph has no root package"))?;
    if !graph.packages.contains_key(root) {
        return Err(PackageError::new(format!(
            "portable package graph is missing root package {root:?}"
        )));
    }
    for (id, package) in &graph.packages {
        validate_package_id(id)?;
        validate_name("package", &package.name).map_err(PackageError::new)?;
        for (alias, target) in &package.dependencies {
            validate_name("dependency alias", alias).map_err(PackageError::new)?;
            if alias == "stdlib" {
                return Err(PackageError::new(
                    "portable package graph uses reserved alias `stdlib`",
                ));
            }
            if !graph.packages.contains_key(target) {
                return Err(PackageError::new(format!(
                    "package {id:?} dependency {alias:?} names missing package {target:?}"
                )));
            }
        }
        for (module, path) in &package.modules {
            validate_module_name(module).map_err(PackageError::new)?;
            if path.contains('\0')
                || path.contains('\\')
                || !safe_relative_path(Path::new(path), "zt")
            {
                return Err(PackageError::new(format!(
                    "package {id:?} has unsafe module path {path:?}"
                )));
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
                return Err(PackageError::new(format!(
                    "package {id:?} has unsafe source path {path:?}"
                )));
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    validate_portable_acyclic(graph, root, &mut visiting, &mut visited)
}

fn validate_portable_acyclic(
    graph: &PortablePackageGraph,
    id: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), PackageError> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(PackageError::new(format!(
            "portable package dependency cycle through {id:?}"
        )));
    }
    for target in graph.packages[id].dependencies.values() {
        validate_portable_acyclic(graph, target, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    Ok(())
}

fn value_field<'a>(path: &Path, block: &'a Block, name: &str) -> Result<&'a Value, PackageError> {
    block
        .iter()
        .find(|pair| pair.field_name == name)
        .map(|pair| &pair.value)
        .ok_or_else(|| PackageError::new(format!("missing field {name:?} in {}", path.display())))
}

fn string_field<'a>(path: &Path, block: &'a Block, name: &str) -> Result<&'a str, PackageError> {
    match value_field(path, block, name)? {
        Value::String(value) => Ok(value),
        _ => Err(PackageError::new(format!(
            "field {name:?} must be a string in {}",
            path.display()
        ))),
    }
}

fn optional_string_field<'a>(
    path: &Path,
    block: &'a Block,
    name: &str,
) -> Result<Option<&'a str>, PackageError> {
    match block.iter().find(|pair| pair.field_name == name) {
        None => Ok(None),
        Some(pair) => match &pair.value {
            Value::String(value) => Ok(Some(value)),
            _ => Err(PackageError::new(format!(
                "field {name:?} must be a string in {}",
                path.display()
            ))),
        },
    }
}

fn integer_field(path: &Path, block: &Block, name: &str) -> Result<i64, PackageError> {
    match value_field(path, block, name)? {
        Value::Integer(value) => Ok(*value),
        _ => Err(PackageError::new(format!(
            "field {name:?} must be an integer in {}",
            path.display()
        ))),
    }
}

fn atom_field<'a>(path: &Path, block: &'a Block, name: &str) -> Result<&'a str, PackageError> {
    match value_field(path, block, name)? {
        Value::Atom(value) => Ok(value),
        _ => Err(PackageError::new(format!(
            "field {name:?} must be an atom in {}",
            path.display()
        ))),
    }
}

fn entry_list(
    path: &Path,
    block: &Block,
    field: &str,
    key_field: &str,
    value_field_name: &str,
) -> Result<Vec<(String, String)>, PackageError> {
    let value = value_field(path, block, field)?;
    let Value::Array(entries) = value else {
        return Err(PackageError::new(format!(
            "field {field:?} must be an array in {}",
            path.display()
        )));
    };
    entries
        .iter()
        .map(|entry| {
            let Value::Block(entry) = entry else {
                return Err(PackageError::new(format!(
                    "entries in {field:?} must be blocks in {}",
                    path.display()
                )));
            };
            reject_duplicate_fields(path, entry)?;
            reject_unknown_fields(path, entry, &[key_field, value_field_name, "visibility"])?;
            Ok((
                string_field(path, entry, key_field)?.to_owned(),
                string_field(path, entry, value_field_name)?.to_owned(),
            ))
        })
        .collect()
}

fn located_field<'a>(
    path: &Path,
    fields: &'a [zutai_im::LocatedPair],
    name: &str,
) -> Result<&'a zutai_im::LocatedPair, PackageError> {
    fields
        .iter()
        .find(|field| field.field_name == name)
        .ok_or_else(|| PackageError::new(format!("missing field {name:?} in {}", path.display())))
}

fn reject_duplicate_fields(path: &Path, block: &Block) -> Result<(), PackageError> {
    let mut seen = BTreeSet::new();
    for pair in block.iter() {
        if !seen.insert(&pair.field_name) {
            return Err(PackageError::new(format!(
                "duplicate field {:?} in {}",
                pair.field_name,
                path.display()
            )));
        }
    }
    Ok(())
}

fn reject_unknown_fields(path: &Path, block: &Block, known: &[&str]) -> Result<(), PackageError> {
    if let Some(field) = block
        .iter()
        .find(|pair| !known.contains(&pair.field_name.as_str()))
    {
        Err(PackageError::new(format!(
            "unknown field {:?} in {}",
            field.field_name,
            path.display()
        )))
    } else {
        Ok(())
    }
}

pub fn validate_name(kind: &str, name: &str) -> Result<(), String> {
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

pub fn validate_module_name(name: &str) -> Result<(), String> {
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

pub fn safe_relative_path(path: &Path, extension: &str) -> bool {
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

#[cfg(not(target_arch = "wasm32"))]
pub mod acquire;

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    #[test]
    fn manifest_v2_parses_explicit_sources() {
        let source = r#"{
          formatVersion = 2;
          name = "app";
          compilerCompatibility = ">=0.1.0, <0.2.0";
          modules = [];
          dependencies = [
            { alias = "local"; source = { kind = #path; path = "../local"; }; };
            { alias = "remote"; source = { kind = #git; url = "HTTPS://Example.COM:443/a/./b.git"; rev = "refs/tags/v1"; }; };
          ];
        }"#;
        let manifest = parse_manifest(Path::new("zutai.zti"), source).unwrap();
        assert_eq!(manifest.format_version, 2);
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(
            manifest.dependencies[1].source,
            ManifestSource::Git {
                url: "https://example.com/a/b.git".to_owned(),
                rev: "refs/tags/v1".to_owned(),
                subdir: ".".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_v2_rejects_implicit_or_unsafe_git_identity() {
        for rev in ["main", "HEAD", "deadbeef"] {
            let source = format!(
                "{{ formatVersion = 2; name = \"app\"; compilerCompatibility = \"*\"; modules = []; dependencies = [{{ alias = \"dep\"; source = {{ kind = #git; url = \"https://example.com/dep.git\"; rev = \"{rev}\"; }}; }};]; }}"
            );
            assert!(parse_manifest(Path::new("zutai.zti"), &source).is_err());
        }
        assert!(normalize_git_url("https://user@example.com/dep.git").is_err());
        assert!(normalize_git_url("http://example.com/dep.git").is_err());
    }

    #[test]
    fn lock_round_trip_is_canonical() {
        let root_source = LockedSource::Path {
            path: ".".to_owned(),
        };
        let root = package_id(&root_source);
        let git_source = LockedSource::Git {
            url: "https://example.com/dep.git".to_owned(),
            resolved_url: "https://example.com/dep.git".to_owned(),
            requested_rev: "refs/heads/main".to_owned(),
            commit: "a".repeat(40),
            object_format: ObjectFormat::Sha1,
            subdir: ".".to_owned(),
            tree_hash: digest('b'),
        };
        let git = package_id(&git_source);
        let lock = Lockfile {
            generated_by: "zutai-cli 0.1.0".to_owned(),
            root: root.clone(),
            packages: BTreeMap::from([
                (
                    root.clone(),
                    LockedPackage {
                        id: root.clone(),
                        name: "app".to_owned(),
                        source: root_source,
                        manifest_hash: digest('c'),
                        dependencies: BTreeMap::from([("dep".to_owned(), git.clone())]),
                    },
                ),
                (
                    git.clone(),
                    LockedPackage {
                        id: git.clone(),
                        name: "dep".to_owned(),
                        source: git_source,
                        manifest_hash: digest('d'),
                        dependencies: BTreeMap::new(),
                    },
                ),
            ]),
        };
        let rendered = render_lockfile(&lock);
        let parsed = parse_lockfile(Path::new(LOCK_NAME), &rendered).unwrap();
        assert_eq!(parsed, lock);
        assert_eq!(render_lockfile(&parsed), rendered);
    }
}
