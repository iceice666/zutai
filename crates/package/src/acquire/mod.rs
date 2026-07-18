use super::*;
use fs2::FileExt as _;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const CACHE_ENV: &str = "ZUTAI_PACKAGE_CACHE";
const MAX_ENTRIES: usize = 100_000;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TREE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_DEPTH: usize = 128;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);
const SNAPSHOT_MODE_NAME: &str = ".zutai-modes";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation<'a> {
    Sync,
    Fetch,
    Update(&'a [String]),
}

#[derive(Clone, Debug)]
pub struct AcquireOptions<'a> {
    pub root: &'a Path,
    pub cache_dir: Option<&'a Path>,
    pub offline: bool,
    pub transport_overrides: &'a [(String, PathBuf)],
    pub operation: Operation<'a>,
}

pub fn run(options: AcquireOptions<'_>) -> Result<Lockfile, PackageError> {
    let root = fs::canonicalize(options.root).map_err(|error| {
        PackageError::new(format!(
            "cannot resolve package root {}: {error}",
            options.root.display()
        ))
    })?;
    let cache = match options.cache_dir {
        Some(path) => path.to_path_buf(),
        None => configured_cache_dir()?,
    };
    fs::create_dir_all(cache.join("repos"))
        .and_then(|()| fs::create_dir_all(cache.join("snapshots")))
        .map_err(|error| PackageError::new(format!("cannot create package cache: {error}")))?;
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(cache.join(".lock"))
        .map_err(|error| PackageError::new(format!("cannot open package cache lock: {error}")))?;
    lock_file.lock_exclusive().map_err(|error| {
        PackageError::new(format!("cannot acquire package cache lock: {error}"))
    })?;

    let existing = read_lockfile(&root)?;
    let lock = match options.operation {
        Operation::Fetch => {
            let lock = existing.ok_or_else(|| {
                PackageError::new(format!(
                    "{} is missing; run `zutai-cli package sync {}` first",
                    LOCK_NAME,
                    root.display()
                ))
            })?;
            validate_local_locked_manifests(&root, &lock)?;
            fetch_locked_graph(&cache, &lock, options.offline, options.transport_overrides)?;
            validate_locked_manifests(&root, &cache, &lock)?;
            lock
        }
        Operation::Sync => {
            let mut builder = GraphBuilder::new(
                &root,
                &cache,
                existing.as_ref(),
                options.offline,
                options.transport_overrides,
            );
            let lock = builder.build(&BTreeSet::new())?;
            install_graph_snapshots(&cache, &lock)?;
            write_lockfile_atomic(&root, &lock)?;
            lock
        }
        Operation::Update(aliases) => {
            let existing = existing.ok_or_else(|| {
                PackageError::new(format!(
                    "{} is missing; run `zutai-cli package sync {}` first",
                    LOCK_NAME,
                    root.display()
                ))
            })?;
            let selected = resolve_update_targets(&existing, aliases)?;
            let mut builder = GraphBuilder::new(
                &root,
                &cache,
                Some(&existing),
                options.offline,
                options.transport_overrides,
            );
            let lock = builder.build(&selected)?;
            install_graph_snapshots(&cache, &lock)?;
            write_lockfile_atomic(&root, &lock)?;
            lock
        }
    };
    validate_lockfile(&lock)?;
    Ok(lock)
}

#[derive(Clone, Debug)]
pub struct PreparedPackageGraph {
    pub graph: PortablePackageGraph,
    pub roots: BTreeMap<String, PathBuf>,
    pub manifests: BTreeMap<PathBuf, String>,
}

