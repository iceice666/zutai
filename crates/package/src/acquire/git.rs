use super::*;

pub(super) fn ensure_repository(
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

pub(super) fn ensure_bare_repository(
    repo: &Path,
    url: &str,
    offline: bool,
) -> Result<(), PackageError> {
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

pub(super) fn ensure_snapshot(cache: &Path, source: &LockedSource) -> Result<(), PackageError> {
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

pub(super) fn hash_repository_tree(
    cache: &Path,
    url: &str,
    commit: &str,
) -> Result<String, PackageError> {
    hash_repository_listing(&repo_path(cache, url), commit)
}

pub(super) fn materialize_tree(
    repo: &Path,
    commit: &str,
    output: &Path,
) -> Result<(), PackageError> {
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
pub(super) struct TreeEntry {
    mode: String,
    object: String,
    path: String,
    size: u64,
}

pub(super) fn parse_tree_entries(listing: &[u8]) -> Result<Vec<TreeEntry>, PackageError> {
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

pub(super) fn write_snapshot_modes(root: &Path, entries: &[TreeEntry]) -> Result<(), PackageError> {
    let mut content = String::new();
    for entry in entries {
        content.push_str(&entry.mode);
        content.push(' ');
        content.push_str(&entry.path);
        content.push('\n');
    }
    fs::write(root.join(SNAPSHOT_MODE_NAME), content).map_err(io_error("write snapshot modes"))
}

pub(super) fn populate_tree_sizes(
    repo: &Path,
    entries: &mut [TreeEntry],
) -> Result<(), PackageError> {
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

pub(super) fn validate_tree_path(path: &str) -> Result<(), PackageError> {
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

pub(super) fn is_windows_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && matches!(&stem[..3], "com" | "lpt")
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

pub(super) fn hash_repository_listing(repo: &Path, commit: &str) -> Result<String, PackageError> {
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

pub(super) fn hash_tree(root: &Path) -> Result<String, PackageError> {
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

pub(super) fn read_snapshot_modes(root: &Path) -> Result<BTreeMap<String, String>, PackageError> {
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

pub(super) fn collect_files(
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

pub(super) fn verify_snapshot(path: &Path, expected: &str) -> Result<(), PackageError> {
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

pub(super) fn source_tree_hash(source: &LockedSource) -> &str {
    match source {
        LockedSource::Git { tree_hash, .. } => tree_hash,
        LockedSource::Path { .. } => unreachable!("local sources have no tree hash"),
    }
}

pub(super) fn repo_path(cache: &Path, url: &str) -> PathBuf {
    cache
        .join("repos")
        .join(sha256_digest(url.as_bytes()).trim_start_matches("sha256:"))
}

pub(super) fn snapshot_path(cache: &Path, tree_hash: &str) -> PathBuf {
    cache
        .join("snapshots")
        .join(tree_hash.trim_start_matches("sha256:"))
}

pub(super) fn temp_sibling(path: &Path, kind: &str) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    path.parent()
        .expect("cache paths have parents")
        .join(format!(".zutai-{kind}-{}-{id}", std::process::id()))
}

pub(super) fn join_git_subdir(base: &str, relative: &str) -> Result<String, PackageError> {
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

pub(super) fn slash_path(path: &Path) -> Result<String, PackageError> {
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
pub(super) fn set_executable_mode(path: &Path, mode: &str) -> Result<(), PackageError> {
    use std::os::unix::fs::PermissionsExt as _;
    let permissions = fs::Permissions::from_mode(if mode == "100755" { 0o755 } else { 0o644 });
    fs::set_permissions(path, permissions).map_err(io_error("set snapshot executable mode"))
}

#[cfg(not(unix))]
pub(super) fn set_executable_mode(_path: &Path, _mode: &str) -> Result<(), PackageError> {
    Ok(())
}

pub(super) fn set_readonly(path: &Path) -> Result<(), PackageError> {
    let mut permissions = fs::metadata(path)
        .map_err(io_error("read snapshot permissions"))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(io_error("set snapshot read-only"))
}

pub(super) fn set_tree_readonly(root: &Path) -> Result<(), PackageError> {
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

pub(super) fn git<const N: usize>(repo: &Path, args: [&str; N]) -> Result<(), PackageError> {
    let output = git_output(Some(repo), args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(&output))
    }
}

pub(super) fn git_text<const N: usize>(
    repo: &Path,
    args: [&str; N],
) -> Result<String, PackageError> {
    let output = git_output(Some(repo), args)?;
    if !output.status.success() {
        return Err(git_failure(&output));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|_| PackageError::new("Git returned non-UTF-8 text"))
}

pub(super) fn git_bytes<const N: usize>(
    repo: &Path,
    args: [&str; N],
) -> Result<Vec<u8>, PackageError> {
    let output = git_output(Some(repo), args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(git_failure(&output))
    }
}

pub(super) fn git_status<const N: usize>(
    repo: &Path,
    args: [&str; N],
) -> Result<bool, PackageError> {
    Ok(git_output(Some(repo), args)?.status.success())
}

pub(super) fn git_no_repo<const N: usize>(args: [&str; N]) -> Result<(), PackageError> {
    let output = git_output(None, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(&output))
    }
}

pub(super) fn git_output<const N: usize>(
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

pub(super) fn null_device() -> &'static OsStr {
    if cfg!(windows) {
        OsStr::new("NUL")
    } else {
        OsStr::new("/dev/null")
    }
}

pub(super) fn git_failure(output: &Output) -> PackageError {
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

pub(super) fn path_arg(path: &Path) -> Result<&str, PackageError> {
    path.to_str()
        .ok_or_else(|| PackageError::new("package cache path is not UTF-8"))
}

pub(super) fn io_error(purpose: &'static str) -> impl Fn(std::io::Error) -> PackageError {
    move |error| PackageError::new(format!("cannot {purpose}: {error}"))
}
