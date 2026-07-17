#!/usr/bin/env python3
"""Measure representative Zutai workloads without external benchmark tools."""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REPEAT = 5
DEFAULT_WARMUP = 1
HEAP_RE = re.compile(
    r"zutai heap stats: allocated (?P<bytes>\d+) bytes in (?P<objects>\d+) objects .*?"
    r"peak committed (?P<peak>\d+) bytes .*?by kind: record (?P<record>\d+), "
    r"tuple (?P<tuple>\d+), cons (?P<cons>\d+), variant (?P<variant>\d+), "
    r"text (?P<text>\d+), closure/raw (?P<closure_raw>\d+)\."
)

WORKLOADS = (
    {
        "name": "website",
        "kind": "web",
        "entry": "website/main.zt",
    },
    {
        "name": "configuration_decoder",
        "kind": "native",
        "entry": "examples/from_data_runtime.zt",
    },
    {
        "name": "stream",
        "kind": "native",
        "entry": "examples/stream_summary.zt",
    },
    {
        "name": "effectful_service",
        "kind": "native",
        "entry": "examples/host_stream_read.zt",
    },
)


def run(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: int = 600,
) -> subprocess.CompletedProcess[bytes]:
    completed = subprocess.run(
        args,
        cwd=ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if completed.returncode != 0:
        command = " ".join(args)
        stdout = completed.stdout.decode("utf-8", errors="replace")
        stderr = completed.stderr.decode("utf-8", errors="replace")
        raise RuntimeError(
            f"command failed ({completed.returncode}): {command}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )
    return completed


def measure(args: list[str], repeat: int, *, env: dict[str, str] | None = None) -> dict:
    samples: list[float] = []
    stdout: bytes | None = None
    stderr: bytes | None = None
    for _ in range(repeat):
        start = time.perf_counter_ns()
        completed = run(args, env=env)
        samples.append((time.perf_counter_ns() - start) / 1_000_000.0)
        if stdout is None:
            stdout = completed.stdout
            stderr = completed.stderr
        elif completed.stdout != stdout:
            raise RuntimeError(f"non-deterministic stdout from {' '.join(args)}")
    assert stdout is not None and stderr is not None
    return {
        "samples_ms": [round(value, 3) for value in samples],
        "median_ms": round(statistics.median(samples), 3),
        "min_ms": round(min(samples), 3),
        "max_ms": round(max(samples), 3),
        "stdout_sha256": sha256(stdout),
        "stderr": stderr.decode("utf-8", errors="replace"),
    }


def sha256(data: bytes) -> str:
    import hashlib

    return hashlib.sha256(data).hexdigest()


def file_sizes(root: Path) -> dict[str, int]:
    return {
        str(path.relative_to(root)): path.stat().st_size
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def parse_heap(stderr: str) -> dict[str, int]:
    match = HEAP_RE.search(stderr)
    if match is None:
        raise RuntimeError(f"native run did not emit heap statistics:\n{stderr}")
    return {key: int(value) for key, value in match.groupdict().items()}


def compiler_version(cli: Path) -> str:
    return f"zutai-cli {workspace_version()} ({sha256(cli.read_bytes())})"


def tool_version(command: list[str], needle: str | None = None) -> str:
    completed = run(command)
    text = (completed.stdout or completed.stderr).decode("utf-8", errors="replace")
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if needle is not None:
        for line in lines:
            if needle.casefold() in line.casefold():
                return line
        raise RuntimeError(f"missing {needle!r} version line from {' '.join(command)}: {text}")
    if not lines:
        raise RuntimeError(f"empty version output from {' '.join(command)}")
    return lines[0]



def cpu_model() -> str:
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.is_file():
        match = re.search(
            r"^model name\s*:\s*(.+)$",
            cpuinfo.read_text(encoding="utf-8"),
            re.MULTILINE,
        )
        if match is not None:
            return match.group(1).strip()
    return platform.processor()


def workspace_version() -> str:
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r"^version\s*=\s*\"([^\"]+)\"$", manifest, re.MULTILINE)
    if match is None:
        raise RuntimeError("workspace package version is missing from Cargo.toml")
    return match.group(1)


def build_tools() -> tuple[Path, Path, Path]:
    run(["cargo", "build", "--release", "-p", "zutai-cli", "-p", "zutai-web", "-p", "zutai-rt"])
    cli = ROOT / "target/release/zutai-cli"
    web = ROOT / "target/release/zutai-web"
    runtime = ROOT / "target/release/libzutai_rt.a"
    return cli, web, runtime


def measure_native(
    workload: dict[str, str],
    cli: Path,
    runtime: Path,
    out_dir: Path,
    repeat: int,
    warmup: int,
) -> dict:
    name = workload["name"]
    entry = workload["entry"]
    binary = out_dir / name
    common = [str(cli), "--stdlib-root", "stdlib"]
    compile_args = common + ["compile", "--emit=bin", entry, "-o", str(binary)]
    compile_env = os.environ.copy()
    compile_env["ZUTAI_RUNTIME_ARCHIVE"] = str(runtime)
    for _ in range(warmup):
        run(compile_args, env=compile_env)
    compile_result = measure(compile_args, repeat, env=compile_env)

    interpreter_args = common + ["run", entry]
    for _ in range(warmup):
        run(interpreter_args)
    interpreter_result = measure(interpreter_args, repeat)

    native_env = os.environ.copy()
    native_env["ZUTAI_HEAP_STATS"] = "1"
    for _ in range(warmup):
        run([str(binary)], env=native_env)
    native_result = measure([str(binary)], repeat, env=native_env)
    heap = parse_heap(native_result.pop("stderr"))
    interpreter_result.pop("stderr")
    compile_result.pop("stderr")

    if interpreter_result["stdout_sha256"] != native_result["stdout_sha256"]:
        raise RuntimeError(f"interpreter/native output mismatch for {name}")

    return {
        "name": name,
        "entry": entry,
        "kind": "native",
        "compile": compile_result,
        "interpreter_runtime": interpreter_result,
        "native_runtime": native_result,
        "allocation": heap,
        "output_bytes": binary.stat().st_size,
        "parity": "interpreter/native stdout SHA-256 match",
    }


def measure_website_runtime(destination: Path, repeat: int, warmup: int) -> dict | str:
    chromium = shutil.which("chromium") or shutil.which("chromium-browser")
    if chromium is None:
        return "not sampled: chromium is unavailable"
    args = [
        chromium,
        "--headless",
        "--disable-gpu",
        "--no-sandbox",
        "--dump-dom",
        destination.joinpath("index.html").as_uri(),
    ]
    for _ in range(warmup):
        run(args, timeout=120)
    result = measure(args, repeat)
    result.pop("stderr")
    result["scope"] = "headless Chromium process start, local bundle hydration, and DOM dump"
    return result


def measure_website(
    workload: dict[str, str],
    web: Path,
    out_dir: Path,
    repeat: int,
    warmup: int,
) -> dict:
    builds = out_dir / "website-builds"
    builds.mkdir()
    samples: list[float] = []
    hashes: list[str] = []
    sizes: dict[str, int] | None = None
    runtime_destination: Path | None = None
    for index in range(warmup + repeat):
        destination = builds / f"run-{index}"
        start = time.perf_counter_ns()
        run(
            [
                str(web),
                "--stdlib-root",
                "stdlib",
                "build",
                workload["entry"],
                "--out-dir",
                str(destination),
            ],
            timeout=1200,
        )
        elapsed = (time.perf_counter_ns() - start) / 1_000_000.0
        current_sizes = file_sizes(destination)
        digest = sha256(
            b"".join(
                path.relative_to(destination).as_posix().encode() + b"\0" + path.read_bytes()
                for path in sorted(destination.rglob("*"))
                if path.is_file()
            )
        )
        if index >= warmup:
            runtime_destination = destination
            samples.append(elapsed)
            hashes.append(digest)
            if sizes is None:
                sizes = current_sizes
            elif current_sizes != sizes:
                raise RuntimeError("website output sizes changed between repeated builds")
    if len(set(hashes)) != 1:
        raise RuntimeError("website output content changed between repeated builds")
    assert sizes is not None and runtime_destination is not None
    return {
        "name": workload["name"],
        "entry": workload["entry"],
        "kind": "web",
        "compile": {
            "samples_ms": [round(value, 3) for value in samples],
            "median_ms": round(statistics.median(samples), 3),
            "min_ms": round(min(samples), 3),
            "max_ms": round(max(samples), 3),
        },
        "runtime": measure_website_runtime(runtime_destination, repeat, warmup),
        "output_bytes": sum(sizes.values()),
        "output_files": sizes,
        "determinism": "all repeated output tree SHA-256 values match",
        "runtime_allocation": "browser-hosted; native heap counters do not cover Wasm execution",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repeat", type=int, default=DEFAULT_REPEAT)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repeat < 1 or args.warmup < 0:
        parser.error("--repeat must be positive and --warmup must be non-negative")

    cli, web, runtime = build_tools()
    with tempfile.TemporaryDirectory(prefix="zutai-baseline-") as temporary:
        out_dir = Path(temporary)
        results = []
        for workload in WORKLOADS:
            if workload["kind"] == "web":
                results.append(
                    measure_website(workload, web, out_dir, args.repeat, args.warmup)
                )
            else:
                results.append(
                    measure_native(
                        workload,
                        cli,
                        runtime,
                        out_dir,
                        args.repeat,
                        args.warmup,
                    )
                )

    report = {
        "schema_version": 1,
        "measurement_policy": {
            "clock": "Python time.perf_counter_ns around child-process completion",
            "summary": "median with min/max and raw samples",
            "repeat": args.repeat,
            "warmup": args.warmup,
            "native_allocation": "ZUTAI_HEAP_STATS with the default-on collector",
            "website_runtime": "headless Chromium process startup, local bundle hydration, and DOM dump when chromium is available",
        },
        "host": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "processor": cpu_model(),
            "python": platform.python_version(),
            "rustc": run(["rustc", "-Vv"]).stdout.decode().strip(),
            "compiler": compiler_version(cli),
            "llc": tool_version(["llc", "--version"], "LLVM version"),
            "clang": tool_version(["clang", "--version"]),
            "wasm_bindgen": tool_version(["wasm-bindgen", "--version"]),
            "wasm_opt": tool_version(["wasm-opt", "--version"]),
        },
        "workloads": results,
    }
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output is None:
        sys.stdout.write(rendered)
    else:
        output = args.output if args.output.is_absolute() else ROOT / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered, encoding="utf-8")
        print(output.relative_to(ROOT) if output.is_relative_to(ROOT) else output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
