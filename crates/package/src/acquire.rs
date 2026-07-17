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

fn validate_local_locked_manifests(root: &Path, lock: &Lockfile) -> Result<(), PackageError> {
    validate_lockfile(lock)?;
    for package in lock.packages.values() {
        if matches!(package.source, LockedSource::Path { .. }) {
            validate_locked_manifest(root, root, lock, package)?;
        }
    }
    Ok(())
}

fn validate_locked_manifests(
    root: &Path,
    cache: &Path,
    lock: &Lockfile,
) -> Result<(), PackageError> {
    validate_lockfile(lock)?;
    for package in lock.packages.values() {
        validate_locked_manifest(root, cache, lock, package)?;
    }
    Ok(())
}

fn validate_locked_manifest(
    root: &Path,
    cache: &Path,
    lock: &Lockfile,
    package: &LockedPackage,
) -> Result<(), PackageError> {
    let package_root = package_root(root, cache, package)?;
    let manifest_path = package_root.join(MANIFEST_NAME);
    let source = fs::read_to_string(&manifest_path).map_err(|error| {
        PackageError::new(format!(
            "cannot read locked package manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    let found = sha256_digest(source.as_bytes());
    if found != package.manifest_hash {
        return Err(PackageError::new(format!(
            "package lock is stale for {}; run `zutai-cli package sync {}`",
            manifest_path.display(),
            root.display()
        )));
    }
    let manifest = parse_manifest(&manifest_path, &source)?;
    let expected: BTreeSet<_> = manifest
        .dependencies
        .iter()
        .map(|dependency| dependency.alias.as_str())
        .collect();
    let locked: BTreeSet<_> = package.dependencies.keys().map(String::as_str).collect();
    if expected != locked {
        return Err(stale_edges_error(root, &manifest_path));
    }
    for dependency in &manifest.dependencies {
        let target_id = &package.dependencies[&dependency.alias];
        let target = &lock.packages[target_id];
        if !locked_dependency_matches(package, dependency, target)?
            && !equivalent_git_dependency(dependency, target)?
        {
            return Err(PackageError::new(format!(
                "package lock dependency edge {:?} is stale in {}; run `zutai-cli package sync {}`",
                dependency.alias,
                manifest_path.display(),
                root.display()
            )));
        }
    }
    Ok(())
}

fn stale_edges_error(root: &Path, manifest_path: &Path) -> PackageError {
    PackageError::new(format!(
        "package lock dependency edges are stale for {}; run `zutai-cli package sync {}`",
        manifest_path.display(),
        root.display()
    ))
}

fn locked_dependency_matches(
    owner: &LockedPackage,
    dependency: &ManifestDependency,
    target: &LockedPackage,
) -> Result<bool, PackageError> {
    match (&owner.source, &dependency.source, &target.source) {
        (
            LockedSource::Path { path: owner_path },
            ManifestSource::Path { path },
            LockedSource::Path { path: target_path },
        ) => {
            let path = normalize_path_dependency(path)?;
            Ok(normalize_local_identity(owner_path, &path)? == *target_path)
        }
        (
            LockedSource::Path { .. },
            ManifestSource::Git { url, rev, subdir },
            LockedSource::Git {
                url: target_url,
                requested_rev,
                subdir: target_subdir,
                ..
            },
        ) => Ok(normalize_git_url(url)? == *target_url
            && git_selector_matches(rev, requested_rev, target)
            && normalize_subdir(subdir)? == *target_subdir),
        (
            LockedSource::Git {
                url,
                commit,
                object_format,
                subdir,
                tree_hash,
                ..
            },
            ManifestSource::Path { path },
            LockedSource::Git {
                url: target_url,
                commit: target_commit,
                object_format: target_format,
                subdir: target_subdir,
                tree_hash: target_tree_hash,
                ..
            },
        ) => Ok(url == target_url
            && commit == target_commit
            && object_format == target_format
            && tree_hash == target_tree_hash
            && join_git_subdir(subdir, &normalize_path_dependency(path)?)? == *target_subdir),
        (
            LockedSource::Git { .. },
            ManifestSource::Git { url, rev, subdir },
            LockedSource::Git {
                url: target_url,
                requested_rev,
                subdir: target_subdir,
                ..
            },
        ) => Ok(normalize_git_url(url)? == *target_url
            && git_selector_matches(rev, requested_rev, target)
            && normalize_subdir(subdir)? == *target_subdir),
        _ => Ok(false),
    }
}
fn equivalent_git_dependency(
    dependency: &ManifestDependency,
    target: &LockedPackage,
) -> Result<bool, PackageError> {
    let (
        ManifestSource::Git { url, subdir, .. },
        LockedSource::Git {
            url: target_url,
            subdir: target_subdir,
            ..
        },
    ) = (&dependency.source, &target.source)
    else {
        return Ok(false);
    };
    Ok(normalize_git_url(url)? == *target_url && normalize_subdir(subdir)? == *target_subdir)
}

fn git_selector_matches(declared: &str, locked: &str, target: &LockedPackage) -> bool {
    if declared == locked {
        return true;
    }
    matches!(&target.source, LockedSource::Git { commit, .. } if declared == commit || locked == commit)
}

fn normalize_local_identity(owner: &str, dependency: &str) -> Result<String, PackageError> {
    let mut parts = Vec::new();
    let base = Path::new(owner).parent().unwrap_or(Path::new("."));
    for component in base.components().chain(Path::new(dependency).components()) {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(part) => parts.push(part.to_owned()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(PackageError::new("local package identity must be relative"));
            }
        }
    }
    if parts.is_empty() {
        Ok(".".to_owned())
    } else {
        slash_path(&parts.iter().collect::<PathBuf>())
    }
}

fn fetch_locked_graph(
    cache: &Path,
    lock: &Lockfile,
    offline: bool,
    transport_overrides: &[(String, PathBuf)],
) -> Result<(), PackageError> {
    for package in lock.packages.values() {
        if let LockedSource::Git {
            resolved_url,
            commit,
            ..
        } = &package.source
        {
            ensure_repository(cache, resolved_url, commit, offline, transport_overrides)?;
            ensure_snapshot(cache, &package.source)?;
        }
    }
    Ok(())
}

fn install_graph_snapshots(cache: &Path, lock: &Lockfile) -> Result<(), PackageError> {
    for package in lock.packages.values() {
        if matches!(package.source, LockedSource::Git { .. }) {
            ensure_snapshot(cache, &package.source)?;
        }
    }
    Ok(())
}

fn package_root(
    root: &Path,
    cache: &Path,
    package: &LockedPackage,
) -> Result<PathBuf, PackageError> {
    match &package.source {
        LockedSource::Path { path } => Ok(root.join(path)),
        LockedSource::Git {
            subdir, tree_hash, ..
        } => {
            let snapshot = snapshot_path(cache, tree_hash);
            if !snapshot.is_dir() {
                return Err(PackageError::new(format!(
                    "locked package snapshot {tree_hash} is missing; run `zutai-cli package fetch {}`",
                    root.display()
                )));
            }
            verify_snapshot(&snapshot, tree_hash)?;
            Ok(snapshot.join(subdir))
        }
    }
}

fn resolve_revision(
    cache: &Path,
    url: &str,
    requested_rev: &str,
    overrides: &[(String, PathBuf)],
) -> Result<(String, String, ObjectFormat), PackageError> {
    let repo = repo_path(cache, url);
    ensure_bare_repository(&repo, url, false)?;
    let transport = transport_url(url, overrides);
    git(
        &repo,
        [
            "fetch",
            "--force",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            transport.as_str(),
            requested_rev,
        ],
    )?;
    let commit = git_text(&repo, ["rev-parse", "--verify", "FETCH_HEAD^{commit}"])?;
    let object_format = match git_text(&repo, ["rev-parse", "--show-object-format"])?.as_str() {
        "sha1" => ObjectFormat::Sha1,
        "sha256" => ObjectFormat::Sha256,
        other => {
            return Err(PackageError::new(format!(
                "unsupported Git object format {other:?}"
            )));
        }
    };
    validate_object_id(&commit, object_format)?;
    Ok((url.to_owned(), commit, object_format))
}

fn transport_url(url: &str, overrides: &[(String, PathBuf)]) -> String {
    overrides
        .iter()
        .find(|(candidate, _)| candidate == url)
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| url.to_owned())
}

fn ensure_repository(
    cache: &Path,
    url: &str,
    commit: &str,
    offline: bool,
    transport_overrides: &[(String, PathBuf)],
) -> Result<(), PackageError> {
    let repo = repo_path(cache, url);
    ensure_bare_repository(&repo, url, offline)?;
    if git_status(&repo, ["cat-file", "-e", &format!("{commit}^{{commit}}")])? {
        return Ok(());
    }
    if offline {
        return Err(PackageError::new(format!(
            "locked Git commit {commit} is absent from the package cache while offline"
        )));
    }
    let transport = transport_url(url, transport_overrides);
    git(
        &repo,
        [
            "fetch",
            "--force",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            transport.as_str(),
            commit,
        ],
    )?;
    if git_status(&repo, ["cat-file", "-e", &format!("{commit}^{{commit}}")])? {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "Git repository {url} did not provide locked commit {commit}"
        )))
    }
}

