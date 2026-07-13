use std::cell::RefCell;
use std::error::Error;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Response, Server, StatusCode};
use zutai_browser::{BrowserProgram, WebBundleV3, decode_program, prerender_document};
use zutai_eval::{EffectHandler, EvalError, TlcSession, Value};

use clap::Subcommand;

const MAX_PAGES_FILE_BYTES: u64 = 25 * 1024 * 1024;

/// Commands exposed by the dedicated `zutai-web` app and the compatibility
/// `zutai-cli web` entry point.
#[derive(Clone, Debug, Subcommand)]
pub enum WebCommand {
    /// Build a prerendered static site and its interpreter WebAssembly kernel
    Build {
        /// Browser program entry `.zt` file
        entry: PathBuf,
        /// Static output directory
        #[arg(short = 'o', long, default_value = "dist")]
        out_dir: PathBuf,
        /// Root used for portable source paths (defaults to the entry directory)
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Static assets copied verbatim (defaults to `<source-root>/public`)
        #[arg(long)]
        public_dir: Option<PathBuf>,
    },
    /// Build, watch, and serve with full-page reload on successful changes
    Serve {
        /// Browser program entry `.zt` file
        entry: PathBuf,
        /// Static output directory
        #[arg(short = 'o', long, default_value = "dist")]
        out_dir: PathBuf,
        /// Root used for portable source paths (defaults to the entry directory)
        #[arg(long)]
        source_root: Option<PathBuf>,
        /// Static assets copied verbatim (defaults to `<source-root>/public`)
        #[arg(long)]
        public_dir: Option<PathBuf>,
        /// Address for the development server
        #[arg(long, default_value = "127.0.0.1:8787")]
        addr: String,
        /// Serve the existing output directory without rebuilding first
        #[arg(long)]
        no_build: bool,
    },
}

impl WebCommand {
    pub fn run(self) -> Result<(), Box<dyn Error>> {
        match self {
            Self::Build {
                entry,
                out_dir,
                source_root,
                public_dir,
            } => run_web_build(WebBuildOptions {
                entry,
                out_dir,
                source_root,
                public_dir,
            }),
            Self::Serve {
                entry,
                out_dir,
                source_root,
                public_dir,
                addr,
                no_build,
            } => run_web_serve(
                WebBuildOptions {
                    entry,
                    out_dir,
                    source_root,
                    public_dir,
                },
                &addr,
                no_build,
            ),
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("web crate lives under crates/web")
        .to_path_buf()
}

fn cargo_target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        }
        None => root.join("target"),
    }
}

