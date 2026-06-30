use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use super::*;

pub(super) fn output_path_for(input: &str, output_path: Option<&str>, emit: EmitMode) -> PathBuf {
    if let Some(out) = output_path {
        return PathBuf::from(out);
    }
    let mut out = PathBuf::from(input);
    match emit {
        EmitMode::Llvm => out.set_extension("ll"),
        EmitMode::Obj => out.set_extension("o"),
        EmitMode::Bin => out.set_extension(""),
    };
    out
}

pub(super) fn tool_name(env_name: &str, fallback_env: &str, default: &'static str) -> String {
    std::env::var(env_name)
        .or_else(|_| std::env::var(fallback_env))
        .unwrap_or_else(|_| default.to_string())
}

pub(super) fn run_tool(
    command: &mut Command,
    tool: &str,
    purpose: &str,
) -> Result<(), Box<dyn Error>> {
    let status = command.status().map_err(|err| {
        format!(
            "compile error: required tool `{tool}` failed to start for {purpose}: {err}; install it, set ZUTAI_{}, or run from a dev shell that provides LLVM/native build tools (for this repo: `nix develop`)",
            tool.to_ascii_uppercase()
        )
    })?;
    if !status.success() {
        return Err(format!("compile error: `{tool}` failed while {purpose}").into());
    }
    Ok(())
}

pub(super) fn assemble_object(ll: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let llc = tool_name("ZUTAI_LLC", "LLC", "llc");
    let mut command = Command::new(&llc);
    command
        .arg("-filetype=obj")
        .arg("-relocation-model=pic")
        .arg("-o")
        .arg(out)
        .arg(ll);
    run_tool(&mut command, &llc, "assembling LLVM IR")
}

pub(super) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli crate lives under crates/cli")
        .to_path_buf()
}

pub(super) fn cargo_target_dir(root: &Path) -> PathBuf {
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

pub(super) fn append_rustflag(flag: &str) -> String {
    match std::env::var("RUSTFLAGS") {
        Ok(existing) if !existing.trim().is_empty() => format!("{existing} {flag}"),
        _ => flag.to_string(),
    }
}

struct RuntimeBuildLock {
    _file: File,
}

pub(super) struct RuntimeArchive {
    path: PathBuf,
    _lock: RuntimeBuildLock,
}

impl RuntimeArchive {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Drop for RuntimeBuildLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self._file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn acquire_runtime_build_lock(target_dir: &Path) -> Result<RuntimeBuildLock, Box<dyn Error>> {
    fs::create_dir_all(target_dir)?;
    let path = target_dir.join(".zutai-rt-build.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    #[cfg(unix)]
    {
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(Box::new(std::io::Error::last_os_error()));
        }
    }
    Ok(RuntimeBuildLock { _file: file })
}

pub(super) fn build_runtime_archive() -> Result<RuntimeArchive, Box<dyn Error>> {
    let root = workspace_root();
    let target_dir = cargo_target_dir(&root);
    let lock = acquire_runtime_build_lock(&target_dir)?;
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("-p")
        .arg("zutai-rt")
        .current_dir(&root);
    if std::env::consts::OS == "linux" {
        command.env("RUSTFLAGS", append_rustflag("-C relocation-model=pic"));
    }
    run_tool(&mut command, "cargo", "building zutai-rt")?;
    Ok(RuntimeArchive {
        path: target_dir.join("debug").join("libzutai_rt.a"),
        _lock: lock,
    })
}

pub(super) fn runtime_link_flags() -> &'static [&'static str] {
    match std::env::consts::OS {
        "linux" => &["-pie", "-lpthread", "-ldl", "-lm"],
        "macos" => &[],
        _ => &[],
    }
}

pub(super) fn link_binary(obj: &Path, runtime: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let clang = tool_name("ZUTAI_CLANG", "CLANG", "clang");
    let mut command = Command::new(&clang);
    command.arg(obj).arg(runtime);
    for flag in runtime_link_flags() {
        command.arg(flag);
    }
    command.arg("-o").arg(out);
    run_tool(&mut command, &clang, "linking native binary")
}