fn ensure_bare_repository(repo: &Path, url: &str, offline: bool) -> Result<(), PackageError> {
    if repo.join("HEAD").is_file() {
        return Ok(());
    }
    if offline {
        return Err(PackageError::new(format!(
            "Git repository {url} is absent from the package cache while offline"
        )));
    }
    fs::create_dir_all(repo.parent().expect("repo path has parent"))
        .map_err(|error| PackageError::new(format!("cannot create Git cache: {error}")))?;
    let temp = temp_sibling(repo, "repo");
    if temp.exists() {
        fs::remove_dir_all(&temp).map_err(io_error("remove temporary Git cache"))?;
    }
    git_no_repo(["init", "--bare", path_arg(&temp)?])?;
    fs::rename(&temp, repo).map_err(io_error("install Git cache"))?;
    Ok(())
}

fn ensure_snapshot(cache: &Path, source: &LockedSource) -> Result<(), PackageError> {
    let LockedSource::Git {
        resolved_url,
        commit,
        tree_hash,
        ..
    } = source
    else {
        return Ok(());
    };
    let target = snapshot_path(cache, tree_hash);
    if target.is_dir() {
        return verify_snapshot(&target, tree_hash);
    }
    let repo = repo_path(cache, resolved_url);
    if !git_status(&repo, ["cat-file", "-e", &format!("{commit}^{{commit}}")])? {
        return Err(PackageError::new(format!(
            "locked Git commit {commit} is absent from the package cache"
        )));
    }
    let temp = temp_sibling(&target, "snapshot");
    if temp.exists() {
        fs::remove_dir_all(&temp).map_err(io_error("remove temporary snapshot"))?;
    }
    fs::create_dir_all(&temp).map_err(io_error("create temporary snapshot"))?;
    materialize_tree(&repo, commit, &temp)?;
    let found = hash_tree(&temp)?;
    if &found != tree_hash {
        fs::remove_dir_all(&temp).ok();
        return Err(PackageError::new(format!(
            "fetched package tree hash mismatch: lock expects {tree_hash}, repository produced {found}"
        )));
    }
    fs::create_dir_all(target.parent().expect("snapshot path has parent"))
        .map_err(io_error("create snapshot cache"))?;
    match fs::rename(&temp, &target) {
        Ok(()) => Ok(()),
        Err(_) if target.is_dir() => {
            fs::remove_dir_all(&temp).ok();
            verify_snapshot(&target, tree_hash)
        }
        Err(error) => Err(PackageError::new(format!(
            "cannot install package snapshot {}: {error}",
            target.display()
        ))),
    }
}