fn run_tool(command: &mut Command, tool: &str, purpose: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status().map_err(|err| {
        format!(
            "web build error: required tool `{tool}` failed to start for {purpose}: {err}; install it, set the corresponding ZUTAI_* environment variable, or run from the Zutai dev shell (`nix develop`)"
        )
    })?;
    if !status.success() {
        return Err(format!("web build error: `{tool}` failed while {purpose}").into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct WebBuildOptions {
    pub entry: PathBuf,
    pub out_dir: PathBuf,
    pub source_root: Option<PathBuf>,
    pub public_dir: Option<PathBuf>,
}

#[derive(Default)]
struct BuildEffects {
    focus: RefCell<Vec<String>>,
}

impl EffectHandler for BuildEffects {
    fn handle(&self, operation: &str, argument: Value) -> Result<Value, EvalError> {
        if operation != "browser.focus" {
            return Err(EvalError::EffectfulNotExecutable(format!(
                "host effect `{operation}` is unavailable in a browser program"
            )));
        }
        let Value::Text(id) = argument else {
            return Err(EvalError::TypeMismatch {
                expected: "Text",
                found: "non-Text browser.focus argument",
            });
        };
        self.focus.borrow_mut().push(id.to_string());
        Ok(Value::Tuple(Rc::from([])))
    }
}

struct BuiltSite {
    out_dir: PathBuf,
    hash: String,
}

pub fn run_web_build(options: WebBuildOptions) -> Result<(), Box<dyn Error>> {
    let built = build_site(&options)?;
    println!(
        "Zutai web build complete: {} (kernel {})",
        built.out_dir.display(),
        built.hash
    );
    Ok(())
}

fn build_site(options: &WebBuildOptions) -> Result<BuiltSite, Box<dyn Error>> {
    let entry = absolute_existing_file(&options.entry)?;
    let entry_parent = entry
        .parent()
        .ok_or("web entry must have a parent directory")?
        .to_path_buf();
    let source_root = match &options.source_root {
        Some(path) => absolute_existing_dir(path)?,
        None => entry_parent,
    };
    if !entry.starts_with(&source_root) {
        return Err(format!(
            "web entry {} is outside source root {}",
            entry.display(),
            source_root.display()
        )
        .into());
    }
    let public_dir = options
        .public_dir
        .as_ref()
        .map(|path| absolute_path(path))
        .unwrap_or_else(|| source_root.join("public"));
    let out_dir = absolute_path(&options.out_dir);
    guard_output_directory(&out_dir, &source_root)?;

    let recorded = zutai_semantic::analyze_path_recording_with_root(&entry, &source_root)?;
    let session = TlcSession::from_analysis(&recorded.analysis)?;
    let program = decode_program(&session, session.entry()?)?;
    let effects = BuildEffects::default();
    let model = program.initialize(&session, &effects)?;
    let document = program.render(&session, model)?;

    let bundle = WebBundleV3::new(
        recorded.entry,
        recorded.sources,
        recorded.stdlib_compiler_compatibility,
        recorded.stdlib_sources,
        recorded.packages,
    );
    // Re-run through the memory loader at build time. This proves the emitted
    // bundle is complete and path-portable before it reaches a browser.
    let bundled_stdlib = zutai_semantic::StdlibSources::from_memory(
        bundle.stdlib_compiler_compatibility.clone(),
        bundle.stdlib_sources.clone(),
    )?;
    let bundled_analysis = zutai_semantic::analyze_sources_with_stdlib_and_packages(
        &bundle.entry,
        &bundle.sources,
        zutai_semantic::AnalysisOptions::default(),
        &bundled_stdlib,
        bundle.packages.clone(),
    )?;
    let bundled_session = TlcSession::from_analysis(&bundled_analysis)?;
    let _: BrowserProgram = decode_program(&bundled_session, bundled_session.entry()?)?;

    let bundle_json = serde_json::to_string(&bundle)?;
    let kernel = build_kernel()?;
    let bootstrap = bootstrap_module();
    let hash = content_hash(&[
        kernel.js.as_slice(),
        kernel.wasm.as_slice(),
        bundle_json.as_bytes(),
        bootstrap.as_bytes(),
    ]);
    let asset_url = format!("/_zutai/{hash}");
    let prerendered = prerender_document(&document, &format!("{asset_url}/bootstrap.js"))?;

    let parent = out_dir
        .parent()
        .ok_or("web output must have a parent directory")?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".zutai-web-{}-{}", std::process::id(), hash));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    if public_dir.is_dir() {
        copy_public_tree(&public_dir, &staging, Path::new(""))?;
    }

    let assets = staging.join("_zutai").join(&hash);
    fs::create_dir_all(&assets)?;
    fs::write(staging.join("index.html"), prerendered.html)?;
    fs::write(assets.join("bootstrap.js"), bootstrap)?;
    fs::write(assets.join("zutai_browser.js"), &kernel.js)?;
    fs::write(assets.join("zutai_browser_bg.wasm"), &kernel.wasm)?;
    fs::write(assets.join("zutai.bundle.json"), bundle_json)?;
    validate_deploy_tree(&staging)?;

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::rename(&staging, &out_dir)?;
    Ok(BuiltSite { out_dir, hash })
}

struct KernelArtifacts {
    js: Vec<u8>,
    wasm: Vec<u8>,
}

fn build_kernel() -> Result<KernelArtifacts, Box<dyn Error>> {
    let root = workspace_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut cargo_command = Command::new(&cargo);
    cargo_command
        .current_dir(&root)
        .arg("build")
        .arg("-p")
        .arg("zutai-browser")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--profile")
        .arg("web");
    run_tool(
        &mut cargo_command,
        &cargo,
        "building the Zutai browser WebAssembly kernel",
    )?;

    let target = cargo_target_dir(&root);
    let input = target
        .join("wasm32-unknown-unknown")
        .join("web")
        .join("zutai_browser.wasm");
    let bindgen_dir = target.join("zutai-web").join("bindgen");
    if bindgen_dir.exists() {
        fs::remove_dir_all(&bindgen_dir)?;
    }
    fs::create_dir_all(&bindgen_dir)?;

    let wasm_bindgen =
        std::env::var("ZUTAI_WASM_BINDGEN").unwrap_or_else(|_| "wasm-bindgen".to_owned());
    let mut bindgen_command = Command::new(&wasm_bindgen);
    bindgen_command
        .arg("--target")
        .arg("web")
        .arg("--no-typescript")
        .arg("--out-name")
        .arg("zutai_browser")
        .arg("--out-dir")
        .arg(&bindgen_dir)
        .arg(&input);
    run_tool(
        &mut bindgen_command,
        &wasm_bindgen,
        "generating browser ES modules",
    )?;

    let wasm = bindgen_dir.join("zutai_browser_bg.wasm");
    let optimized = bindgen_dir.join("zutai_browser_bg.opt.wasm");
    let wasm_opt = std::env::var("ZUTAI_WASM_OPT").unwrap_or_else(|_| "wasm-opt".to_owned());
    let mut optimize_command = Command::new(&wasm_opt);
    optimize_command
        .arg("-Oz")
        .arg("--enable-bulk-memory")
        .arg("--enable-bulk-memory-opt")
        .arg("--enable-nontrapping-float-to-int")
        .arg(&wasm)
        .arg("-o")
        .arg(&optimized);
    run_tool(
        &mut optimize_command,
        &wasm_opt,
        "optimizing the browser WebAssembly kernel",
    )?;

    Ok(KernelArtifacts {
        js: fs::read(bindgen_dir.join("zutai_browser.js"))?,
        wasm: fs::read(optimized)?,
    })
}

fn bootstrap_module() -> &'static str {
    r#"import init, { start } from './zutai_browser.js';

try {
  const bundleUrl = new URL('./zutai.bundle.json', import.meta.url);
  const response = await fetch(bundleUrl, { cache: 'no-cache' });
  if (!response.ok) throw new Error(`bundle request failed: ${response.status}`);
  const bundle = await response.text();
  await init();
  start(bundle, false);
} catch (error) {
  console.error('Zutai browser kernel failed to start; keeping prerendered page', error);
}
"#
}

