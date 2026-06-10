# Zutai v0 Implementation Roadmap

This roadmap tracks the path from the current parser/HIR/THIR workspace to a complete v0 implementation with an AOT compiler targeting LLVM IR. The v0 spec under `docs/v0_spec/` remains the source of truth; this document is an implementation plan, not a language-design override.

## Compilation pipeline

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

**THIR** is the error-tolerant, source-preserving typed IR. It carries spans on every node, tolerates partial type information, and is produced even when type checking fails partially. It is the foundation for LSP (language server) features: go-to-definition, hover types, inline diagnostics. THIR is also the final output of the `check` subcommand.

**TLC** (Type Lambda Calculus) is the fully-elaborated IR produced only when type checking succeeds. All inference variables are resolved, polymorphism is explicit via `TyLam`/`TyApp`, and complete type information is guaranteed. TLC is the clean input contract for all compilation stages.

The production execution strategy is this AOT compile pipeline. Laziness and sharing are represented **structurally** in Dataflow Core — not via runtime thunks. No tree-walking interpreter sits on the critical path to production.

**Interim reference interpreter (`zutai-eval`).** While the back half of the pipeline (TLC, Dataflow Core, ANF, SSA, LLVM) does not yet exist, `crates/general/eval/` provides a THIR tree-walking interpreter that can run any fully type-checked `.zt` program today. It is a *semantics oracle* — it refuses to evaluate any program that is not fully type-checked — and its output is the ground truth for future differential testing of the LLVM backend. It also provides the `run` and `repl` CLI subcommands. See [`docs/decisions/0002-interim-thir-interpreter.md`](decisions/0002-interim-thir-interpreter.md) for the full rationale. "Superseded" applies to the *compilation* strategy; a reference interpreter as a tool is compatible with and complementary to that strategy.

The Dataflow Core design is specified in [`docs/dataflow-core.md`](dataflow-core.md).

## Current Baseline

- Immediate mode parses `.zti` data through selectable parser backends.
- General mode parses `.zt`, lowers to HIR, and partially lowers/checks THIR.
- THIR currently supports scalar literals, records, tuples, lists, closed record checking, field access, optional-field defaulting, `if`, scalar binary operators, type aliases, block locals, and explicitly typed monomorphic function declarations/application.
- The CLI currently analyzes files and prints ASTs; it does not compile or execute `.zt` output.

## Phase 1: Complete THIR (LSP Foundation)

Goal: every v0 syntax form parses, lowers through HIR, and produces complete THIR with source-located diagnostics. THIR is the stable foundation for LSP tooling — it must be robust, span-preserving, and useful even when type inference is incomplete.

- Parser
  - Keep declaration disambiguation aligned with `docs/v0_spec/02-lexical/grammar-sketch.md`.
  - Cover type application such as `List Int` and `Optional T`.
  - Preserve existing rejection of ambiguous mixed pipelines and chained comparisons.
- HIR
  - Keep name resolution, top-level namespace rules, local scopes, and syntax-only desugarings here.
  - Add module/import name resolution once module loading is introduced.
- THIR
  - Finish optional access lowering (`?.`) and defaulting flattening rules.
  - Add tuple and record pattern checking.
  - Add `match` exhaustiveness and unreachable-arm diagnostics.
  - Add lambda lowering with closure type inference.
  - Add no-signature function inference.
  - Add predicative polymorphism: `TypeVar` inference variables, call-site unification, let-generalization.
  - Add type-level expression normalization with deterministic limits.
  - Add import typing for `.zti` and `.zt` modules.
  - Keep `TypeVar` and the `instantiation` stub for now — TLC will replace them in Phase 2.

Verification gate: `cargo test --workspace` includes spec-shaped parser, HIR, THIR, and semantic facade tests for every v0 chapter.

## Phase 2: TLC (Type Lambda Calculus)

Goal: produce a fully-elaborated, polymorphism-explicit IR from completed THIR. TLC is only produced when THIR type checking succeeds. It is the clean input contract for all compilation stages downstream.

- Add crate `crates/general/tlc/` (`zutai-tlc`).
- Implement the TLC IR: `TyLam`/`TyApp` structural nodes, no `TypeVar` free, spans in a side-table only.
- Implement the THIR→TLC lowering pass:
  - **Constraint solving** — run the unification engine on all accumulated type constraints; report unsolvable constraints as THIR diagnostics.
  - **Zonking** — substitute all solved `TypeVar`s with their concrete types throughout the expression tree.
  - **Let-generalization** — generalize let-bound and top-level bindings whose remaining free `TypeVar`s can be safely quantified; wrap with `TyLam`.
  - **Call-site instantiation** — replace the `instantiation: Vec<TypeId>` stub on `Apply` with explicit `TyApp` nodes at polymorphic call sites.
