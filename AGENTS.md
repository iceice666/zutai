# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project overview

Zutai is an experimental two-mode language system:

- Immediate mode (`.zti`) is an inert data literal format.
- General mode (`.zt`) is a pure, lazy, typed computation language over data.

The implementation is an early Rust workspace. The language design lives under `docs/v0_spec/` and should be treated as the source of truth when changing parser, AST, type-system, or language behavior.

## Repository layout

```text
crates/
  cli/                 Command-line interface crate
  general/syntax/      Parser and AST definitions for general mode (`.zt`)
  immediate/syntax/    Parser and AST definitions for immediate mode (`.zti`)
  immediate/simd/      SIMD-accelerated parser for immediate mode (`.zti`)
  immediate/types/     Shared AST types for immediate mode (`.zti`)
docs/
  README.md            Documentation index
  v0_spec/             Zutai v0 language specification (31 sections, source of truth)
  stdlib/              Standard-library notes
```

## Development commands

Run these before finishing Rust changes when practical:

```sh
cargo fmt
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Agent guidelines

- Prefer small, focused changes.
- Do not overwrite user changes you did not make.
- Read the relevant files in `docs/v0_spec/` before implementing language syntax or semantics.
- Keep crate descriptions and README layout in sync when crates are renamed or added.
- Use Rust 2024 edition conventions from the workspace configuration.
