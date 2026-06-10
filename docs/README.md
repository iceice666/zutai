# Zutai Documentation

## Sections

- [v0 language specification](v0_spec/00-index.md)
- [v0 implementation roadmap](v0-implementation-roadmap.md)
- [Dataflow Core IR design](dataflow-core.md)
- [v1 deferred feature specification](v1_spec/00-index.md)
- [Standard library](stdlib/00-index.md)
- [Design decisions](decisions/0001-record-update-and-config-layering.md)

## General-mode compiler layers

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
```

- **HIR** — resolved, source-preserving, not fully typed. Produced by `zutai-hir`.
- **THIR** — typed, source-preserving, error-tolerant. Carries spans on every node; produced even when type inference is incomplete. Foundation for LSP tooling (diagnostics, hover types, go-to-definition). Produced by `zutai-thir`.
- **TLC** (Type Lambda Calculus) — fully elaborated; all inference variables resolved; polymorphism explicit via `TyLam`/`TyApp`; spans in a side-table only. Produced only when type checking succeeds. Clean input contract for all compilation stages. Produced by `zutai-tlc`.
- **Dataflow Core** — graph IR where sharing and recursion are structurally explicit. A node may be referenced by many consumers (sharing); a cycle represents recursion. Laziness = graph reachability from the output root. Produced by `zutai-dataflow`.
- **ANF** — linear schedule of `let`/`letrec` bindings with one operation per binding. Every sub-expression is named. SCCs from the DC graph become `letrec` groups. Produced by `zutai-anf`.
- **SSA** — basic blocks with phi-nodes. Standard form for LLVM emission. Produced by `zutai-ssa`.
- **LLVM IR** — final backend target. Emitted by `zutai-codegen`.
- **Semantic facade** (`zutai-semantic`) — wires parse, HIR, THIR, and TLC into one staged API. Passes live in the IR crate they transform.
