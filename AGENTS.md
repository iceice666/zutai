# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project overview

Zutai is an experimental two-mode language system:

- Immediate mode (`.zti`) is an inert data literal format.
- General mode (`.zt`) is a pure, lazy, typed computation language over data.

The implementation is an early Rust workspace. The stable language design
lives under `docs/spec/` and is the implementation source of truth when
changing parser, AST, type-system, or language behavior. Zutai has no numbered
language-version buckets; support limits are recorded per feature.

Local skill: use `skill://zutai-language` (project-local `.omp/skills/zutai-language/SKILL.md`) for quick routing to Zutai language facts, source-of-truth docs, implementation support levels, and compiler-layer references before answering language questions or changing language behavior.

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

See `docs/compiler/dataflow-core.md` for the Dataflow Core IR specification,
`docs/project/status.md` for implemented status, and `docs/project/roadmap.md` for the open
phase plan.

## Updating project status, roadmap, and history

- Keep `docs/project/status.md` focused on the current baseline and validation notes.
- Keep active TODOs and TBD items in `docs/project/roadmap.md`, ordered newest/open work first.
- Keep closed decisions that remain useful in `docs/project/decisions.md`.
- When a milestone finishes, move a short summary from the roadmap into the current half-year file under `docs/history/`, newest first, and leave unfinished follow-up in the roadmap.
- State support levels precisely: check-only, reference-interpreter support, backend rejection, or full compile/runtime support.
- Update the relevant "Last updated" note and verification gate when changing implementation status; keep old long-form details compressed unless they explain a current risk.

## Repository layout

```text
crates/
  cli/                 Command-line interface crate
  general/syntax/      Parser and AST definitions for general mode (`.zt`)
  general/hir/         Name-resolved high-level IR and structural validation
  general/thir/        Typed HIR — error-tolerant, source-preserving, LSP foundation
  general/tlc/         Type Lambda Calculus — fully elaborated, explicit TyLam/TyApp
  general/semantic/    Facade wiring parse -> HIR -> THIR; also exposes TLC when available
  general/eval/        Reference interpreter + REPL — TLC-first eval, THIR semantics oracle
  general/dataflow/    Dataflow Core IR — graph-based pure computation representation
  general/anf/         Administrative Normal Form lowering from Dataflow Core
  general/ssa/         SSA form lowering from ANF
  general/codegen/     LLVM IR code generation from SSA
  general/runtime/     Runtime library + ABI (zutai-rt) linked into compiled programs
  immediate/core/      Immediate-mode facade over selectable parser backends
  immediate/syntax/    Parser definitions for immediate mode (`.zti`)
  immediate/simd/      SIMD-accelerated parser for immediate mode (`.zti`)
  immediate/types/     Shared AST types for immediate mode (`.zti`)
docs/
  README.md            Documentation index
  language-manual.md   User-facing current syntax, examples, and support levels
  spec/                Stable language specification, organized by language area
  stdlib/              Standard-library notes
  compiler/            TLC, Dataflow Core, ANF, and runtime/ABI internals
  project/             Current status, roadmap, and archived decisions
  history/             Completed implementation milestones grouped by date
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

Browser kernel wasm-bindgen-test suite (requires a headless browser +
WebDriver from the dev shell, e.g. Chromium + chromedriver on Linux;
`nixpkgs` Chromium is not packaged for Darwin):

```sh
cargo test --target wasm32-unknown-unknown -p zutai-browser --test browser_hydration
```

Scope to `--test browser_hydration` — it is the only test binary in the
crate written against `wasm-bindgen-test`; the crate's other tests are
native-only (`#![cfg(not(target_arch = "wasm32"))]`) since they read the
stdlib from disk, which wasm32 cannot do.

## Agent guidelines

- Prefer small, focused changes.
- Do not overwrite user changes you did not make.
- Read the relevant files in `docs/spec/` before implementing language syntax
  or semantics.
- For remaining roadmap work, treat `docs/project/roadmap.md` as the implementation order; completed post-frontend history lives under `docs/history/`.
- Do not extend parser syntax until the existing surface forms have HIR/THIR/TLC
  semantics. Prefer check-only support with precise unsupported-feature
  diagnostics before claiming compiler or interpreter support.
- Keep parser syntax in `general/syntax`, name resolution and syntax-only normalization in `general/hir`, and type-dependent checking/elaboration in `general/thir`.
- Keep constraint solving, zonking, and TyLam/TyApp elaboration in `general/tlc`; keep graph construction and TLC→DC lowering in `general/dataflow`; keep ANF scheduling in `general/anf`; keep SSA and LLVM emission in `general/ssa` and `general/codegen` respectively.
- Route end-to-end general-mode semantic behavior through `general/semantic` where practical so callers can inspect parse, HIR, THIR, and diagnostics consistently.
- Runtime evaluation semantics live in `general/eval` (`zutai-eval`). All evaluation entry points must remain gated on complete typed IR — never evaluate a program with a THIR error node, TLC elaboration failure, or incomplete type information. The interpreter is a semantics oracle; a wrong value is worse than a refused evaluation.
- Keep crate descriptions and README layout in sync when crates are renamed or added.
- Use Rust 2024 edition conventions from the workspace configuration.
