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

**Interim reference interpreter (`zutai-eval`).** While the back half of the pipeline (TLC, Dataflow Core, ANF, SSA, LLVM) does not yet exist, `crates/general/eval/` provides a THIR tree-walking interpreter that can run any fully type-checked `.zt` program today. It is a *semantics oracle* — it refuses to evaluate any program that is not fully type-checked — and its output is the ground truth for future differential testing of the LLVM backend. It also provides the `run` and `repl` CLI subcommands. "Superseded" applies to the *compilation* strategy; a reference interpreter as a tool is compatible with and complementary to that strategy.

The TLC IR design is specified in [`docs/tlc-core.md`](tlc-core.md). The Dataflow Core design is specified in [`docs/dataflow-core.md`](dataflow-core.md).

## Current Baseline

_Last updated: after the constraints/witnesses milestone._

- Immediate mode parses `.zti` data through selectable parser backends (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and elaborates to TLC.
- THIR is feature-complete for v0: scalar/record/tuple/list literals and patterns, optional access and defaulting, `if`, binary operators, type aliases, block locals, lambda lowering, HM-style unification, match exhaustiveness, let-generalization, predicative polymorphism with call-site `instantiation`, generic type aliases, cross-module imports, and constraints/witnesses (parsing → HIR resolution → THIR type-checking → coherence checking → named-method and operator dispatch in the interpreter).
- The CLI exposes `parse`, `run`, and `repl` subcommands backed by the reference interpreter.
- `crates/general/eval/` is a THIR tree-walking semantics oracle: it refuses to evaluate any program that is not fully type-checked; it provides ground truth for future compiler differential testing.
- `crates/general/tlc/` (TLC — Type Lambda Calculus) is substantially built: TLC IR with kinds, rows, singletons, variants, NbE normalizer, and THIR→TLC lowering are all functional. Phase 3–5 of the TLC sub-roadmap (row-var normalization, effect-row elaboration, dictionary-passing for constraints) remain.
- The backend (Dataflow Core, ANF, SSA, LLVM IR) does not yet exist.

## Phase 1: Complete THIR (LSP Foundation) ✅

Goal: every v0 syntax form parses, lowers through HIR, and produces complete THIR with source-located diagnostics.

**Done.** All items below are implemented and test-covered:

- Parser: declaration disambiguation per spec; `List Int` / `Optional T` type application; rejection of ambiguous pipelines and chained comparisons.
- HIR: name resolution, top-level namespace, local scopes, syntax-only desugarings, module/import name resolution.
- THIR: optional access (`?.`) and defaulting; tuple/record pattern checking; `match` exhaustiveness and unreachable-arm diagnostics; lambda lowering; no-signature function inference; predicative polymorphism (HM unification, call-site instantiation, let-generalization); type-level normalization; import typing for `.zti` and `.zt` modules; **constraints and witnesses** (see below).

### Constraints and Witnesses (v1-adjacent, implemented during v0 cycle)

The `constraint` / `witness` declarations from `docs/v1_spec/03-constraints.md` were built into THIR and the interpreter ahead of schedule because they are needed by the standard library:

- ✅ Parsing and HIR name resolution
- ✅ THIR lowering: `ThirDeclKind::Constraint` / `Witness`, method signatures, operator methods (binding allocated), default method bodies carried through IR
- ✅ Witness type-checking (`check_witnesses`): field sigs matched against constraint method sigs; optional/defaulted methods not required
- ✅ Coherence checking (`check_witness_coherence`): duplicate `(Constraint, Type)` pairs rejected
- ✅ Monomorphic named-method dispatch in `zutai-eval`: `eq 1 2` resolves to `Eq @Int` witness
- ✅ Operator dispatch in `zutai-eval`: `==`, `!=`, `<`, `<=`, `>`, `>=` dispatch to witness fields
- ✅ Default-body dispatch: method call falls back to default body when witness omits it
- ✅ Polymorphic dispatch (direct calls): `eq x y` inside `<A: Eq>` body resolves via injected `WitnessDict` when the bounded function is called at a concrete type from the top level
- ✅ Function type-param bounds recorded in HIR and THIR (`ThirDeclKind::Function.param_bounds`)

**Remaining constraint/witness work** (deferred — each is its own milestone):

- ⬜ **Dictionary-passing in TLC**: the correct solution for polymorphic dispatch through indirect calls. The interpreter currently returns `EvalError::UnresolvedWitness` for bounded functions called from within other functions; TLC elaboration will thread witness arguments as implicit parameters, eliminating this limitation.
- ⬜ **Conditional / higher-kinded witnesses**: `Eq @(List A)` where `A: Eq`; blocked by parametric `AliasApply` targets in `type_key`.
- ⬜ **Cross-module witnesses + orphan rule**: `import.rs` / `export.rs` have no constraint/witness handling.
- ⬜ **`derive` synthesis**: `Witness { derive: true }` currently lowers to an empty-fields no-op in the interpreter.
- ⬜ **Method-level type params** (`<A,B>` on individual methods): dropped at THIR.

Verification gate: `cargo test --workspace` includes spec-shaped parser, HIR, THIR, and semantic facade tests for every v0 chapter. ✅

## Phase 2: TLC (Type Lambda Calculus) 🔄

Goal: produce a fully-elaborated, polymorphism-explicit IR from completed THIR. TLC is only produced when THIR type checking succeeds. It is the clean input contract for all compilation stages downstream.

The full TLC design is specified in [`docs/tlc-core.md`](tlc-core.md).

**Done:**

- ✅ `crates/general/tlc/` (`zutai-tlc`) exists (~3 700 lines of production + test code).
- ✅ TLC IR: `TyLam`/`TyApp`, `VariantT(Row)`, `Singleton(Lit)`, `Variant(label, e)`, kind annotations (`Kind` enum), `Row`/`EffRow`, NbE normalizer with fuel-bounded β-reduction and alias unfolding.
- ✅ THIR→TLC lowering: constraint solving, zonking, let-generalization, call-site instantiation (explicit `TyApp` replaces the `instantiation` stub). TLC Phase 0 ("close the live hole") is complete.
- ✅ `zutai-semantic` exposes `TlcModule` alongside THIR output.

**Remaining (TLC sub-roadmap Phases 3–5):**

- ⬜ **Phase 3 — Row polymorphism**: `RVar` row-tail normalization; open-record and open-union unification. Currently `subst` is not capture-avoiding (sound only because all current type arguments are closed).
- ⬜ **Phase 4 — Effect rows**: `EffRow` on `Fun`; free-monad elaboration for `perform`/`handle`. Scaffolded but `v0` programs are always pure — deferred until the effects chapter of the spec is prioritised.
- ⬜ **Phase 5 — Dictionary-passing for constraints**: elaborate witness arguments as implicit `TyLam`/`TyApp` parameters in TLC, eliminating the interpreter's `UnresolvedWitness` limitation for indirect bounded calls. This is the proper resolution of the constraint/witness polymorphic-dispatch gap noted in Phase 1.

Verification gate: TLC modules for all v0 spec examples have no free `TypeVar`s, correct `VariantT`/`Singleton` nodes, correct `TyLam`/`TyApp` structure, and match expected polymorphic signatures.

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

_Updated to reflect current state._

1. ✅ **~~Finish THIR~~** — complete (lambda, match, optional access, HM polymorphism, constraints/witnesses).
2. 🔄 **Finish TLC** — row-var normalization (Phase 3), then dictionary-passing for constraints (Phase 5, prerequisite for correct polymorphic dispatch). Effect rows (Phase 4) can follow.
3. ⬜ **Dataflow Core crate + TLC→DC lowering** — start with monomorphic programs, then add polymorphism and recursion. Design is in `docs/dataflow-core.md`.
4. ⬜ **ANF lowering** — SCC analysis, topological sort, let/letrec introduction. Write `docs/anf.md` at the start of this phase.
5. ⬜ **SSA + LLVM IR** — standard path from ANF.
6. ⬜ **CLI `compile` subcommand** — wire the full pipeline.

**Constraint/witness completion** can be woven in alongside step 2 or deferred until after step 3 (the interpreter oracle remains useful throughout):

- Dictionary-passing in TLC (step 2, Phase 5) — highest value, unblocks correct polymorphic dispatch
- Conditional witnesses — requires parametric `AliasApply` resolution in `type_key`
- Cross-module witnesses + orphan rule — wires into the module import/export system
- `derive` synthesis — can be a post-v0 convenience