- Extend `zutai-semantic` to expose `TlcModule` alongside THIR output.

Verification gate: TLC modules for all v0 spec examples have no free `TypeVar`s, correct `TyLam`/`TyApp` structure, and match expected polymorphic signatures.

## Phase 3: Dataflow Core

Goal: lower the completed TLC to a Dataflow Core graph where sharing, laziness, and recursion are structurally explicit.

- Add crate `crates/general/dataflow/` (`zutai-dataflow`).
- Implement the `DataflowGraph` IR as specified in `docs/dataflow-core.md`.
- Implement the TLC→DC lowering pass in `zutai-dataflow::lower`:
  - Tree-to-graph conversion: local bindings lowered once; all references share a single `NodeId`.
  - Global-to-`GlobalRef` conversion: top-level references become `GlobalRef` nodes.
  - Recursive definitions: body may produce `GlobalRef` nodes pointing back to the same global (cycles); these are valid and expected.
  - Multi-clause functions: desugar into `Lambda + Match`.
  - Polymorphic functions: `TyLam` → `TyLam` node; call-site `TyApp` → `TyApp` node.
- Implement the DC validation pass (invariant checking in debug builds).

Verification gate: unit tests lower TLC for all v0 language forms and assert correct graph structure (sharing, SCC detection, type consistency).

## Phase 4: ANF Lowering

Goal: convert the Dataflow Core graph into Administrative Normal Form — a linear schedule where every sub-expression is named by a `let` or `letrec` binding.

- Add crate `crates/general/anf/` (`zutai-anf`).
- Implement SCC analysis on the global dependency graph.
- Implement a topological sort of SCCs.
- Implement node-to-ANF lowering: one fresh name per non-trivial sub-expression.
- Emit `let` for non-recursive SCCs; emit `letrec` for recursive or mutually-recursive SCCs.
- The ANF design is specified in `docs/anf.md` (to be written at the start of this phase).

Verification gate: ANF-lowered modules for all v0 forms are well-formed (every name defined before first use; `letrec` only where cycles exist in DC).

## Phase 5: SSA and LLVM IR

Goal: compile ANF to SSA form and emit LLVM IR.

- Add crate `crates/general/ssa/` (`zutai-ssa`).
- Lower ANF functions to basic-block SSA: introduce phi-nodes for branches, eliminate nested lets into straight-line code within blocks.
- Add crate `crates/general/codegen/` (`zutai-codegen`).
- Emit LLVM IR via `inkwell` or `llvm-sys`.
- Represent v0 values as LLVM types: `i64` for Int, `double` for Float, `i1` for Bool, pointer-tagged structs for records/tuples/lists, closures as function-pointer + environment pairs.
- Map Zutai's structural laziness (unreachable DC nodes = dead code) to LLVM dead-code elimination; do not emit thunk machinery.
- Map `letrec` to LLVM IR functions with mutual tail-call or direct-call structure.

Verification gate: LLVM IR for the complete example and all v0 spec examples compiles without errors; `opt -O2` produces plausible output.

## Phase 6: CLI Compilation

Goal: make `zutai-cli` a usable compiler for `.zt` files.

- Replace the single positional mode with subcommands:
  - `parse <path>` — print AST or parse diagnostics.
  - `check <path>` — run parse, HIR, THIR, and semantic diagnostics (THIR output; no TLC needed).
  - `compile <path> [-o output]` — compile to a native binary via LLVM.
  - `dataflow <path>` — print the Dataflow Core graph (debugging aid).
- Add output rendering for diagnostics with source locations.
- Keep diagnostics source-located through the semantic facade.

Verification gate: CLI integration tests cover successful `.zt` compile + run, parse errors, semantic errors, and a check-only invocation.

## Near-Term Implementation Order

1. **Finish THIR** — lambda lowering, match lowering, optional access, polymorphism with unification engine.
2. **TLC crate + THIR→TLC lowering** — constraint solving, zonking, generalization, explicit TyLam/TyApp.
3. **Dataflow Core crate + TLC→DC lowering** — start with monomorphic programs, then add polymorphism and recursion.
4. **ANF lowering** — SCC analysis, topological sort, let/letrec introduction.
5. **SSA + LLVM IR** — standard path from ANF.
6. **CLI `compile` subcommand** — wire the full pipeline.
