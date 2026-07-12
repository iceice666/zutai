# Zutai workspace recipes — run `just` to list all
# Every recipe runs inside `nix develop` so tools are always available.
set shell := ["nix", "develop", "--command", "bash", "-c"]

default:
    just --list

# ── Build ─────────────────────────────────────────────────────────────────────

build:
    cargo build --workspace

build-release:
    cargo build --workspace --release

# ── Test ──────────────────────────────────────────────────────────────────────

# Run tests with nextest (accepts extra args, e.g. `just test -p zutai-tlc`)
test *ARGS:
    cargo nextest run --workspace {{ ARGS }}

# Check real examples through check/run/native compile parity.
native-examples:
    cargo test -p zutai-cli --test cli real_examples_check_run_and_compile_match -- --test-threads=1

# Type-check every checked-in .zt example, including network demos.
examples-check:
    @for f in examples/*.zt; do \
        echo "==> check $f"; \
        cargo run -q -p zutai-cli -- check "$f"; \
    done

# Run examples that terminate without an external client.
examples-run:
    @for f in examples/service_health.zt examples/canary_forecast.zt examples/deploy_readiness.zt examples/stdlib_pipeline.zt examples/stream_summary.zt examples/host_stream_read.zt examples/stdlib_fs_lines.zt examples/stdlib_fs_manual.zt examples/stdlib_fs_whole_file.zt examples/text_report.zt; do \
        echo "==> run $f"; \
        cargo run -q -p zutai-cli -- run "$f"; \
    done

# Compile deploy_readiness.zt as a native library and serve a tiny browser page.
browser-demo ADDR="127.0.0.1:8787":
    @set -euo pipefail; \
    case "$(uname -s)" in \
        Darwin) ext=".dylib" ;; \
        MINGW*|MSYS*|CYGWIN*) ext=".dll" ;; \
        *) ext=".so" ;; \
    esac; \
    mkdir -p target/browser-demo; \
    lib="target/browser-demo/libdeploy_readiness$ext"; \
    host="target/browser-demo/zutai-browser-demo-host"; \
    cargo run -q -p zutai-cli -- compile --emit=lib examples/deploy_readiness.zt -o "$lib"; \
    rustc --edition=2024 examples/browser_demo/host.rs -o "$host"; \
    exec "$host" "$lib" "{{ ADDR }}"

# Type-check the official site without producing a bundle.
web-check:
    cargo run -q -p zutai-cli -- check website/main.zt

# Build the official Zutai site and its hashed WebAssembly bundle.
web-build OUT_DIR="dist":
    cargo run -q -p zutai-web -- build website/main.zt --out-dir "{{ OUT_DIR }}"

# Serve a built site through the same Pages-compatible local server used in CI.
web-preview OUT_DIR="dist" PORT="8788": (web-build OUT_DIR)
    wrangler pages dev "{{ OUT_DIR }}" --port "{{ PORT }}"

# ── Lint & format ─────────────────────────────────────────────────────────────

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

clippy:
    cargo clippy --workspace --all-targets

# ── CI gate (what to run before every commit) ─────────────────────────────────

ci: fmt-check test clippy

# ── Coverage ──────────────────────────────────────────────────────────────────

# Line/function coverage summary in terminal
cov:
    cargo llvm-cov nextest --workspace

# HTML coverage report — opens in browser
cov-html:
    cargo llvm-cov nextest --workspace --html --open

# lcov output for external tools / CI upload
cov-lcov:
    cargo llvm-cov nextest --workspace --lcov --output-path target/lcov.info

# ── Docs ──────────────────────────────────────────────────────────────────────

docs-check:
    bash scripts/check-doc-links.sh

doc:
    cargo doc --workspace --no-deps --open

# ── Housekeeping ──────────────────────────────────────────────────────────────

clean:
    cargo clean