fn hash_repository_tree(cache: &Path, url: &str, commit: &str) -> Result<String, PackageError> {
    hash_repository_listing(&repo_path(cache, url), commit)
}

fn materialize_tree(repo: &Path, commit: &str, output: &Path) -> Result<(), PackageError> {
    let listing = git_bytes(repo, ["ls-tree", "-rz", "--full-tree", commit])?;
    let mut entries = parse_tree_entries(&listing)?;
    populate_tree_sizes(repo, &mut entries)?;
    let mut total = 0_u64;
    for entry in &entries {
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| PackageError::new("Git package expanded size overflow"))?;
        if total > MAX_TREE_BYTES {
            return Err(PackageError::new(
                "Git package exceeds the total expanded size limit",
            ));
        }
        let bytes = git_bytes(repo, ["cat-file", "blob", entry.object.as_str()])?;
        if bytes.len() as u64 != entry.size {
            return Err(PackageError::new(format!(
                "Git blob size changed while reading {:?}",
                entry.path
            )));
        }
        let destination = output.join(&entry.path);
        fs::create_dir_all(destination.parent().expect("tree file has parent"))
            .map_err(io_error("create snapshot directory"))?;
        fs::write(&destination, bytes).map_err(io_error("write snapshot file"))?;
        set_executable_mode(&destination, &entry.mode)?;
        set_readonly(&destination)?;
    }
    write_snapshot_modes(output, &entries)?;
    set_tree_readonly(output)?;
    Ok(())
}

#[derive(Clone, Debug)]
struct TreeEntry {
    mode: String,
    object: String,
    path: String,
    size: u64,
}

fn parse_tree_entries(listing: &[u8]) -> Result<Vec<TreeEntry>, PackageError> {
    let mut entries = Vec::new();
    for record in listing
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if entries.len() == MAX_ENTRIES {
            return Err(PackageError::new(
                "Git package exceeds the maximum entry count",
            ));
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| PackageError::new("Git returned a malformed tree entry"))?;
        let meta = std::str::from_utf8(&record[..tab])
            .map_err(|_| PackageError::new("Git tree metadata is not UTF-8"))?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| PackageError::new("Git package contains a non-UTF-8 path"))?;
        if path == SNAPSHOT_MODE_NAME {
            return Err(PackageError::new(format!(
                "Git package contains reserved snapshot metadata path {path:?}"
            )));
        }
        let mut meta = meta.split_ascii_whitespace();
        let mode = meta.next().unwrap_or_default();
        let kind = meta.next().unwrap_or_default();
        let object = meta.next().unwrap_or_default();
        if kind == "tree" {
            continue;
        }
        if kind != "blob" || !matches!(mode, "100644" | "100755") {
            return Err(PackageError::new(format!(
                "Git package contains unsupported tree entry {path:?} with mode {mode} and type {kind}"
            )));
        }
        validate_tree_path(path)?;
        entries.push(TreeEntry {
            mode: mode.to_owned(),
            object: object.to_owned(),
            path: path.to_owned(),
            size: 0,
        });
    }
    Ok(entries)
}

