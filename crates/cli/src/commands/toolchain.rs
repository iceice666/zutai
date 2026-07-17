use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
pub(super) const RELOCATION_MODEL: &str = "pic";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildMetadata<'a> {
    format_version: u32,
    artifact: &'a str,
    entry: &'a str,
    compiler_compatibility: &'static str,
    target_triple: &'static str,
    relocation_model: &'static str,
    runtime_abi_version: u32,
    stdlib: BuildInputIdentity<'a>,
    packages: PackageMetadata<'a>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildInputIdentity<'a> {
    compiler_compatibility: &'a str,
    identity: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageMetadata<'a> {
    root: Option<&'a str>,
    identity: String,
    roots: Vec<PackageRoot<'a>>,
}

#[derive(serde::Serialize)]
struct PackageRoot<'a> {
    id: &'a str,
    name: &'a str,
}

pub(super) fn write_build_metadata(
    path: &Path,
    emit: EmitMode,
    recorded: &zutai_semantic::RecordedAnalysis,
) -> Result<(), Box<dyn Error>> {
    let graph_json = serde_json::to_vec(&recorded.packages)?;
    let stdlib_json = serde_json::to_vec(&recorded.stdlib_sources)?;
    let roots = recorded
        .packages
        .packages
        .iter()
        .map(|(id, package)| PackageRoot {
            id,
            name: &package.name,
        })
        .collect();
    let metadata = BuildMetadata {
        format_version: 1,
        artifact: emit_name(emit),
        entry: &recorded.entry,
        compiler_compatibility: env!("CARGO_PKG_VERSION"),
        target_triple: zutai_codegen::target_triple(),
        relocation_model: RELOCATION_MODEL,
        runtime_abi_version: zutai_rt::ABI_VERSION,
        stdlib: BuildInputIdentity {
            compiler_compatibility: &recorded.stdlib_compiler_compatibility,
            identity: zutai_package::sha256_digest(&stdlib_json),
        },
        packages: PackageMetadata {
            root: recorded.packages.root_package.as_deref(),
            identity: zutai_package::sha256_digest(&graph_json),
            roots,
        },
    };
    let mut json = serde_json::to_string_pretty(&metadata)?;
    json.push('\n');
    fs::write(path, json)?;
    Ok(())
}

pub(super) fn emit_name(emit: EmitMode) -> &'static str {
    match emit {
        EmitMode::Llvm => "llvm",
        EmitMode::Obj => "obj",
        EmitMode::Bin => "bin",
        EmitMode::Lib => "lib",
    }
}

pub(super) fn output_path_for(input: &str, output_path: Option<&str>, emit: EmitMode) -> PathBuf {
    if let Some(out) = output_path {
        return PathBuf::from(out);
    }
    let mut out = PathBuf::from(input);
    match emit {
        EmitMode::Llvm => {
            out.set_extension("ll");
        }
        EmitMode::Obj => {
            out.set_extension("o");
        }
        EmitMode::Bin => {
            out.set_extension("");
        }
        EmitMode::Lib => {
            let stem = out
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("zutai");
            let file_name = format!("lib{}{}", stem, shared_library_extension());
            out.set_file_name(file_name);
        }
    };
    out
}

pub(super) fn shared_library_extension() -> &'static str {
    match std::env::consts::OS {
        "macos" => ".dylib",
        "windows" => ".dll",
        _ => ".so",
    }
}

pub(super) fn tool_name(env_name: &str, fallback_env: &str, default: &'static str) -> String {
    std::env::var(env_name)
        .or_else(|_| std::env::var(fallback_env))
        .unwrap_or_else(|_| default.to_string())
}
pub(super) fn runtime_archive_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = std::env::var_os("ZUTAI_RUNTIME_ARCHIVE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "compile error: ZUTAI_RUNTIME_ARCHIVE does not name a runtime archive: {}",
            path.display()
        )
        .into());
    }
    let executable = std::env::current_exe()?;
    let Some(bin_dir) = executable.parent() else {
        return Err(format!(
            "compile error: compiler executable path {} has no parent directory",
            executable.display()
        )
        .into());
    };
    let installed = bin_dir
        .join("..")
        .join("lib")
        .join("zutai")
        .join(zutai_codegen::target_triple())
        .join("libzutai_rt.a");
    if installed.is_file() {
        return Ok(installed);
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli crate lives under crates/cli");
    let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        Some(dir) => workspace.join(dir),
        None => workspace.join("target"),
    };
    let development = target_dir.join("debug").join("libzutai_rt.a");
    if development.is_file() {
        return Ok(development);
    }
    Err(format!(
        "compile error: could not locate libzutai_rt.a at {}; install the target runtime or set ZUTAI_RUNTIME_ARCHIVE",
        installed.display()
    )
    .into())
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
        .arg(format!("-relocation-model={RELOCATION_MODEL}"))
        .arg("-o")
        .arg(out)
        .arg(ll);
    run_tool(&mut command, &llc, "assembling LLVM IR")
}

pub(super) fn runtime_link_flags() -> &'static [&'static str] {
    match std::env::consts::OS {
        "linux" => &["-pie", "-lpthread", "-ldl", "-lm"],
        "macos" => &[],
        _ => &[],
    }
}

pub(super) fn shared_runtime_link_flags() -> &'static [&'static str] {
    match std::env::consts::OS {
        "linux" => &["-lpthread", "-ldl", "-lm"],
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

pub(super) fn link_shared_library(
    obj: &Path,
    runtime: &Path,
    out: &Path,
) -> Result<(), Box<dyn Error>> {
    let clang = tool_name("ZUTAI_CLANG", "CLANG", "clang");
    let mut command = Command::new(&clang);
    match std::env::consts::OS {
        "macos" => {
            command.arg("-dynamiclib");
        }
        _ => {
            command.arg("-shared");
        }
    }
    command.arg(obj);
    match std::env::consts::OS {
        "macos" => {
            command.arg(format!("-Wl,-force_load,{}", runtime.display()));
        }
        _ => {
            command
                .arg("-Wl,--whole-archive")
                .arg(runtime)
                .arg("-Wl,--no-whole-archive");
        }
    }
    for flag in shared_runtime_link_flags() {
        command.arg(flag);
    }
    command.arg("-o").arg(out);
    run_tool(&mut command, &clang, "linking native library")
}
