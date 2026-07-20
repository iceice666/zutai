# Zutai

Zutai is a typed data-transformation language built around inert input, pure
computation, structural validation, and explicit host boundaries.

- `.zti` is an inert data literal format: deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.
- `.zt` is a pure, lazy, typed computation language over data.

The core workflow is inert data, followed by typed validation or
transformation, followed by serializable output. Native compilation, packages,
editor support, and browser execution support and validate that workflow; they
are not independent product directions. New capabilities are admitted only
when a concrete data-oriented program demonstrates that the existing language,
library, or host boundary is insufficient.

## Repository layout

```text
crates/
  cli/                 Command-line interface crate
  general/
    syntax/            Parser and AST definitions for general mode (`.zt`)
    hir/               Name-resolved high-level IR and structural validation
    thir/              Typed HIR, constraints/witness checking, diagnostics foundation
    tlc/               Type Lambda Calculus lowering with explicit polymorphism/dictionaries
    semantic/          Facade wiring parse -> HIR -> THIR -> TLC analysis
    eval/              THIR and TLC reference evaluators
    model/             Bounded explicit-state checking over pure typed `.zt` models
    dataflow/          TLC-to-Dataflow Core graph lowering
    anf/ ssa/ codegen/ Downstream LLVM/native compile pipeline stages
  immediate/
    core/              Immediate-mode facade over selectable parser backends
    syntax/            Parser and AST definitions for immediate mode (`.zti`)
    simd/              SIMD-accelerated parser for immediate mode (`.zti`)
    types/             Shared AST types for immediate mode (`.zti`)
docs/
  README.md            Documentation index
  language-manual.md   User-facing language manual
  spec/                Stable Zutai language specification
  stdlib/              Standard library notes
  compiler/            Compiler IR and runtime internals
  project/             Current status, roadmap, and closed decisions
  history/             Completed implementation milestones by date
```

## Documentation

Start with the language manual or documentation index:

- [Zutai language manual](docs/language-manual.md)
- [docs/README.md](docs/README.md)
- [Implementation status](docs/project/status.md)
- [Open roadmap](docs/project/roadmap.md)
- [Implementation history](docs/history/README.md)
- [Zutai language specification](docs/spec/00-index.md)
- [Final design statement](docs/spec/01-overview/final-design-statement.md)

## Examples

Runnable `.zt` examples live under [examples](examples). See the
[examples guide](examples/README.md) for a suggested reading order, commands,
and notes on the network examples that wait for a local client.

The larger examples are:

- [service_health.zt](examples/service_health.zt), backed by
  [service_health.zti](examples/service_health.zti), imports inert data,
  validates typed service records, and produces an operational rollup.
- [canary_forecast.zt](examples/canary_forecast.zt) builds a bounded canary
  report from an infinite synthetic telemetry stream.

Smaller stdlib-focused snippets include [stdlib_pipeline.zt](examples/stdlib_pipeline.zt),
[stream_summary.zt](examples/stream_summary.zt),
[host_stream_read.zt](examples/host_stream_read.zt), and
[text_report.zt](examples/text_report.zt).

For conventions used by larger examples, see the
[real-program style](docs/language-manual.md#real-program-style) notes.

## Official website

The project website under [website](website) is the browser portability and
integration workload for the stable language. Its state loop, HTML tree, and
typed stylesheet are `.zt`, while its copy is inert `.zti` data. Build its
prerendered WebAssembly bundle with:

```sh
just web-build
# Equivalent direct invocation:
cargo run -p zutai-web -- build website/main.zt
```

`zutai-web` owns this supported integration surface; it is not a commitment to
grow Zutai into a general-purpose frontend framework. The older
`zutai-cli web ...` spelling remains available as a compatibility alias.

See [website/README.md](website/README.md) for the source layout, local preview,
and guarded Cloudflare Pages Direct Upload workflow.

## Development

This project is a Rust workspace.

The standard library is ordinary `.zt` source under `stdlib/packages/*/modules`,
a plain filesystem package tree independent of the Rust workspace (not a Cargo
crate). Workspace Cargo commands automatically set
`ZUTAI_STDLIB_ROOT` through `.cargo/config.toml`. Installed binaries look for
`../share/zutai/stdlib` relative to their executable; `just install` installs
both binaries and those sources. Plain `cargo install` does not install data
files, so set `ZUTAI_STDLIB_ROOT` explicitly when using it.

```sh
cargo build
cargo test --workspace
cargo fmt
cargo clippy --workspace --all-targets
```

If you use Nix, enter the development shell with:

```sh
nix develop
```

The shell provides the Rust workspace tools plus the `wasm32-unknown-unknown`
target, `wasm-bindgen`, Binaryen, and Wrangler for browser builds and local Pages
previews.

## Status

Zutai's currently accepted syntax is stable and documented as one language.
Some constructs have narrower interpreter or native-backend support; those
limits are recorded per feature rather than represented as language versions.

The implementation is organized by role:

- **Core language and data workflow:** immediate-mode data, general-mode parsing and checking, structural validation, data encoding and decoding, reference evaluation, and the `check`/`run`/`format` CLI paths.
- **Supported deployment and tooling:** the Dataflow Core-to-LLVM native pipeline, local packages, diagnostics, formatting, and editor integration.
- **Maintained integration surfaces:** locked Git acquisition, package-wide editor operations, native libraries, browser/Wasm execution, and web tooling. Existing contracts remain tested, but expansion requires a motivating workload and roadmap promotion.

See the [implementation status](docs/project/status.md) for exact support levels
and the [roadmap](docs/project/roadmap.md) for the investment policy.