fn write_snapshot_modes(root: &Path, entries: &[TreeEntry]) -> Result<(), PackageError> {
    let mut content = String::new();
    for entry in entries {
        content.push_str(&entry.mode);
        content.push(' ');
        content.push_str(&entry.path);
        content.push('\n');
    }
    fs::write(root.join(SNAPSHOT_MODE_NAME), content).map_err(io_error("write snapshot modes"))
}

fn populate_tree_sizes(repo: &Path, entries: &mut [TreeEntry]) -> Result<(), PackageError> {
    for entry in entries {
        let size_text = git_text(repo, ["cat-file", "-s", entry.object.as_str()])?;
        entry.size = size_text
            .parse()
            .map_err(|_| PackageError::new("Git returned an invalid blob size"))?;
        if entry.size > MAX_FILE_BYTES {
            return Err(PackageError::new(format!(
                "Git package file {:?} exceeds the per-file size limit",
                entry.path
            )));
        }
    }
    Ok(())
}

fn validate_tree_path(path: &str) -> Result<(), PackageError> {
    if path.len() > MAX_PATH_BYTES
        || path.contains('\\')
        || path.bytes().any(|byte| byte <= 0x1f || byte == 0x7f)
        || path.split('/').count() > MAX_PATH_DEPTH
        || path.split('/').any(|part| {
            part.is_empty()
                || matches!(part, "." | ".." | ".git")
                || part.ends_with('.')
                || part.ends_with(' ')
                || is_windows_reserved(part)
        })
    {
        return Err(PackageError::new(format!(
            "Git package contains unsafe portable path {path:?}"
        )));
    }
    Ok(())
}