pub fn prepare_graph(
    root: &Path,
    cache_dir: Option<&Path>,
) -> Result<PreparedPackageGraph, PackageError> {
    let root = fs::canonicalize(root).map_err(|error| {
        PackageError::new(format!(
            "cannot resolve package root {}: {error}",
            root.display()
        ))
    })?;
    let root_manifest_path = root.join(MANIFEST_NAME);
    let root_source = fs::read_to_string(&root_manifest_path).map_err(|error| {
        PackageError::new(format!(
            "cannot read package manifest {}: {error}",
            root_manifest_path.display()
        ))
    })?;
    let root_manifest = parse_manifest(&root_manifest_path, &root_source)?;
    if root_manifest.format_version == 1 {
        return Err(PackageError::new(
            "manifest format 1 does not use a locked package graph",
        ));
    }
    let cache = match cache_dir {
        Some(path) => path.to_path_buf(),
        None => configured_cache_dir()?,
    };
    let lock = read_lockfile(&root)?.ok_or_else(|| {
        PackageError::new(format!(
            "{} is missing for manifest format 2; run `zutai-cli package sync {}`",
            LOCK_NAME,
            root.display()
        ))
    })?;
    validate_locked_manifests(&root, &cache, &lock)?;
    let mut prepared = PreparedPackageGraph {
        graph: PortablePackageGraph {
            root_package: Some(lock.root.clone()),
            packages: BTreeMap::new(),
        },
        roots: BTreeMap::new(),
        manifests: BTreeMap::new(),
    };
    for package in lock.packages.values() {
        let package_root = package_root(&root, &cache, package)?;
        let manifest_path = package_root.join(MANIFEST_NAME);
        let manifest_source = fs::read_to_string(&manifest_path).map_err(|error| {
            PackageError::new(format!(
                "cannot read locked package manifest {}: {error}",
                manifest_path.display()
            ))
        })?;
        let manifest = parse_manifest(&manifest_path, &manifest_source)?;
        prepared
            .manifests
            .insert(manifest_path.clone(), manifest_source.clone());
        prepared
            .roots
            .insert(package.id.clone(), package_root.clone());
        let mut sources = BTreeMap::new();
        for module_path in manifest.modules.values() {
            let path = package_root.join(module_path);
            let contents = fs::read_to_string(&path).map_err(|error| {
                PackageError::new(format!(
                    "cannot read locked package module {}: {error}",
                    path.display()
                ))
            })?;
            sources.insert(module_path.clone(), contents);
        }
        prepared.graph.packages.insert(
            package.id.clone(),
            PortablePackage {
                source: match &package.source {
                    LockedSource::Path { .. } => PortablePackageSource::Path,
                    LockedSource::Git { .. } => PortablePackageSource::LockedGit,
                },
                name: package.name.clone(),
                dependencies: package.dependencies.clone(),
                modules: manifest.modules,
                sources,
            },
        );
    }
    validate_portable(&prepared.graph)?;
    Ok(prepared)
}

fn configured_cache_dir() -> Result<PathBuf, PackageError> {
    if let Some(path) = std::env::var_os(CACHE_ENV) {
        return Ok(PathBuf::from(path));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        Some(PathBuf::from(path))
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"))
    };
    base.map(|path| path.join("zutai").join("packages"))
        .ok_or_else(|| {
            PackageError::new(format!(
                "cannot determine package cache directory; set {CACHE_ENV}"
            ))
        })
}

struct GraphBuilder<'a> {
    root: &'a Path,
    cache: &'a Path,
    existing: Option<&'a Lockfile>,
    offline: bool,
    transport_overrides: &'a [(String, PathBuf)],
    packages: BTreeMap<String, LockedPackage>,
    local_identities: BTreeMap<PathBuf, String>,
    existing_edges: BTreeMap<(String, String), String>,
    active: BTreeSet<String>,
}

impl<'a> GraphBuilder<'a> {
    fn new(
        root: &'a Path,
        cache: &'a Path,
        existing: Option<&'a Lockfile>,
        offline: bool,
        transport_overrides: &'a [(String, PathBuf)],
    ) -> Self {
        Self {
            root,
            cache,
            existing,
            offline,
            transport_overrides,
            packages: BTreeMap::new(),
            local_identities: BTreeMap::new(),
            existing_edges: existing.map(existing_edge_index).unwrap_or_default(),
            active: BTreeSet::new(),
        }
    }

    fn build(&mut self, updates: &BTreeSet<String>) -> Result<Lockfile, PackageError> {
        let root = self.load_local(self.root, ".", updates, false)?;
        Ok(Lockfile {
            generated_by: format!("zutai-cli {COMPILER_VERSION}"),
            root,
            packages: std::mem::take(&mut self.packages),
        })
    }

