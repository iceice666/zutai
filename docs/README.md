# Zutai Documentation

## Sections

- [Zutai language manual](language-manual.md)
- [v0 language specification](v0_spec/00-index.md)
- [archived implementation status](ARCHIVED.md)
- [open work ledger](TBD.md)
- [TLC IR design](tlc-core.md)
- [Dataflow Core IR design](dataflow-core.md)
- [v1 deferred feature specification](v1_spec/00-index.md)
- [v2 deferred feature specification](v2_spec/00-index.md)
- [v3 feature specification](v3_spec/00-index.md)
- [Standard library](stdlib/00-index.md)

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
                        ↓  llc/clang + libzutai_rt
                    Object / native binary
```

- **HIR** — resolved, source-preserving, not fully typed. Produced by `zutai-hir`.
- **THIR** — typed, source-preserving, error-tolerant. Carries spans on every node; produced even when type inference is incomplete. Foundation for LSP tooling (diagnostics, hover types, go-to-definition). Produced by `zutai-thir`.
- **TLC** (Type Lambda Calculus) — fully elaborated; all inference variables resolved; polymorphism explicit via `TyLam`/`TyApp`; spans in a side-table only. Produced only when type checking succeeds. Clean input contract for all compilation stages. Produced by `zutai-tlc`. See [TLC IR design](tlc-core.md).
- **Dataflow Core** — graph IR where sharing and recursion are structurally explicit. A node may be referenced by many consumers (sharing); a cycle represents recursion. Laziness = graph reachability from the output root. Produced by `zutai-dataflow`.
- **ANF** — linear schedule of `let`/`letrec` bindings with one operation per binding. Every sub-expression is named. SCCs from the DC graph become `letrec` groups. Produced by `zutai-anf`.
- **SSA** — basic blocks with phi-nodes. Standard form for LLVM emission. Produced by `zutai-ssa`.
- **LLVM IR / native output** — final backend target. `zutai-codegen` emits LLVM text with runtime descriptors; `zutai-cli compile --emit=llvm|obj|bin` writes IR, objects, or linked native binaries when the host LLVM toolchain is available.
- **Semantic facade** (`zutai-semantic`) — wires parse, HIR, THIR, and TLC into one staged API; owns filesystem/module-graph analysis (imports, caches, cycles, witness merging) and cross-stage gate predicates. Passes live in the IR crate they transform.
- **Reference evaluators** (`zutai-eval`) — semantics oracles over completed typed IR. The default `run`/`repl` path is TLC-first for executable value programs; the THIR evaluator remains the regression oracle and the runtime `Type`/reflection boundary until TLC represents type values directly. All evaluators refuse programs that are not fully type-checked.