fn is_windows_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && matches!(&stem[..3], "com" | "lpt")
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn hash_repository_listing(repo: &Path, commit: &str) -> Result<String, PackageError> {
    let listing = git_bytes(repo, ["ls-tree", "-rz", "--full-tree", commit])?;
    let mut entries = parse_tree_entries(&listing)?;
    populate_tree_sizes(repo, &mut entries)?;
    let mut digest = Sha256::new();
    digest.update(b"zutai-git-tree-v1");
    let mut total = 0_u64;
    for entry in entries {
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| PackageError::new("Git package expanded size overflow"))?;
        if total > MAX_TREE_BYTES {
            return Err(PackageError::new(
                "Git package exceeds the total expanded size limit",
            ));
        }
        update_part(&mut digest, entry.mode.as_bytes());
        update_part(&mut digest, entry.path.as_bytes());
        digest.update(entry.size.to_le_bytes());
        let bytes = git_bytes(repo, ["cat-file", "blob", entry.object.as_str()])?;
        if bytes.len() as u64 != entry.size {
            return Err(PackageError::new(format!(
                "Git blob size changed while reading {:?}",
                entry.path
            )));
        }
        digest.update(bytes);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn hash_tree(root: &Path) -> Result<String, PackageError> {
    let mut entries = Vec::new();
    collect_files(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let modes = read_snapshot_modes(root)?;
    let mut digest = Sha256::new();
    digest.update(b"zutai-git-tree-v1");
    let mut total = 0_u64;
    let entry_count = entries.len();
    for (path, filesystem_path) in entries {
        let metadata =
            fs::metadata(&filesystem_path).map_err(io_error("read snapshot metadata"))?;
        let size = metadata.len();
        total = total
            .checked_add(size)
            .ok_or_else(|| PackageError::new("Git package expanded size overflow"))?;
        if size > MAX_FILE_BYTES || total > MAX_TREE_BYTES {
            return Err(PackageError::new(
                "Git package exceeds configured size limits",
            ));
        }
        let mode = modes.get(&path).ok_or_else(|| {
            PackageError::new(format!(
                "package snapshot mode metadata is missing {path:?}"
            ))
        })?;
        update_part(&mut digest, mode.as_bytes());
        update_part(&mut digest, path.as_bytes());
        digest.update(size.to_le_bytes());
        let mut file = File::open(&filesystem_path).map_err(io_error("open snapshot file"))?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(io_error("read snapshot file"))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
    }
    if modes.len() != entry_count {
        return Err(PackageError::new(
            "package snapshot mode metadata does not match snapshot files",
        ));
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn read_snapshot_modes(root: &Path) -> Result<BTreeMap<String, String>, PackageError> {
    let source = fs::read_to_string(root.join(SNAPSHOT_MODE_NAME))
        .map_err(io_error("read snapshot modes"))?;
    let mut modes = BTreeMap::new();
    for line in source.lines() {
        let Some((mode, entry_path)) = line.split_once(' ') else {
            return Err(PackageError::new(
                "package snapshot mode metadata is malformed",
            ));
        };
        if !matches!(mode, "100644" | "100755") || validate_tree_path(entry_path).is_err() {
            return Err(PackageError::new(
                "package snapshot mode metadata is invalid",
            ));
        }
        if modes
            .insert(entry_path.to_owned(), mode.to_owned())
            .is_some()
        {
            return Err(PackageError::new(
                "package snapshot mode metadata has duplicate paths",
            ));
        }
    }
    Ok(modes)
}

fn collect_files(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<(String, PathBuf)>,
) -> Result<(), PackageError> {
    for entry in fs::read_dir(dir).map_err(io_error("read snapshot directory"))? {
        let entry = entry.map_err(io_error("read snapshot entry"))?;
        let file_type = entry
            .file_type()
            .map_err(io_error("read snapshot file type"))?;
        let path = entry.path();
        if file_type.is_symlink() || (!file_type.is_dir() && !file_type.is_file()) {
            return Err(PackageError::new(format!(
                "package snapshot contains unsupported entry {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_files(root, &path, entries)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("collected file is below root");
            let relative = slash_path(relative)?;
            if relative != SNAPSHOT_MODE_NAME {
                entries.push((relative, path));
            }
        }
        if entries.len() > MAX_ENTRIES {
            return Err(PackageError::new(
                "package snapshot exceeds the maximum entry count",
            ));
        }
    }
    Ok(())
}

fn verify_snapshot(path: &Path, expected: &str) -> Result<(), PackageError> {
    let found = hash_tree(path)?;
    if found == expected {
        Ok(())
    } else {
        Err(PackageError::new(format!(
            "cached package snapshot {} was modified: expected {expected}, found {found}; remove it and run `zutai-cli package fetch`",
            path.display()
        )))
    }
}

fn source_tree_hash(source: &LockedSource) -> &str {
    match source {
        LockedSource::Git { tree_hash, .. } => tree_hash,
        LockedSource::Path { .. } => unreachable!("local sources have no tree hash"),
    }
}

fn repo_path(cache: &Path, url: &str) -> PathBuf {
    cache
        .join("repos")
        .join(sha256_digest(url.as_bytes()).trim_start_matches("sha256:"))
}

fn snapshot_path(cache: &Path, tree_hash: &str) -> PathBuf {
    cache
        .join("snapshots")
        .join(tree_hash.trim_start_matches("sha256:"))
}

fn temp_sibling(path: &Path, kind: &str) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    path.parent()
        .expect("cache paths have parents")
        .join(format!(".zutai-{kind}-{}-{id}", std::process::id()))
}

fn join_git_subdir(base: &str, relative: &str) -> Result<String, PackageError> {
    let mut parts = if base == "." {
        Vec::new()
    } else {
        base.split('/').map(str::to_owned).collect()
    };
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(PackageError::new(
                        "Git path dependency escapes the locked repository tree",
                    ));
                }
            }
            part => parts.push(part.to_owned()),
        }
    }
    Ok(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

fn slash_path(path: &Path) -> Result<String, PackageError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| PackageError::new("package path is not UTF-8"))?
                    .to_owned(),
            ),
            Component::ParentDir => parts.push("..".to_owned()),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => {
                return Err(PackageError::new("package path is not relative"));
            }
        }
    }
    Ok(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

#[cfg(unix)]
fn set_executable_mode(path: &Path, mode: &str) -> Result<(), PackageError> {
    use std::os::unix::fs::PermissionsExt as _;
    let permissions = fs::Permissions::from_mode(if mode == "100755" { 0o755 } else { 0o644 });
    fs::set_permissions(path, permissions).map_err(io_error("set snapshot executable mode"))
}

#[cfg(not(unix))]
fn set_executable_mode(_path: &Path, _mode: &str) -> Result<(), PackageError> {
    Ok(())
}

fn set_readonly(path: &Path) -> Result<(), PackageError> {
    let mut permissions = fs::metadata(path)
        .map_err(io_error("read snapshot permissions"))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(io_error("set snapshot read-only"))
}

fn set_tree_readonly(root: &Path) -> Result<(), PackageError> {
    let mut dirs = vec![root.to_path_buf()];
    let mut index = 0;
    while index < dirs.len() {
        let dir = dirs[index].clone();
        index += 1;
        for entry in fs::read_dir(&dir).map_err(io_error("read snapshot directory"))? {
            let entry = entry.map_err(io_error("read snapshot entry"))?;
            if entry
                .file_type()
                .map_err(io_error("read snapshot type"))?
                .is_dir()
            {
                dirs.push(entry.path());
            }
        }
    }
    for dir in dirs.into_iter().rev() {
        set_readonly(&dir)?;
    }
    Ok(())
}

fn git<const N: usize>(repo: &Path, args: [&str; N]) -> Result<(), PackageError> {
    let output = git_output(Some(repo), args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(&output))
    }
}