    fn load_local(
        &mut self,
        package_root: &Path,
        identity_path: &str,
        updates: &BTreeSet<String>,
        update_all: bool,
    ) -> Result<String, PackageError> {
        let canonical = fs::canonicalize(package_root).map_err(|error| {
            PackageError::new(format!(
                "cannot resolve local package {}: {error}",
                package_root.display()
            ))
        })?;
        if let Some(existing) = self.local_identities.get(&canonical) {
            if existing != identity_path {
                return Err(PackageError::new(format!(
                    "local package {} is reachable through both {existing:?} and {identity_path:?}",
                    canonical.display()
                )));
            }
        } else {
            self.local_identities
                .insert(canonical.clone(), identity_path.to_owned());
        }
        let source = LockedSource::Path {
            path: identity_path.to_owned(),
        };
        let id = package_id(&source);
        if self.packages.contains_key(&id) {
            return Ok(id);
        }
        if !self.active.insert(id.clone()) {
            return Err(PackageError::new(format!(
                "package dependency cycle through {identity_path:?}"
            )));
        }
        let manifest_path = canonical.join(MANIFEST_NAME);
        let manifest_source = fs::read_to_string(&manifest_path).map_err(|error| {
            PackageError::new(format!(
                "cannot read package manifest {}: {error}",
                manifest_path.display()
            ))
        })?;
        let manifest = parse_manifest(&manifest_path, &manifest_source)?;
        let mut dependencies = BTreeMap::new();
        for dependency in &manifest.dependencies {
            if dependencies.contains_key(&dependency.alias) {
                return Err(PackageError::at(
                    &manifest_path,
                    dependency.alias_span,
                    format!("duplicate dependency alias {:?}", dependency.alias),
                ));
            }
            let target = match &dependency.source {
                ManifestSource::Path { path } => {
                    let path = normalize_path_dependency(path)?;
                    let dependency_root = canonical.join(&path);
                    if manifest.format_version == 2 {
                        let relative = pathdiff::diff_paths(&dependency_root, self.root)
                            .ok_or_else(|| {
                                PackageError::new("cannot derive root-relative package path")
                            })?;
                        let identity = slash_path(&relative)?;
                        self.load_local(&dependency_root, &identity, updates, update_all)?
                    } else {
                        self.load_local(&dependency_root, &path, updates, update_all)?
                    }
                }
                ManifestSource::Git { url, rev, subdir } => {
                    let prior_id = self
                        .existing_edges
                        .get(&(id.clone(), dependency.alias.clone()))
                        .cloned();
                    self.load_git(
                        url,
                        rev,
                        subdir,
                        update_all || updates.contains(&dependency.alias),
                        prior_id.as_deref(),
                    )?
                }
            };
            dependencies.insert(dependency.alias.clone(), target);
        }
        self.active.remove(&id);
        self.packages.insert(
            id.clone(),
            LockedPackage {
                id: id.clone(),
                name: manifest.name,
                source,
                manifest_hash: sha256_digest(manifest_source.as_bytes()),
                dependencies,
            },
        );
        Ok(id)
    }

    fn load_git(
        &mut self,
        url: &str,
        requested_rev: &str,
        subdir: &str,
        update: bool,
        prior_id: Option<&str>,
    ) -> Result<String, PackageError> {
        self.load_git_source(url, requested_rev, subdir, update, prior_id, None)
    }

