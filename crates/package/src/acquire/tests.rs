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
        let path = std::env::temp_dir().join(format!("zutai-{name}-{}-{id}", std::process::id()));
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
    let updated = acquire(&app, &cache, false, &overrides, Operation::Update(&aliases)).unwrap();
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