fn git_text<const N: usize>(repo: &Path, args: [&str; N]) -> Result<String, PackageError> {
    let output = git_output(Some(repo), args)?;
    if !output.status.success() {
        return Err(git_failure(&output));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|_| PackageError::new("Git returned non-UTF-8 text"))
}

fn git_bytes<const N: usize>(repo: &Path, args: [&str; N]) -> Result<Vec<u8>, PackageError> {
    let output = git_output(Some(repo), args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(git_failure(&output))
    }
}

fn git_status<const N: usize>(repo: &Path, args: [&str; N]) -> Result<bool, PackageError> {
    Ok(git_output(Some(repo), args)?.status.success())
}

fn git_no_repo<const N: usize>(args: [&str; N]) -> Result<(), PackageError> {
    let output = git_output(None, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(&output))
    }
}

fn git_output<const N: usize>(
    repo: Option<&Path>,
    args: [&str; N],
) -> Result<Output, PackageError> {
    let mut command = Command::new("git");
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_COUNT", "5")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", null_device())
        .env("GIT_CONFIG_KEY_1", "filter.lfs.smudge")
        .env("GIT_CONFIG_VALUE_1", "cat")
        .env("GIT_CONFIG_KEY_2", "filter.lfs.required")
        .env("GIT_CONFIG_VALUE_2", "false")
        .env("GIT_CONFIG_KEY_3", "submodule.recurse")
        .env("GIT_CONFIG_VALUE_3", "false")
        .env("GIT_CONFIG_KEY_4", "fetch.recurseSubmodules")
        .env("GIT_CONFIG_VALUE_4", "false")
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(repo) = repo {
        command.arg("--git-dir").arg(repo);
    }
    command.args(args);
    command.output().map_err(|error| {
        PackageError::new(format!(
            "required tool `git` failed to start for package acquisition: {error}; install Git or run from a dev shell that provides it"
        ))
    })
}

fn null_device() -> &'static OsStr {
    if cfg!(windows) {
        OsStr::new("NUL")
    } else {
        OsStr::new("/dev/null")
    }
}

fn git_failure(output: &Output) -> PackageError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    PackageError::new(format!(
        "Git package acquisition failed{}",
        stderr
            .lines()
            .next()
            .map(|line| format!(": {line}"))
            .unwrap_or_default()
    ))
}

fn path_arg(path: &Path) -> Result<&str, PackageError> {
    path.to_str()
        .ok_or_else(|| PackageError::new("package cache path is not UTF-8"))
}

