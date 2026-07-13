//! Filesystem standard-library loader for Zutai general mode.
//!
//! The compiler never embeds standard-library source. A validated manifest and
//! its `.zt` files are loaded from an explicitly selected stdlib root.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const MANIFEST_FORMAT_VERSION: u32 = 1;
pub const COMPILER_COMPATIBILITY: &str = env!("CARGO_PKG_VERSION");
pub const STDLIB_ROOT_ENV: &str = "ZUTAI_STDLIB_ROOT";
pub const MANIFEST_NAME: &str = "zutai.zti";

static PROCESS_STDLIB_ROOT: OnceLock<PathBuf> = OnceLock::new();
static CONFIGURED_STDLIB: OnceLock<Result<StdlibSources, String>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibVisibility {
    Ambient,
    Explicit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StdlibManifest {
    format_version: u32,
    compiler_compatibility: String,
    modules: Vec<ManifestModule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestModule {
    name: String,
    path: String,
    visibility: StdlibVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdlibModule {
    name: String,
    source: String,
    visibility: StdlibVisibility,
}

impl StdlibModule {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn visibility(&self) -> StdlibVisibility {
        self.visibility
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdlibSources {
    root: Option<PathBuf>,
    compiler_compatibility: String,
    modules: BTreeMap<String, StdlibModule>,
}

impl StdlibSources {
    pub fn load(root: impl AsRef<Path>) -> Result<Self, StdlibError> {
        let root = root.as_ref();
        let manifest_path = root.join(MANIFEST_NAME);
        let bytes = fs::read(&manifest_path)
            .map_err(|source| StdlibError::read(manifest_path.clone(), source))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| StdlibError::invalid(manifest_path.clone(), "manifest is not UTF-8"))?;
        let manifest = parse_manifest(&manifest_path, &text)?;
        Self::load_manifest(root, manifest)
    }

    fn load_manifest(root: &Path, manifest: StdlibManifest) -> Result<Self, StdlibError> {
        if manifest.format_version != MANIFEST_FORMAT_VERSION {
            return Err(StdlibError::invalid(
                root.join(MANIFEST_NAME),
                format!(
                    "unsupported format version {}; expected {MANIFEST_FORMAT_VERSION}",
                    manifest.format_version
                ),
            ));
        }
        if manifest.compiler_compatibility != COMPILER_COMPATIBILITY {
            return Err(StdlibError::invalid(
                root.join(MANIFEST_NAME),
                format!(
                    "stdlib compatibility {:?} does not match compiler {:?}",
                    manifest.compiler_compatibility, COMPILER_COMPATIBILITY
                ),
            ));
        }

        let canonical_root = fs::canonicalize(root)
            .map_err(|source| StdlibError::read(root.to_path_buf(), source))?;
        let mut modules = BTreeMap::new();
        for entry in manifest.modules {
            if !valid_module_name(&entry.name) {
                return Err(StdlibError::invalid(
                    root.join(MANIFEST_NAME),
                    format!("invalid module name {:?}", entry.name),
                ));
            }
            let relative = Path::new(&entry.path);
            if !safe_relative_zt_path(relative) {
                return Err(StdlibError::invalid(
                    root.join(MANIFEST_NAME),
                    format!("unsafe module path {:?}", entry.path),
                ));
            }
            if modules.contains_key(&entry.name) {
                return Err(StdlibError::invalid(
                    root.join(MANIFEST_NAME),
                    format!("duplicate module {:?}", entry.name),
                ));
            }
            let path = root.join(relative);
            let canonical = fs::canonicalize(&path)
                .map_err(|source| StdlibError::read(path.clone(), source))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(StdlibError::invalid(path, "module escapes the stdlib root"));
            }
            let bytes = fs::read(&canonical)
                .map_err(|source| StdlibError::read(canonical.clone(), source))?;
            let source = String::from_utf8(bytes)
                .map_err(|_| StdlibError::invalid(canonical, "module source is not UTF-8"))?;
            modules.insert(
                entry.name.clone(),
                StdlibModule {
                    name: entry.name,
                    source,
                    visibility: entry.visibility,
                },
            );
        }

        for required in ["stream", "prelude"] {
            let Some(module) = modules.get(required) else {
                return Err(StdlibError::invalid(
                    root.join(MANIFEST_NAME),
                    format!("missing required ambient module {required:?}"),
                ));
            };
            if module.visibility != StdlibVisibility::Ambient {
                return Err(StdlibError::invalid(
                    root.join(MANIFEST_NAME),
                    format!("required module {required:?} must be ambient"),
                ));
            }
        }
        let unexpected_ambient: BTreeSet<_> = modules
            .values()
            .filter(|module| module.visibility == StdlibVisibility::Ambient)
            .map(|module| module.name.as_str())
            .filter(|name| !matches!(*name, "stream" | "prelude"))
            .collect();
        if !unexpected_ambient.is_empty() {
            return Err(StdlibError::invalid(
                root.join(MANIFEST_NAME),
                format!("unexpected ambient modules: {unexpected_ambient:?}"),
            ));
        }

        Ok(Self {
            root: Some(canonical_root),
            compiler_compatibility: manifest.compiler_compatibility,
            modules,
        })
    }

    pub fn from_memory(
        compiler_compatibility: impl Into<String>,
        sources: BTreeMap<String, String>,
    ) -> Result<Self, StdlibError> {
        let compiler_compatibility = compiler_compatibility.into();
        if compiler_compatibility != COMPILER_COMPATIBILITY {
            return Err(StdlibError::invalid(
                PathBuf::from("<bundle>/stdlib"),
                format!(
                    "stdlib compatibility {:?} does not match compiler {:?}",
                    compiler_compatibility, COMPILER_COMPATIBILITY
                ),
            ));
        }
        let mut modules = BTreeMap::new();
        for (name, source) in sources {
            if !valid_module_name(&name) {
                return Err(StdlibError::invalid(
                    PathBuf::from("<bundle>/stdlib"),
                    format!("invalid module name {name:?}"),
                ));
            }
            let visibility = if matches!(name.as_str(), "stream" | "prelude") {
                StdlibVisibility::Ambient
            } else {
                StdlibVisibility::Explicit
            };
            modules.insert(
                name.clone(),
                StdlibModule {
                    name,
                    source,
                    visibility,
                },
            );
        }
        for required in ["stream", "prelude"] {
            if !modules.contains_key(required) {
                return Err(StdlibError::invalid(
                    PathBuf::from("<bundle>/stdlib"),
                    format!("missing required ambient module {required:?}"),
                ));
            }
        }
        Ok(Self {
            root: None,
            compiler_compatibility,
            modules,
        })
    }

    pub fn load_configured(explicit: Option<&Path>) -> Result<Self, StdlibError> {
        if let Some(root) = explicit {
            return Self::load(root);
        }
        match CONFIGURED_STDLIB.get_or_init(|| {
            configured_root(None)
                .and_then(Self::load)
                .map_err(|error| error.to_string())
        }) {
            Ok(stdlib) => Ok(stdlib.clone()),
            Err(detail) => Err(StdlibError::Resolve {
                detail: detail.clone(),
            }),
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn compiler_compatibility(&self) -> &str {
        &self.compiler_compatibility
    }

    pub fn modules(&self) -> impl Iterator<Item = &StdlibModule> {
        self.modules.values()
    }

    pub fn module(&self, name: &str) -> Option<&StdlibModule> {
        self.modules.get(name)
    }

    pub fn source(&self, name: &str) -> Option<&str> {
        self.module(name).map(StdlibModule::source)
    }

    pub fn source_map(&self) -> BTreeMap<String, String> {
        self.modules
            .iter()
            .map(|(name, module)| (name.clone(), module.source.clone()))
            .collect()
    }
}

fn parse_manifest(path: &Path, source: &str) -> Result<StdlibManifest, StdlibError> {
    let block = zutai_im::parse_syntax(source)
        .map_err(|error| StdlibError::invalid(path.to_path_buf(), error.to_string()))?;
    let mut seen = BTreeSet::new();
    for pair in block.iter() {
        if !seen.insert(pair.field_name.as_str()) {
            return Err(StdlibError::invalid(
                path.to_path_buf(),
                format!("duplicate field {:?}", pair.field_name),
            ));
        }
    }
    let format_version = manifest_integer(path, &block, "formatVersion")?;
    let format_version = u32::try_from(format_version)
        .map_err(|_| StdlibError::invalid(path.to_path_buf(), "formatVersion must fit in u32"))?;
    let compiler_compatibility = manifest_string(path, &block, "compilerCompatibility")?.to_owned();
    let modules_value = manifest_value(path, &block, "modules")?;
    let zutai_im::Value::Array(entries) = modules_value else {
        return Err(StdlibError::invalid(
            path.to_path_buf(),
            "modules must be an array",
        ));
    };
    let mut modules = Vec::new();
    for entry in entries {
        let zutai_im::Value::Block(entry) = entry else {
            return Err(StdlibError::invalid(
                path.to_path_buf(),
                "module entries must be blocks",
            ));
        };
        let name = manifest_string(path, entry, "name")?.to_owned();
        let module_path = manifest_string(path, entry, "path")?.to_owned();
        let visibility = match manifest_value(path, entry, "visibility")? {
            zutai_im::Value::Atom(value) if value == "ambient" => StdlibVisibility::Ambient,
            zutai_im::Value::Atom(value) if value == "explicit" => StdlibVisibility::Explicit,
            _ => {
                return Err(StdlibError::invalid(
                    path.to_path_buf(),
                    "module visibility must be #ambient or #explicit",
                ));
            }
        };
        modules.push(ManifestModule {
            name,
            path: module_path,
            visibility,
        });
    }
    Ok(StdlibManifest {
        format_version,
        compiler_compatibility,
        modules,
    })
}

fn manifest_value<'a>(
    path: &Path,
    block: &'a zutai_im::Block,
    field: &str,
) -> Result<&'a zutai_im::Value, StdlibError> {
    block
        .iter()
        .find(|pair| pair.field_name == field)
        .map(|pair| &pair.value)
        .ok_or_else(|| StdlibError::invalid(path.to_path_buf(), format!("missing field {field:?}")))
}

fn manifest_string<'a>(
    path: &Path,
    block: &'a zutai_im::Block,
    field: &str,
) -> Result<&'a str, StdlibError> {
    match manifest_value(path, block, field)? {
        zutai_im::Value::String(value) => Ok(value),
        _ => Err(StdlibError::invalid(
            path.to_path_buf(),
            format!("field {field:?} must be a string"),
        )),
    }
}

fn manifest_integer(path: &Path, block: &zutai_im::Block, field: &str) -> Result<i64, StdlibError> {
    match manifest_value(path, block, field)? {
        zutai_im::Value::Integer(value) => Ok(*value),
        _ => Err(StdlibError::invalid(
            path.to_path_buf(),
            format!("field {field:?} must be an integer"),
        )),
    }
}

pub fn configured_root(explicit: Option<&Path>) -> Result<PathBuf, StdlibError> {
    let executable = env::current_exe().map_err(|source| StdlibError::Resolve {
        detail: format!("could not locate current executable: {source}"),
    })?;
    select_root(
        explicit,
        PROCESS_STDLIB_ROOT.get().map(PathBuf::as_path),
        env::var_os(STDLIB_ROOT_ENV).as_deref().map(Path::new),
        &executable,
    )
}

fn select_root(
    explicit: Option<&Path>,
    process: Option<&Path>,
    environment: Option<&Path>,
    executable: &Path,
) -> Result<PathBuf, StdlibError> {
    if let Some(root) = explicit.or(process).or(environment) {
        return Ok(root.to_path_buf());
    }
    let Some(bin_dir) = executable.parent() else {
        return Err(StdlibError::Resolve {
            detail: format!("executable path {} has no parent", executable.display()),
        });
    };
    Ok(bin_dir
        .join("..")
        .join("share")
        .join("zutai")
        .join("stdlib"))
}

/// Select a process-wide stdlib root before analysis starts.
///
/// CLI frontends use this for `--stdlib-root`; repeated selection is accepted
/// only when it names the same path.
pub fn set_process_root(root: PathBuf) -> Result<(), StdlibError> {
    if CONFIGURED_STDLIB.get().is_some() {
        return Err(StdlibError::Resolve {
            detail: "stdlib sources have already been loaded for this process".to_owned(),
        });
    }
    if let Some(existing) = PROCESS_STDLIB_ROOT.get() {
        if existing == &root {
            return Ok(());
        }
        return Err(StdlibError::Resolve {
            detail: format!(
                "stdlib root is already configured as {}; cannot replace it with {}",
                existing.display(),
                root.display()
            ),
        });
    }
    PROCESS_STDLIB_ROOT
        .set(root)
        .map_err(|root| StdlibError::Resolve {
            detail: format!("could not configure stdlib root {}", root.display()),
        })
}

fn valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn safe_relative_zt_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.extension().is_some_and(|extension| extension == "zt")
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[derive(Debug)]
pub enum StdlibError {
    Resolve {
        detail: String,
    },
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Invalid {
        path: PathBuf,
        detail: String,
    },
}

impl StdlibError {
    fn read(path: PathBuf, source: std::io::Error) -> Self {
        Self::Read { path, source }
    }

    fn invalid(path: PathBuf, detail: impl Into<String>) -> Self {
        Self::Invalid {
            path,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StdlibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve { detail } => write!(f, "could not resolve Zutai stdlib root: {detail}"),
            Self::Read { path, source } => write!(
                f,
                "could not read Zutai stdlib at {}: {source}; pass --stdlib-root or set {STDLIB_ROOT_ENV}",
                path.display()
            ),
            Self::Invalid { path, detail } => write!(
                f,
                "invalid Zutai stdlib at {}: {detail}; pass --stdlib-root or set {STDLIB_ROOT_ENV}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StdlibError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Resolve { .. } | Self::Invalid { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("ZUTAI_STDLIB_ROOT"))
    }

    fn manifest(modules: Vec<ManifestModule>) -> StdlibManifest {
        StdlibManifest {
            format_version: MANIFEST_FORMAT_VERSION,
            compiler_compatibility: COMPILER_COMPATIBILITY.to_owned(),
            modules,
        }
    }

    fn module(name: &str, visibility: StdlibVisibility) -> ManifestModule {
        ManifestModule {
            name: name.to_owned(),
            path: format!("modules/{name}.zt"),
            visibility,
        }
    }

    fn temp_root() -> TempRoot {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("zutai-stdlib-test-{}-{id}", std::process::id()));
        fs::create_dir_all(path.join("modules")).unwrap();
        TempRoot(path)
    }

    #[test]
    fn loads_checked_in_manifest_and_sources() {
        let stdlib = StdlibSources::load(fixture_root()).unwrap();
        assert!(stdlib.source("stream").is_some());
        assert!(stdlib.source("browser").is_some());
        assert_eq!(
            stdlib.module("prelude").unwrap().visibility(),
            StdlibVisibility::Ambient
        );
    }

    #[test]
    fn configured_root_prefers_explicit_path() {
        let explicit = Path::new("/tmp/explicit-zutai-stdlib");
        assert_eq!(configured_root(Some(explicit)).unwrap(), explicit);
    }

    #[test]
    fn root_precedence_is_explicit_process_environment_install() {
        let executable = Path::new("/opt/zutai/bin/zutai-cli");
        assert_eq!(
            select_root(
                Some(Path::new("explicit")),
                Some(Path::new("process")),
                Some(Path::new("environment")),
                executable,
            )
            .unwrap(),
            Path::new("explicit")
        );
        assert_eq!(
            select_root(
                None,
                Some(Path::new("process")),
                Some(Path::new("environment")),
                executable,
            )
            .unwrap(),
            Path::new("process")
        );
        assert_eq!(
            select_root(None, None, Some(Path::new("environment")), executable).unwrap(),
            Path::new("environment")
        );
        assert_eq!(
            select_root(None, None, None, executable).unwrap(),
            Path::new("/opt/zutai/bin/../share/zutai/stdlib")
        );
    }

    #[test]
    fn memory_sources_require_both_ambient_modules() {
        let error = StdlibSources::from_memory(
            COMPILER_COMPATIBILITY,
            BTreeMap::from([("stream".to_owned(), "1".to_owned())]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("prelude"));
    }

    #[test]
    fn rejects_unsafe_paths_before_reading_them() {
        let root = fixture_root();
        let manifest = manifest(vec![ManifestModule {
            name: "stream".to_owned(),
            path: "../stream.zt".to_owned(),
            visibility: StdlibVisibility::Ambient,
        }]);
        let error = StdlibSources::load_manifest(&root, manifest).unwrap_err();
        assert!(error.to_string().contains("unsafe module path"));
    }

    #[test]
    fn rejects_incompatible_manifest() {
        let temp = temp_root();
        let mut manifest = manifest(Vec::new());
        manifest.compiler_compatibility = "different".to_owned();
        let error = StdlibSources::load_manifest(temp.path(), manifest).unwrap_err();
        assert!(error.to_string().contains("does not match compiler"));
    }

    #[test]
    fn rejects_duplicate_module_names() {
        let temp = temp_root();
        fs::write(temp.path().join("modules/stream.zt"), "1").unwrap();
        let duplicate = module("stream", StdlibVisibility::Ambient);
        let error =
            StdlibSources::load_manifest(temp.path(), manifest(vec![duplicate.clone(), duplicate]))
                .unwrap_err();
        assert!(error.to_string().contains("duplicate module"));
    }

    #[test]
    fn rejects_missing_module_file() {
        let temp = temp_root();
        let error = StdlibSources::load_manifest(
            temp.path(),
            manifest(vec![module("stream", StdlibVisibility::Ambient)]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("stream.zt"));
    }

    #[test]
    fn rejects_non_utf8_module_source() {
        let temp = temp_root();
        fs::write(temp.path().join("modules/stream.zt"), [0xff]).unwrap();
        let error = StdlibSources::load_manifest(
            temp.path(),
            manifest(vec![module("stream", StdlibVisibility::Ambient)]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not UTF-8"));
    }

    #[test]
    fn rejects_missing_required_ambient_module() {
        let temp = temp_root();
        fs::write(temp.path().join("modules/prelude.zt"), "1").unwrap();
        let error = StdlibSources::load_manifest(
            temp.path(),
            manifest(vec![module("prelude", StdlibVisibility::Ambient)]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("stream"));
    }
}
