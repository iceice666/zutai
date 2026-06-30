# Zutai

Zutai is an experimental two-mode language system for data, configuration, validation, and pure data transformation.

- `.zti` is an inert data literal format: deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.
- `.zt` is a pure, lazy, typed computation language over data.

The current repository is an early Rust workspace containing parser, semantic IR, CLI, and editor-support work alongside the versioned language specifications under `docs/spec/`.

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
    dataflow/          TLC-to-Dataflow Core graph lowering
    anf/ ssa/ codegen/ Downstream LLVM/native compile pipeline stages
  immediate/
    core/              Immediate-mode facade over selectable parser backends
    syntax/            Parser and AST definitions for immediate mode (`.zti`)
    simd/              SIMD-accelerated parser for immediate mode (`.zti`)
    types/             Shared AST types for immediate mode (`.zti`)
docs/
  README.md            Documentation index
  language-manual.md     User-facing language manual
  ARCHIVED.md          Archived implementation status and completed milestones
  TBD.md               Open work ledger
  spec/                Versioned language specifications
  spec/v0/             Zutai v0 language specification
  spec/v1/             Deferred post-v0 language design notes
  stdlib/              Standard library notes
```

## Documentation

Start with the language manual or documentation index:

- [Zutai language manual](docs/language-manual.md)
- [docs/README.md](docs/README.md)
- [Archived implementation status](docs/ARCHIVED.md)
- [Open work ledger](docs/TBD.md)
- [Zutai v0 language specification](docs/spec/v0/00-index.md)
- [Final design statement](docs/spec/v0/01-overview/final-design-statement.md)

## Examples

Runnable `.zt` examples live under [examples](examples). The larger examples are:

- [service_health.zt](examples/service_health.zt), backed by
  [service_health.zti](examples/service_health.zti), imports inert data,
  validates typed service records, and produces an operational rollup.
- [canary_forecast.zt](examples/canary_forecast.zt) builds a bounded canary
  report from an infinite synthetic telemetry stream.

Smaller stdlib-focused snippets include [stdlib_pipeline.zt](examples/stdlib_pipeline.zt),
[stream_summary.zt](examples/stream_summary.zt), and
[text_report.zt](examples/text_report.zt).

## Development

This project is a Rust workspace.

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

The shell provides `cargo`, `rustc`, `rustfmt`, `clippy`, and `rust-analyzer`.

## Status

Zutai is under active design. The docs describe the intended v0 language plus selected v1-adjacent features already implemented during the v0 cycle.

Current implementation highlights:

- Immediate mode has syntax and SIMD parser crates behind the `zutai-im` facade.
- General mode parses `.zt`, lowers through HIR and THIR, elaborates to TLC, and has test-covered Dataflow Core, ANF, SSA, LLVM IR, object, and native binary driver stages.
- Constraints/witnesses support named methods and operator methods. Direct, bounded, conditional, and imported witness calls dispatch through TLC dictionary passing on the default evaluator, with the THIR evaluator retained as a regression oracle.