fn content_hash(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest
        .finalize()
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn copy_public_tree(source: &Path, out: &Path, relative: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(source.join(relative))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let next_relative = relative.join(entry.file_name());
        if is_reserved_public_path(&next_relative) {
            return Err(format!(
                "public asset `{}` collides with Zutai-generated output",
                next_relative.display()
            )
            .into());
        }
        if file_type.is_symlink() {
            return Err(format!(
                "public asset symlinks are not portable: {}",
                entry.path().display()
            )
            .into());
        }
        if file_type.is_dir() {
            fs::create_dir_all(out.join(&next_relative))?;
            copy_public_tree(source, out, &next_relative)?;
        } else if file_type.is_file() {
            if let Some(parent) = out.join(&next_relative).parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), out.join(&next_relative))?;
        }
    }
    Ok(())
}

fn is_reserved_public_path(path: &Path) -> bool {
    path == Path::new("index.html")
        || path
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == "_zutai")
}

fn validate_deploy_tree(root: &Path) -> Result<(), Box<dyn Error>> {
    fn walk(path: &Path) -> Result<(), Box<dyn Error>> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                walk(&entry.path())?;
            } else if metadata.len() > MAX_PAGES_FILE_BYTES {
                return Err(format!(
                    "{} is {:.1} MiB; Cloudflare Pages files must not exceed 25 MiB",
                    entry.path().display(),
                    metadata.len() as f64 / (1024.0 * 1024.0)
                )
                .into());
            }
        }
        Ok(())
    }
    walk(root)
}

fn guard_output_directory(out: &Path, source_root: &Path) -> Result<(), Box<dyn Error>> {
    if out == Path::new("/") || out == source_root || source_root.starts_with(out) {
        return Err(format!("refusing unsafe web output directory {}", out.display()).into());
    }
    if out == workspace_root() {
        return Err("refusing to replace the workspace root with web output".into());
    }
    Ok(())
}

fn absolute_existing_file(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = fs::canonicalize(path)?;
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()).into());
    }
    Ok(path)
}