    fn load_git_source(
        &mut self,
        url: &str,
        requested_rev: &str,
        subdir: &str,
        update: bool,
        prior_id: Option<&str>,
        inherited: Option<(&str, &str, ObjectFormat, &str)>,
    ) -> Result<String, PackageError> {
        let url = normalize_git_url(url)?;
        let subdir = normalize_subdir(subdir)?;
        let preserved = prior_id
            .and_then(|id| self.existing?.packages.get(id))
            .or_else(|| {
                self.existing?.packages.values().find(|package| {
                    matches!(
                        &package.source,
                        LockedSource::Git {
                            url: locked_url,
                            requested_rev: locked_rev,
                            ..
                        } if locked_url == &url && locked_rev == requested_rev
                    )
                })
            });
        let (resolved_url, commit, object_format, tree_hash) = if let Some((
            inherited_url,
            inherited_commit,
            inherited_format,
            inherited_tree_hash,
        )) = inherited
        {
            (
                inherited_url.to_owned(),
                inherited_commit.to_owned(),
                inherited_format,
                inherited_tree_hash.to_owned(),
            )
        } else if let Some(package) = preserved.filter(|_| !update || self.offline) {
            let LockedSource::Git {
                resolved_url,
                commit,
                object_format,
                tree_hash,
                ..
            } = &package.source
            else {
                unreachable!()
            };
            (
                resolved_url.clone(),
                commit.clone(),
                *object_format,
                tree_hash.clone(),
            )
        } else {
            if self.offline {
                return Err(PackageError::new(format!(
                    "cannot resolve new or updated Git selector {url} {requested_rev:?} while offline"
                )));
            }
            let (resolved_url, commit, object_format) =
                resolve_revision(self.cache, &url, requested_rev, self.transport_overrides)?;
            let tree_hash = hash_repository_tree(self.cache, &resolved_url, &commit)?;
            (resolved_url, commit, object_format, tree_hash)
        };
        ensure_repository(
            self.cache,
            &resolved_url,
            &commit,
            self.offline,
            self.transport_overrides,
        )?;
        if inherited.is_none() && !snapshot_path(self.cache, &tree_hash).is_dir() {
            let found = hash_repository_tree(self.cache, &resolved_url, &commit)?;
            if found != tree_hash {
                return Err(PackageError::new(format!(
                    "locked Git tree changed for {resolved_url}: expected {tree_hash}, found {found}"
                )));
            }
        }
        let source = LockedSource::Git {
            url: url.clone(),
            resolved_url: resolved_url.clone(),
            requested_rev: requested_rev.to_owned(),
            commit: commit.clone(),
            object_format,
            subdir: subdir.clone(),
            tree_hash: tree_hash.clone(),
        };
        let id = package_id(&source);
        if self.packages.contains_key(&id) {
            return Ok(id);
        }
        if !self.active.insert(id.clone()) {
            return Err(PackageError::new(format!(
                "package dependency cycle through {id:?}"
            )));
        }
        ensure_snapshot(self.cache, &source)?;
        let snapshot = snapshot_path(self.cache, source_tree_hash(&source));
        let package_root = snapshot.join(&subdir);
        let manifest_path = package_root.join(MANIFEST_NAME);
        let manifest_source = fs::read_to_string(&manifest_path).map_err(|error| {
            PackageError::new(format!(
                "cannot read package manifest {}: {error}",
                manifest_path.display()
            ))
        })?;
        let manifest = parse_manifest(&manifest_path, &manifest_source)?;
        if manifest.format_version != 2 {
            return Err(PackageError::new(format!(
                "Git package manifest {} must use formatVersion = 2",
                manifest_path.display()
            )));
        }
        let mut dependencies = BTreeMap::new();
        for dependency in &manifest.dependencies {
            if dependencies.contains_key(&dependency.alias) {
                return Err(PackageError::at(
                    &manifest_path,
                    dependency.alias_span,
                    format!("duplicate dependency alias {:?}", dependency.alias),
                ));
            }
            let target = match &dependency.source {
                ManifestSource::Path { path } => {
                    let path = normalize_path_dependency(path)?;
                    let target_subdir = join_git_subdir(&subdir, &path)?;
                    self.load_git_source(
                        &url,
                        requested_rev,
                        &target_subdir,
                        update,
                        None,
                        Some((&resolved_url, &commit, object_format, &tree_hash)),
                    )?
                }
                ManifestSource::Git { url, rev, subdir } => {
                    self.load_git(url, rev, subdir, update, None)?
                }
            };
            dependencies.insert(dependency.alias.clone(), target);
        }
        self.active.remove(&id);
        self.packages.insert(
            id.clone(),
            LockedPackage {
                id: id.clone(),
                name: manifest.name,
                source,
                manifest_hash: sha256_digest(manifest_source.as_bytes()),
                dependencies,
            },
        );
        Ok(id)
    }
}

fn existing_edge_index(lock: &Lockfile) -> BTreeMap<(String, String), String> {
    let mut edges = BTreeMap::new();
    for (id, package) in &lock.packages {
        for (alias, target) in &package.dependencies {
            edges.insert((id.clone(), alias.clone()), target.clone());
        }
    }
    edges
}

fn resolve_update_targets(
    lock: &Lockfile,
    aliases: &[String],
) -> Result<BTreeSet<String>, PackageError> {
    if aliases.is_empty() {
        return Ok(lock.packages[&lock.root]
            .dependencies
            .keys()
            .cloned()
            .collect());
    }
    let root = &lock.packages[&lock.root];
    let mut selected = BTreeSet::new();
    for alias in aliases {
        if !root.dependencies.contains_key(alias) {
            return Err(PackageError::new(format!(
                "unknown root package dependency alias {alias:?}"
            )));
        }
        selected.insert(alias.clone());
    }
    Ok(selected)
}

fn read_lockfile(root: &Path) -> Result<Option<Lockfile>, PackageError> {
    let path = root.join(LOCK_NAME);
    match fs::read_to_string(&path) {
        Ok(source) => parse_lockfile(&path, &source).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(PackageError::new(format!(
            "cannot read package lock {}: {error}",
            path.display()
        ))),
    }
}

mod git;
mod lock;
#[cfg(test)]
mod tests;

use git::*;
use lock::*;

fn write_lockfile_atomic(root: &Path, lock: &Lockfile) -> Result<(), PackageError> {
    let target = root.join(LOCK_NAME);
    let temp = root.join(format!(".{LOCK_NAME}.tmp-{}", std::process::id()));
    let content = render_lockfile(lock);
    let mut file = File::create(&temp).map_err(io_error("create temporary package lock"))?;
    file.write_all(content.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io_error("write temporary package lock"))?;
    fs::rename(&temp, &target).map_err(io_error("install package lock"))
}
