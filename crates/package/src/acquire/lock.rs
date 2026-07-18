use super::*;

pub(super) fn validate_local_locked_manifests(
    root: &Path,
    lock: &Lockfile,
) -> Result<(), PackageError> {
    validate_lockfile(lock)?;
    for package in lock.packages.values() {
        if matches!(package.source, LockedSource::Path { .. }) {
            validate_locked_manifest(root, root, lock, package)?;
        }
    }
    Ok(())
}

pub(super) fn validate_locked_manifests(
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

pub(super) fn validate_locked_manifest(
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

pub(super) fn stale_edges_error(root: &Path, manifest_path: &Path) -> PackageError {
    PackageError::new(format!(
        "package lock dependency edges are stale for {}; run `zutai-cli package sync {}`",
        manifest_path.display(),
        root.display()
    ))
}

pub(super) fn locked_dependency_matches(
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
pub(super) fn equivalent_git_dependency(
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

pub(super) fn git_selector_matches(declared: &str, locked: &str, target: &LockedPackage) -> bool {
    if declared == locked {
        return true;
    }
    matches!(&target.source, LockedSource::Git { commit, .. } if declared == commit || locked == commit)
}

pub(super) fn normalize_local_identity(
    owner: &str,
    dependency: &str,
) -> Result<String, PackageError> {
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

pub(super) fn fetch_locked_graph(
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

pub(super) fn install_graph_snapshots(cache: &Path, lock: &Lockfile) -> Result<(), PackageError> {
    for package in lock.packages.values() {
        if matches!(package.source, LockedSource::Git { .. }) {
            ensure_snapshot(cache, &package.source)?;
        }
    }
    Ok(())
}

pub(super) fn package_root(
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

pub(super) fn resolve_revision(
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

pub(super) fn transport_url(url: &str, overrides: &[(String, PathBuf)]) -> String {
    overrides
        .iter()
        .find(|(candidate, _)| candidate == url)
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| url.to_owned())
}