fn absolute_existing_dir(path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = fs::canonicalize(path)?;
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()).into());
    }
    Ok(path)
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .expect("current directory is available")
            .join(path)
    }
}

#[derive(Default)]
struct DevStatus {
    revision: u64,
    error: Option<String>,
}

pub fn run_web_serve(
    options: WebBuildOptions,
    addr: &str,
    no_build: bool,
) -> Result<(), Box<dyn Error>> {
    let built = if no_build {
        let out_dir = absolute_path(&options.out_dir);
        if !out_dir.join("index.html").is_file() {
            return Err(format!(
                "cannot serve existing output: {} has no index.html",
                out_dir.display()
            )
            .into());
        }
        BuiltSite {
            out_dir,
            hash: "existing".into(),
        }
    } else {
        build_site(&options)?
    };
    let status = Arc::new(Mutex::new(DevStatus {
        revision: 1,
        error: None,
    }));
    if !no_build {
        spawn_watcher(options.clone(), Arc::clone(&status));
    }

    let server = Server::http(addr).map_err(|err| format!("cannot bind {addr}: {err}"))?;
    println!("Zutai web server: http://{addr}/");
    for request in server.incoming_requests() {
        serve_request(request, &built.out_dir, &status)?;
    }
    Ok(())
}

fn spawn_watcher(options: WebBuildOptions, status: Arc<Mutex<DevStatus>>) {
    thread::spawn(move || {
        let (send, receive) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |result| {
            let _ = send.send(result);
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                status.lock().unwrap().error = Some(format!("watcher failed: {err}"));
                return;
            }
        };
        let source_root = options.source_root.clone().unwrap_or_else(|| {
            options
                .entry
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf()
        });
        if let Err(err) = watcher.watch(&source_root, RecursiveMode::Recursive) {
            status.lock().unwrap().error = Some(format!("watcher failed: {err}"));
            return;
        }
        loop {
            match receive.recv() {
                Ok(Ok(_)) => {
                    thread::sleep(Duration::from_millis(150));
                    while receive.try_recv().is_ok() {}
                    match build_site(&options) {
                        Ok(_) => {
                            let mut status = status.lock().unwrap();
                            status.revision = status.revision.wrapping_add(1);
                            status.error = None;
                        }
                        Err(err) => status.lock().unwrap().error = Some(err.to_string()),
                    }
                }
                Ok(Err(err)) => status.lock().unwrap().error = Some(err.to_string()),
                Err(_) => return,
            }
        }
    });
}

