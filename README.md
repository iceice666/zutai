# Zutai

Zutai is an experimental two-mode language system for data, configuration, validation, and pure data transformation.

- `.zti` is an inert data literal format: deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.
- `.zt` is a pure, lazy, typed computation language over data.

The current repository is an early Rust workspace containing parser, semantic IR, CLI, and editor-support work alongside the v0 language design documents.

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
    anf/ ssa/ codegen/ Downstream compile pipeline stages
  immediate/
    core/              Immediate-mode facade over selectable parser backends
    syntax/            Parser and AST definitions for immediate mode (`.zti`)
    simd/              SIMD-accelerated parser for immediate mode (`.zti`)
    types/             Shared AST types for immediate mode (`.zti`)
docs/
  README.md            Documentation index
  v0_spec/             Zutai v0 language specification
  v1_spec/             Deferred post-v0 language design notes
  stdlib/              Standard library notes
```

## Documentation

Start with the documentation index:

- [docs/README.md](docs/README.md)
- [Zutai v0 language specification](docs/v0_spec/00-index.md)
- [Final design statement](docs/v0_spec/01-overview/final-design-statement.md)

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
- General mode parses `.zt`, lowers through HIR and THIR, elaborates to TLC, and has test-covered Dataflow Core, ANF, SSA, and LLVM IR text stages.
- Constraints/witnesses support named methods and operator methods. Direct and bounded comparison-operator syntax (`==`, `!=`, `<`, `<=`, `>`, `>=`) now dispatches through the same witness dictionaries on the THIR evaluator, TLC evaluator, and import-free TLC→Dataflow path.
