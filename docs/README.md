# Zutai Documentation

Choose the path that matches what you are trying to do. Stable language
behavior is defined by the specification; implementation status and history do
not override it.

## Learn and use Zutai

- [Language manual](language-manual.md) — start here for syntax, examples, and current support levels
- [Grammar reference](spec/02-lexical/grammar-reference.md) — compact parser-aligned general-mode grammar
- [Standard library](stdlib/00-index.md) — importable modules and ambient prelude

## Language reference

- [Language specification](spec/00-index.md) — normative stable syntax and semantics
- [Reserved language boundaries](design/reserved-language-boundaries.md) — demand-gated non-goals and design constraints

## Compiler contributors

- [Compiler internals](compiler/README.md) — post-frontend IR and runtime documents
- [TLC](compiler/tlc.md)
- [Dataflow Core](compiler/dataflow-core.md)
- [ANF](compiler/anf.md)
- [Runtime and ABI](compiler/runtime-abi.md)

## Project state

- [Implementation status](project/status.md) — current baseline and validation notes
- [Roadmap](project/roadmap.md) — concrete unfinished work only
- [Archived decisions](project/decisions.md) — closed decisions that remain useful
- [Implementation history](history/README.md) — completed milestones grouped by date

## Compiler layer ownership

```text
Source → HIR → THIR → TLC
                        ↓  TLC→DC: tree-to-graph, sharing, explicit recursion
                   Dataflow Core
                        ↓  DC→ANF: topo-sort SCCs, name every sub-expression
                       ANF
                        ↓  ANF→SSA: basic blocks, phi-nodes
                       SSA
                        ↓  SSA→LLVM
                    LLVM IR
                        ↓  llc/clang + libzutai_rt
                    Object / native binary / native library
```

- **HIR** — resolved, source-preserving, not fully typed; produced by `zutai-hir`.
- **Standard library loader** — version-checked filesystem `.zt` sources and manifest handling owned by `zutai-stdlib`.
- **THIR** — typed, source-preserving, error-tolerant; produced by `zutai-thir` and used by LSP tooling.
- **TLC** — fully elaborated with explicit polymorphism; produced by `zutai-tlc` only after successful checking.
- **Dataflow Core** — graph IR where sharing and recursion are structurally explicit; produced by `zutai-dataflow`.
- **ANF** — scheduled `let`/`letrec` bindings with one operation per binding; produced by `zutai-anf`.
- **SSA and LLVM IR** — block form and native emission owned by `zutai-ssa` and `zutai-codegen`.
- **Semantic facade** — `zutai-semantic` wires parse, HIR, THIR, TLC, imports, and stage gates.
- **Reference evaluators** — `zutai-eval` evaluates only complete typed IR and remains the semantics oracle.
- **Browser tooling** — `zutai-web` owns browser bundle builds and the local rebuild/reload server; `zutai-cli web` is a compatibility frontend over the same library.