fn serve_request(
    request: tiny_http::Request,
    out_dir: &Path,
    status: &Arc<Mutex<DevStatus>>,
) -> Result<(), Box<dyn Error>> {
    let path = request.url().split('?').next().unwrap_or("/");
    if path == "/__zutai/status" {
        let status = status.lock().unwrap();
        let body = serde_json::json!({
            "revision": status.revision,
            "error": status.error,
        })
        .to_string();
        return respond(
            request,
            body.into_bytes(),
            "application/json; charset=utf-8",
            StatusCode(200),
        );
    }

    let relative = if path == "/" {
        PathBuf::from("index.html")
    } else {
        safe_request_path(path)?
    };
    let file = out_dir.join(&relative);
    if !file.is_file() {
        return respond(
            request,
            b"not found\n".to_vec(),
            "text/plain; charset=utf-8",
            StatusCode(404),
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(&file)?.read_to_end(&mut bytes)?;
    if relative == Path::new("index.html") {
        let revision = status.lock().unwrap().revision;
        let html = String::from_utf8(bytes)?;
        bytes = inject_development_reload(&html, revision).into_bytes();
    }
    respond(request, bytes, content_type(&file), StatusCode(200))
}

fn respond(
    request: tiny_http::Request,
    bytes: Vec<u8>,
    content_type: &str,
    status: StatusCode,
) -> Result<(), Box<dyn Error>> {
    let content_type = Header::from_bytes("Content-Type", content_type)
        .map_err(|_| "invalid content type header")?;
    let cache =
        Header::from_bytes("Cache-Control", "no-store").map_err(|_| "invalid cache header")?;
    request.respond(
        Response::from_data(bytes)
            .with_status_code(status)
            .with_header(content_type)
            .with_header(cache),
    )?;
    Ok(())
}

fn safe_request_path(url_path: &str) -> Result<PathBuf, Box<dyn Error>> {
    let mut out = PathBuf::new();
    for component in Path::new(url_path.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => out.push(part),
            _ => return Err("invalid request path".into()),
        }
    }
    Ok(out)
}

fn inject_development_reload(html: &str, revision: u64) -> String {
    let script = format!(
        r#"<script data-zutai-development-reload>
let zutaiRevision = {revision};
setInterval(async () => {{
  try {{
    const response = await fetch('/__zutai/status', {{ cache: 'no-store' }});
    const status = await response.json();
    let overlay = document.getElementById('zutai-build-error');
    if (status.error) {{
      if (!overlay) {{
        overlay = document.createElement('pre');
        overlay.id = 'zutai-build-error';
        overlay.style.cssText = 'position:fixed;inset:auto 1rem 1rem;z-index:2147483647;padding:1rem;max-height:40vh;overflow:auto;background:#170b22;color:#ff8ea1;border:1px solid #ff5678;border-radius:.5rem;white-space:pre-wrap';
        document.body.append(overlay);
      }}
      overlay.textContent = status.error;
    }} else {{
      overlay?.remove();
      if (status.revision !== zutaiRevision) location.reload();
    }}
  }} catch (error) {{ console.debug('Zutai reload poll failed', error); }}
}}, 500);
</script>"#
    );
    html.replacen("</body>", &format!("{script}</body>"), 1)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "zutai-web-{label}-{}-{}",
                std::process::id(),
                NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn hash_is_stable_and_length_delimited() {
        assert_eq!(content_hash(&[b"ab", b"c"]), content_hash(&[b"ab", b"c"]));
        assert_ne!(content_hash(&[b"ab", b"c"]), content_hash(&[b"a", b"bc"]));
        assert_eq!(content_hash(&[b"value"]).len(), 24);
    }

    #[test]
    fn request_paths_cannot_escape_dist() {
        assert_eq!(
            safe_request_path("/assets/site.js").unwrap(),
            Path::new("assets/site.js")
        );
        assert_eq!(
            safe_request_path("/assets/./site.js").unwrap(),
            Path::new("assets/site.js")
        );
        for path in ["/../Cargo.toml", "/assets/../site.js"] {
            assert!(safe_request_path(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn public_generated_paths_are_reserved() {
        assert!(is_reserved_public_path(Path::new("index.html")));
        assert!(is_reserved_public_path(Path::new("_zutai/x.js")));
        assert!(!is_reserved_public_path(Path::new("_headers")));
    }

    #[test]
    fn build_effects_record_focus_and_reject_other_effects() {
        let effects = BuildEffects::default();
        let result = effects
            .handle("browser.focus", Value::Text(Rc::from("search")))
            .unwrap();
        assert!(matches!(result, Value::Tuple(values) if values.is_empty()));
        assert_eq!(&*effects.focus.borrow(), &["search"]);

        let wrong_effect = effects.handle("net.listen", Value::Int(80)).unwrap_err();
        assert!(
            wrong_effect
                .to_string()
                .contains("unavailable in a browser")
        );
        let wrong_argument = effects.handle("browser.focus", Value::Int(1)).unwrap_err();
        assert!(wrong_argument.to_string().contains("non-Text"));
    }

    #[test]
    fn public_tree_copy_preserves_nested_files_and_rejects_collisions() {
        let temp = TempDir::new("public-copy");
        let public = temp.path().join("public");
        let out = temp.path().join("out");
        fs::create_dir_all(public.join("assets")).unwrap();
        fs::write(public.join("assets/site.css"), "body {}").unwrap();
        fs::write(public.join("_headers"), "/*\n  X-Test: yes\n").unwrap();
        fs::create_dir_all(&out).unwrap();

        copy_public_tree(&public, &out, Path::new("")).unwrap();
        assert_eq!(
            fs::read_to_string(out.join("assets/site.css")).unwrap(),
            "body {}"
        );
        assert!(out.join("_headers").is_file());

        fs::write(public.join("index.html"), "collision").unwrap();
        let error = copy_public_tree(&public, &out, Path::new("")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("collides with Zutai-generated output")
        );
    }

    #[cfg(unix)]
    #[test]
    fn public_tree_copy_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("public-symlink");
        let public = temp.path().join("public");
        let out = temp.path().join("out");
        fs::create_dir_all(&public).unwrap();
        fs::create_dir_all(&out).unwrap();
        fs::write(temp.path().join("outside.txt"), "outside").unwrap();
        symlink(temp.path().join("outside.txt"), public.join("linked.txt")).unwrap();

        let error = copy_public_tree(&public, &out, Path::new("")).unwrap_err();
        assert!(error.to_string().contains("symlinks are not portable"));
    }

    #[test]
    fn deploy_tree_rejects_files_over_pages_limit() {
        let temp = TempDir::new("pages-limit");
        let small = temp.path().join("small.txt");
        fs::write(&small, "ok").unwrap();
        validate_deploy_tree(temp.path()).unwrap();

        let large = fs::File::create(temp.path().join("large.wasm")).unwrap();
        large.set_len(MAX_PAGES_FILE_BYTES + 1).unwrap();
        let error = validate_deploy_tree(temp.path()).unwrap_err();
        assert!(error.to_string().contains("must not exceed 25 MiB"));
    }

    #[test]
    fn output_guard_rejects_source_ancestors_but_allows_siblings() {
        let temp = TempDir::new("output-guard");
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();

        assert!(guard_output_directory(&source, &source).is_err());
        assert!(guard_output_directory(temp.path(), &source).is_err());
        assert!(guard_output_directory(&temp.path().join("dist"), &source).is_ok());
        assert!(guard_output_directory(&workspace_root(), &source).is_err());
    }

    #[test]
    fn existing_path_helpers_distinguish_files_and_directories() {
        let temp = TempDir::new("existing-paths");
        let file = temp.path().join("entry.zt");
        fs::write(&file, "1").unwrap();

        assert_eq!(
            absolute_existing_file(&file).unwrap(),
            fs::canonicalize(&file).unwrap()
        );
        assert!(absolute_existing_file(temp.path()).is_err());
        assert_eq!(
            absolute_existing_dir(temp.path()).unwrap(),
            fs::canonicalize(temp.path()).unwrap()
        );
        assert!(absolute_existing_dir(&file).is_err());
        assert!(absolute_existing_file(&temp.path().join("missing")).is_err());
    }

    #[test]
    fn no_build_serve_fails_before_binding_when_index_is_missing() {
        let temp = TempDir::new("serve-missing");
        let options = WebBuildOptions {
            entry: temp.path().join("main.zt"),
            out_dir: temp.path().join("dist"),
            source_root: None,
            public_dir: None,
        };

        let error = run_web_serve(options.clone(), "127.0.0.1:0", true).unwrap_err();
        assert!(error.to_string().contains("has no index.html"));

        let error = WebCommand::Serve {
            entry: options.entry,
            out_dir: options.out_dir,
            source_root: None,
            public_dir: None,
            addr: "127.0.0.1:0".into(),
            no_build: true,
        }
        .run()
        .unwrap_err();
        assert!(error.to_string().contains("has no index.html"));
    }

    #[test]
    fn development_reload_is_injected_only_before_a_body_close() {
        let html = inject_development_reload("<html><body>ready</body></html>", 7);
        assert!(html.contains("let zutaiRevision = 7"));
        assert!(html.contains("/__zutai/status"));
        assert!(html.contains("data-zutai-development-reload"));
        assert!(html.contains("</script></body>"));

        assert_eq!(
            inject_development_reload("<html>prerender fragment</html>", 7),
            "<html>prerender fragment</html>"
        );
    }

    #[test]
    fn content_types_cover_generated_and_public_assets() {
        let cases = [
            ("index.html", "text/html; charset=utf-8"),
            ("bootstrap.js", "text/javascript; charset=utf-8"),
            ("bundle.json", "application/json; charset=utf-8"),
            ("kernel.wasm", "application/wasm"),
            ("site.css", "text/css; charset=utf-8"),
            ("mark.svg", "image/svg+xml"),
            ("image.png", "image/png"),
            ("favicon.ico", "image/x-icon"),
            ("robots.txt", "text/plain; charset=utf-8"),
            ("asset.bin", "application/octet-stream"),
        ];
        for (path, expected) in cases {
            assert_eq!(content_type(Path::new(path)), expected, "{path}");
        }
    }
}
