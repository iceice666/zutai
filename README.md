# Zutai

Zutai is an experimental two-mode language system for data, configuration, validation, and pure data transformation.

- `.zti` is an inert data literal format: deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.
- `.zt` is a pure, lazy, typed computation language over data.

The current repository is an early Rust workspace containing language-support crates and the v0 language design documents.

## Repository layout

```text
crates/
  cli/                 Command-line interface crate
  general/
    syntax/            Parser and AST definitions for general mode (`.zt`)
    hir/               High-level IR and lowering for general mode (`.zt`)
    semantic/          Semantic analysis framework for general mode (`.zt`)
    eval/              Pure lazy evaluator scaffold for general mode (`.zt`)
    mir/               Mid-level IR for general mode (`.zt`)
  immediate/
    core/              Immediate-mode core APIs
    syntax/            Parser and AST definitions for immediate mode (`.zti`)
    simd/              SIMD-accelerated parser for immediate mode (`.zti`)
    types/             Shared types
docs/
  README.md            Documentation index
  v0_spec/             Zutai v0 language specification
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
cargo test
cargo fmt
cargo clippy --workspace --all-targets
```

If you use Nix, enter the development shell with:

```sh
nix develop
```

The shell provides `cargo`, `rustc`, `rustfmt`, `clippy`, and `rust-analyzer`.

## Status

Zutai is under active design. The docs describe the intended v0 language; implementation details may lag behind the specification.
