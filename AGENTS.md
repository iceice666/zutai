# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project overview

Zutai is an experimental two-mode language system:

- Immediate mode (`.zti`) is an inert data literal format.
- General mode (`.zt`) is a pure, lazy, typed computation language over data.

The implementation is an early Rust workspace. The v0 language design lives under `docs/v0_spec/` and should be treated as the current implementation source of truth when changing parser, AST, type-system, or language behavior. Deferred post-v0 language features live under `docs/v1_spec/`.

## Compilation pipeline

General mode compiles through the following stages:

```
Source → HIR → THIR → TLC
                        ↓  TLC→DC: tree-to-graph, sharing, recursion explicit
                   Dataflow Core
                        ↓  DC→ANF: topo-sort SCCs, name every node, let/letrec
                       ANF
                        ↓  ANF→SSA: basic blocks, phi-nodes
                       SSA
                        ↓  SSA→LLVM
                    LLVM IR
```

THIR is error-tolerant and source-preserving — the foundation for LSP tooling (diagnostics, hover types, go-to-definition). TLC (Type Lambda Calculus) is the fully-elaborated IR with explicit `TyLam`/`TyApp` and no free type variables; it is the clean input for all compilation stages.

See `docs/dataflow-core.md` for the Dataflow Core IR specification,
`docs/ARCHIVED.md` for implemented status, and `docs/TBD.md` for the open
phase plan.

## Updating `docs/ARCHIVED.md` and `docs/TBD.md`

- Keep `docs/ARCHIVED.md` focused on current baseline, validation notes, archived decisions, and completed milestones newest-to-oldest.
- Keep active TODOs and TBD items in `docs/TBD.md`, ordered newest/open work first.
- When a milestone finishes, move a short summary from `docs/TBD.md` into `docs/ARCHIVED.md`'s "Completed milestones, newest first" and leave unfinished follow-up in `docs/TBD.md`.
- State support levels precisely: check-only, reference-interpreter support, backend rejection, or full compile/runtime support.
- Update the relevant "Last updated" note and verification gate when changing implementation status; keep old long-form details compressed unless they explain a current risk.

## Repository layout

```text
crates/
  cli/                 Command-line interface crate
  general/syntax/      Parser and AST definitions for general mode (`.zt`)
  general/hir/         Name-resolved high-level IR and structural validation
  general/thir/        Typed HIR — error-tolerant, source-preserving, LSP foundation
  general/tlc/         Type Lambda Calculus — fully elaborated, explicit TyLam/TyApp (planned)
  general/semantic/    Facade wiring parse -> HIR -> THIR; also exposes TLC when available
  general/eval/        Interim THIR reference interpreter + REPL (semantics oracle)
  general/dataflow/    Dataflow Core IR — graph-based pure computation representation (planned)
  general/anf/         Administrative Normal Form lowering from Dataflow Core (planned)
  general/ssa/         SSA form lowering from ANF (planned)
  general/codegen/     LLVM IR code generation from SSA (planned)
  immediate/core/      Immediate-mode facade over selectable parser backends
  immediate/syntax/    Parser definitions for immediate mode (`.zti`)
  immediate/simd/      SIMD-accelerated parser for immediate mode (`.zti`)
  immediate/types/     Shared AST types for immediate mode (`.zti`)
docs/
  README.md            Documentation index
  dataflow-core.md     Dataflow Core IR design specification
  ARCHIVED.md          Archived implementation status and completed milestones
  TBD.md               Open milestones and unresolved TBD items
  v0_spec/             Zutai v0 language specification (8 chapters, source of truth)
  v1_spec/             Zutai v1 deferred feature specification
  stdlib/              Standard-library notes
```

## Development commands

Run these before finishing Rust changes when practical:

```sh
cargo fmt
cargo test --workspace
cargo clippy --workspace --all-targets
```

Coverage (requires `cargo-llvm-cov` and `cargo-nextest` from the dev shell):

```sh
cargo llvm-cov nextest --workspace
```

Add `--html` to generate an HTML report in `target/llvm-cov/html/`.

## Agent guidelines

- Prefer small, focused changes.
- Do not overwrite user changes you did not make.
- Read the relevant files in `docs/v0_spec/` before implementing v0 language syntax or semantics; read `docs/v1_spec/` as design context only when working on deferred v1 features.
- For remaining roadmap work, treat `docs/TBD.md` as the implementation order; completed post-frontend history lives in `docs/ARCHIVED.md`.
- For v1 features, do not extend parser syntax further until the existing surface forms have HIR/THIR/TLC semantics. Prefer check-only support with precise unsupported-feature diagnostics before claiming compiler or interpreter support.
- Keep parser syntax in `general/syntax`, name resolution and syntax-only normalization in `general/hir`, and type-dependent checking/elaboration in `general/thir`.
- Keep constraint solving, zonking, and TyLam/TyApp elaboration in `general/tlc`; keep graph construction and TLC→DC lowering in `general/dataflow`; keep ANF scheduling in `general/anf`; keep SSA and LLVM emission in `general/ssa` and `general/codegen` respectively.
- Route end-to-end general-mode semantic behavior through `general/semantic` where practical so callers can inspect parse, HIR, THIR, and diagnostics consistently.
- Runtime evaluation semantics live in `general/eval` (`zutai-eval`). All evaluation entry points must remain gated on complete typed IR — never evaluate a program with a THIR error node, TLC elaboration failure, or incomplete type information. The interpreter is a semantics oracle; a wrong value is worse than a refused evaluation.
- Keep crate descriptions and README layout in sync when crates are renamed or added.
- Use Rust 2024 edition conventions from the workspace configuration.