fn io_error(purpose: &'static str) -> impl Fn(std::io::Error) -> PackageError {
    move |error| PackageError::new(format!("cannot {purpose}: {error}"))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_path_dependencies_cannot_escape_repository() {
        assert_eq!(join_git_subdir("packages/a", "../b").unwrap(), "packages/b");
        assert!(join_git_subdir(".", "../b").is_err());
    }

    #[test]
    fn tree_paths_reject_nonportable_entries() {
        for path in [
            "a/../b",
            "a\\b",
            "nul",
            "deep/.git/config",
            "bad. ",
            "line\nbreak",
            "carriage\rreturn",
            "delete\u{7f}",
        ] {
            assert!(validate_tree_path(path).is_err(), "accepted {path:?}");
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("zutai-{name}-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            make_writable(&self.0);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn locked_git_graph_is_deterministic_offline_and_tamper_evident() {
        let temp = TempDir::new("package-git");
        let repo = temp.0.join("repo");
        let app = temp.0.join("app");
        let cache = temp.0.join("cache");
        fs::create_dir_all(repo.join("packages/math/src")).unwrap();
        fs::create_dir_all(repo.join("packages/util/src")).unwrap();
        fs::create_dir_all(app.join("src")).unwrap();
        git_no_repo(["init", path_arg(&repo).unwrap()]).unwrap();
        git_worktree(&repo, ["config", "user.email", "test@example.com"]);
        git_worktree(&repo, ["config", "user.name", "Zutai Test"]);
        fs::write(
            repo.join("packages/math/zutai.zti"),
            manifest(
                "math",
                "[{ name = \"answer\"; path = \"src/answer.zt\"; };]",
                "[{ alias = \"util\"; source = { kind = #path; path = \"../util\"; }; };]",
            ),
        )
        .unwrap();
        fs::write(
            repo.join("packages/math/src/answer.zt"),
            "{ value = 41; }\n",
        )
        .unwrap();
        fs::write(
            repo.join("packages/util/zutai.zti"),
            manifest(
                "util",
                "[{ name = \"one\"; path = \"src/one.zt\"; };]",
                "[]",
            ),
        )
        .unwrap();
        fs::write(repo.join("packages/util/src/one.zt"), "1\n").unwrap();
        git_worktree(&repo, ["add", "."]);
        git_worktree(&repo, ["commit", "-m", "first"]);
        git_worktree(&repo, ["tag", "-a", "v1", "-m", "v1"]);
        let first = git_worktree_text(&repo, ["rev-parse", "HEAD"]);
        fs::write(
            repo.join("packages/math/src/answer.zt"),
            "{ value = 42; }\n",
        )
        .unwrap();
        git_worktree(&repo, ["add", "."]);
        git_worktree(&repo, ["commit", "-m", "second"]);
        let second = git_worktree_text(&repo, ["rev-parse", "HEAD"]);
        git_worktree(&repo, ["branch", "--force", "moving", &first]);

        let url = "https://fixtures.invalid/packages.git";
        fs::write(
            app.join(MANIFEST_NAME),
            manifest(
                "app",
                "[]",
                &format!(
                    "[{{ alias = \"old\"; source = {{ kind = #git; url = \"{url}\"; rev = \"refs/tags/v1\"; subdir = \"packages/math\"; }}; }}; {{ alias = \"moving\"; source = {{ kind = #git; url = \"{url}\"; rev = \"refs/heads/moving\"; subdir = \"packages/math\"; }}; }}; {{ alias = \"new\"; source = {{ kind = #git; url = \"{url}\"; rev = \"{second}\"; subdir = \"packages/math\"; }}; }};]"
                ),
            ),
        )
        .unwrap();
        fs::write(app.join("src/main.zt"), "1\n").unwrap();

        let overrides = [(url.to_owned(), repo.clone())];
        let lock = acquire(&app, &cache, false, &overrides, Operation::Sync).unwrap();
        let root = &lock.packages[&lock.root];
        assert_ne!(root.dependencies["old"], root.dependencies["new"]);
        assert_eq!(lock.packages.len(), 5);
        let old = &lock.packages[&root.dependencies["old"]];
        let LockedSource::Git {
            commit, tree_hash, ..
        } = &old.source
        else {
            panic!("old dependency was not Git")
        };
        assert_eq!(commit, &first);
        let old_source = old.source.clone();
        let old_tree_hash = tree_hash.clone();
        let rendered = fs::read_to_string(app.join(LOCK_NAME)).unwrap();
        let cold_cache = temp.0.join("cold-cache");
        acquire(&app, &cold_cache, false, &overrides, Operation::Fetch).unwrap();
        let old_snapshot = snapshot_path(&cold_cache, &old_tree_hash);
        assert!(old_snapshot.is_dir());
        assert_eq!(hash_tree(&old_snapshot).unwrap(), old_tree_hash);

        acquire(&app, &cache, true, &[], Operation::Sync).unwrap();
        assert_eq!(fs::read_to_string(app.join(LOCK_NAME)).unwrap(), rendered);

        git_worktree(&repo, ["branch", "--force", "moving", &second]);
        let aliases = ["moving".to_owned()];
        let updated =
            acquire(&app, &cache, false, &overrides, Operation::Update(&aliases)).unwrap();
        let updated_root = &updated.packages[&updated.root];
        assert_eq!(
            updated.packages[&updated_root.dependencies["old"]].source,
            old_source
        );
        let LockedSource::Git { commit, .. } =
            &updated.packages[&updated_root.dependencies["moving"]].source
        else {
            panic!("moving dependency was not Git")
        };
        assert_eq!(commit, &second);

        let snapshot = snapshot_path(&cache, &old_tree_hash);
        make_writable(&snapshot);
        fs::write(snapshot.join("packages/math/src/answer.zt"), "tampered\n").unwrap();
        let error = acquire(&app, &cache, true, &[], Operation::Fetch).unwrap_err();
        assert!(error.message.contains("modified"), "{}", error.message);
    }

    #[test]
    fn prepared_graph_reports_missing_snapshot() {
        let temp = TempDir::new("package-missing-cache");
        let app = temp.0.join("app");
        let cache = temp.0.join("cache");
        fs::create_dir_all(&app).unwrap();
        fs::write(
            app.join(MANIFEST_NAME),
            manifest(
                "app",
                "[]",
                "[{ alias = \"dep\"; source = { kind = #git; url = \"https://fixtures.invalid/missing.git\"; rev = \"refs/heads/main\"; }; };]",
            ),
        )
        .unwrap();
        let source = LockedSource::Git {
            url: "https://fixtures.invalid/missing.git".to_owned(),
            resolved_url: "https://fixtures.invalid/missing.git".to_owned(),
            requested_rev: "refs/heads/main".to_owned(),
            commit: "a".repeat(40),
            object_format: ObjectFormat::Sha1,
            subdir: ".".to_owned(),
            tree_hash: format!("sha256:{}", "b".repeat(64)),
        };
        let dep_id = package_id(&source);
        let root_source = LockedSource::Path {
            path: ".".to_owned(),
        };
        let root_id = package_id(&root_source);
        let manifest_source = fs::read_to_string(app.join(MANIFEST_NAME)).unwrap();
        let lock = Lockfile {
            generated_by: format!("zutai-cli {COMPILER_VERSION}"),
            root: root_id.clone(),
            packages: BTreeMap::from([
                (
                    root_id.clone(),
                    LockedPackage {
                        id: root_id,
                        name: "app".to_owned(),
                        source: root_source,
                        manifest_hash: sha256_digest(manifest_source.as_bytes()),
                        dependencies: BTreeMap::from([("dep".to_owned(), dep_id.clone())]),
                    },
                ),
                (
                    dep_id.clone(),
                    LockedPackage {
                        id: dep_id,
                        name: "dep".to_owned(),
                        source,
                        manifest_hash: format!("sha256:{}", "c".repeat(64)),
                        dependencies: BTreeMap::new(),
                    },
                ),
            ]),
        };
        fs::write(app.join(LOCK_NAME), render_lockfile(&lock)).unwrap();
        let error = prepare_graph(&app, Some(&cache)).unwrap_err();
        assert!(error.message.contains("snapshot"), "{}", error.message);
        assert!(error.message.contains("package fetch"), "{}", error.message);
    }

    #[test]
    fn prepared_graph_rejects_stale_dependency_source() {
        let temp = TempDir::new("package-stale-source");
        let app = temp.0.join("app");
        let old = temp.0.join("old");
        let cache = temp.0.join("cache");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&old).unwrap();
        fs::write(
            app.join(MANIFEST_NAME),
            manifest(
                "app",
                "[]",
                "[{ alias = \"dep\"; source = { kind = #path; path = \"../new\"; }; };]",
            ),
        )
        .unwrap();
        fs::write(old.join(MANIFEST_NAME), manifest("dep", "[]", "[]")).unwrap();
        let root_source = LockedSource::Path {
            path: ".".to_owned(),
        };
        let root_id = package_id(&root_source);
        let target_source = LockedSource::Path {
            path: "../old".to_owned(),
        };
        let target_id = package_id(&target_source);
        let root_manifest = fs::read_to_string(app.join(MANIFEST_NAME)).unwrap();
        let target_manifest = fs::read_to_string(old.join(MANIFEST_NAME)).unwrap();
        let lock = Lockfile {
            generated_by: format!("zutai-cli {COMPILER_VERSION}"),
            root: root_id.clone(),
            packages: BTreeMap::from([
                (
                    root_id.clone(),
                    LockedPackage {
                        id: root_id,
                        name: "app".to_owned(),
                        source: root_source,
                        manifest_hash: sha256_digest(root_manifest.as_bytes()),
                        dependencies: BTreeMap::from([("dep".to_owned(), target_id.clone())]),
                    },
                ),
                (
                    target_id.clone(),
                    LockedPackage {
                        id: target_id,
                        name: "dep".to_owned(),
                        source: target_source,
                        manifest_hash: sha256_digest(target_manifest.as_bytes()),
                        dependencies: BTreeMap::new(),
                    },
                ),
            ]),
        };
        fs::write(app.join(LOCK_NAME), render_lockfile(&lock)).unwrap();
        let error = prepare_graph(&app, Some(&cache)).unwrap_err();
        assert!(
            error.message.contains("dependency edge") && error.message.contains("is stale"),
            "{}",
            error.message
        );
    }

    fn acquire<'a>(
        root: &'a Path,
        cache: &'a Path,
        offline: bool,
        overrides: &'a [(String, PathBuf)],
        operation: Operation<'a>,
    ) -> Result<Lockfile, PackageError> {
        run(AcquireOptions {
            root,
            cache_dir: Some(cache),
            offline,
            transport_overrides: overrides,
            operation,
        })
    }

    fn manifest(name: &str, modules: &str, dependencies: &str) -> String {
        format!(
            "{{ formatVersion = 2; name = \"{name}\"; compilerCompatibility = \">=0.1.0, <0.2.0\"; modules = {modules}; dependencies = {dependencies}; }}"
        )
    }

    fn git_worktree<const N: usize>(repo: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_worktree_text<const N: usize>(repo: &Path, args: [&str; N]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[allow(clippy::permissions_set_readonly_false)]
    fn make_writable(path: &Path) {
        if !path.exists() {
            return;
        }
        if let Ok(metadata) = fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
        if path.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                make_writable(&entry.path());
            }
        }
    }
}
