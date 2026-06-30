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
    cargo nextest run --workspace {{ARGS}}

# Check real examples through check/run/native compile parity.
native-examples:
    cargo test -p zutai-cli --test cli real_examples_check_run_and_compile_match -- --test-threads=1

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

doc:
    cargo doc --workspace --no-deps --open

# ── Housekeeping ──────────────────────────────────────────────────────────────

clean:
    cargo clean
