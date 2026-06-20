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

**Interim reference interpreter (`zutai-eval`).** `crates/general/eval/` provides a semantics oracle that refuses to evaluate any program that is not fully type-checked. The default `run` path still uses the THIR evaluator for pure programs; `eval_tlc_file` runs the same source through TLC elaboration and the TLC eager walker for compiler-path parity checks. "Superseded" applies to the *compilation* strategy; reference interpreters remain compatible with and complementary to that strategy.

The TLC IR design is specified in [`docs/tlc-core.md`](tlc-core.md). The Dataflow Core design is specified in [`docs/dataflow-core.md`](dataflow-core.md).

## Current Baseline

_Last updated: after canonical Optional runtime representation._

- Immediate mode parses `.zti` data through selectable parser backends (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and elaborates to TLC.
- THIR is feature-complete for v0: scalar/record/tuple/list literals and patterns, optional access and defaulting, `if`, binary operators, type aliases, block locals, lambda lowering, HM-style unification, match exhaustiveness, let-generalization, predicative polymorphism with call-site `instantiation`, generic type aliases, cross-module imports, and constraints/witnesses (parsing → HIR resolution → THIR type-checking → coherence checking → named-method and operator dispatch in the interpreter).
- The CLI exposes `parse`, `run`, and `repl` subcommands backed by the reference interpreter.
- `crates/general/eval/` contains both the THIR oracle and the TLC eager walker. Their import-free differential battery now covers direct and bounded operator witnesses, so `eval_file`, `eval_tlc_file`, and TLC→Dataflow lowering agree for those programs.
- `print` remains seeded in the prelude (`zutai_hir::BUILTIN_VALUE_NAMES`) as a compatibility binding, but its type is now `Text -> Text ! { io.print : Text -> Text }`. The TLC evaluator represents it as an `io.print` effect: source handlers can intercept it, and the host `run` boundary handles residual `io.print`. `compile`/`dataflow` reject residual effect markers or non-empty function effect rows after TLC lowering.
- `crates/general/tlc/` (TLC — Type Lambda Calculus) is complete through Phase 5 plus operator-witness parity: TLC IR with kinds, rows (`RVar`), singletons, variants, NbE normalizer, effect rows + eraser, dictionary-passing elaboration for named constraint methods, and witnessed comparison-operator lowering are all functional.
- `crates/general/dataflow/` (Dataflow Core) and `crates/general/anf/` (ANF) exist and are test-covered.
- `crates/general/ssa/` (SSA) and `crates/general/codegen/` (LLVM IR codegen) exist and are test-covered. SSA provides basic-block IR with phi nodes; codegen emits LLVM IR text using an `i64` universal value representation for v0.

## V0 Validation Findings (TBD)

_Added after a v0 stress-test/validation pass: real `.zt` programs run through `run`/`compile`, plus a THIR-vs-TLC differential oracle (`crates/general/eval/tests/differential.rs`). Four bugs were fixed in that pass — chained-comparison false positive across `&&`/`||`/`??`/pipelines, `??` ignoring explicit `#none`/`#some`, deep-recursion native stack overflow (CLI now evaluates on a large-stack worker thread), and malformed `inf.0`/`NaN.0` float display. TLC/operator-witness parity was fixed afterwards: direct and bounded `==`/`!=`/`<`/`<=`/`>`/`>=` syntax now uses the same witness dictionaries as named methods on the THIR evaluator, TLC evaluator, and TLC→Dataflow path. The remaining items below are deferred work or corrections to claims elsewhere in this file._

- **Canonical Optional runtime representation is fixed in the reference walkers.** Optional field access and `?.` now produce canonical `Optional` union values in both the THIR walker (`eval_file`) and TLC walker (`eval_tlc_file`): absent fields and explicit `#none` yield `#none`, present fields yield `#some { value = v }`, and `field? : T?` access flattens to the stored `T?` value. `??`, `?.`, and `match` consume that single representation. Deferred sliver: matching an absent optional field inside a record pattern still fails because the record-pattern matcher has no field name/value to hand to the subpattern when the field is absent.
- **Spec example inconsistencies (parser follows the formal grammar; the chapter examples are stale).** `grammar-sketch.md` defines record-pattern fields as `;`-terminated (`RecordPattern ::= "{" (FieldName "=" Pattern ";")* "}"`), but `pattern-matching.md`/`complete-example.md` write `#circle { radius = r }` without the trailing `;`, which the parser rejects. Likewise several examples (`records.md`, `overview.md`, `config.md`) use single-colon typed value bindings `name : Type = value`, but the symbol table and `grammar-sketch.md` reserve `:` for type positions and require `::` (the parser raises `TopLevelSingleColon`). Either relax the parser or fix the examples.
- **Minor:** `Int??` lexes as `Int` + `??` (defaulting), not a double optional — write `(Int?)?`. Type-mismatch diagnostics between two distinct record types render as the unhelpful "expected record, found record" (the type formatter collapses record structure).

### `print` and the effect system

The old string-only `print` builtin has been re-pointed to the effect system: `print` performs `io.print`, source handlers can intercept it, and the host `run` boundary provides the current console handler. The compiled pure core still stays effect-free until a post-TLC effect lowering exists; `compile`/`dataflow` use a residual-effect gate instead of treating `print` as an interpreter-only ambient primitive.

## Phase 1: Complete THIR (LSP Foundation) ✅

Goal: every v0 syntax form parses, lowers through HIR, and produces complete THIR with source-located diagnostics.

**Done.** All items below are implemented and test-covered:

- Parser: declaration disambiguation per spec; `List Int` / `Optional T` type application; rejection of ambiguous pipelines and chained comparisons.
- HIR: name resolution, top-level namespace, local scopes, syntax-only desugarings, module/import name resolution.
- THIR: optional access (`?.`) and defaulting; tuple/record pattern checking; `match` exhaustiveness and unreachable-arm diagnostics; lambda lowering; no-signature function inference; predicative polymorphism (HM unification, call-site instantiation, let-generalization); type-level normalization; import typing for `.zti` and `.zt` modules; **constraints and witnesses** (see below).

### Constraints and Witnesses (v1-adjacent, implemented during v0 cycle)

The `constraint` / `witness` declarations from `docs/v1_spec/03-constraints.md` were built into THIR and the interpreter ahead of schedule because they are needed by the standard library:

- [x] Parsing and HIR name resolution
- [x] THIR lowering: `ThirDeclKind::Constraint` / `Witness`, method signatures, operator methods (binding allocated), default method bodies carried through IR
- [x] Witness type-checking (`check_witnesses`): field sigs matched against constraint method sigs; optional/defaulted methods not required
- [x] Coherence checking (`check_witness_coherence`): duplicate `(Constraint, Type)` pairs rejected
- [x] Monomorphic named-method dispatch in `zutai-eval`: `eq 1 2` resolves to `Eq @Int` witness
- [x] Operator dispatch in `zutai-eval`: `==`, `!=`, `<`, `<=`, `>`, `>=` dispatch to witness fields
- [x] Default-body dispatch: method call falls back to default body when witness omits it
- [x] Polymorphic dispatch (direct calls): `eq x y` inside `<A: Eq>` body resolves via injected `WitnessDict` when the bounded function is called at a concrete type from the top level
- [x] Function type-param bounds recorded in HIR and THIR (`ThirDeclKind::Function.param_bounds`)

**Remaining constraint/witness work** (deferred — each is its own milestone):

- [x] **Dictionary-passing in TLC**: polymorphic dispatch through indirect calls. TLC elaboration threads witness dictionaries as implicit `Lam(dict, …)` parameters and injects them at call sites; named constraint methods and witnessed comparison operators lower to `GetField`/`App` on the selected dict. Completed in TLC Phase 5 and the operator-witness parity follow-up; `eval_tlc.rs` and Dataflow lowering now share this shape for import-free programs.
- [x] **Conditional / higher-kinded witnesses**: `Eq @(List A)` where `A: Eq`. Parametric witnesses lower to `TyLam … Lam(dict) … Record` in TLC, registered as `conditional_witnesses` with their `param_bounds`. Dispatch resolves them structurally: `unify_env` matches a witness target (params as holes) against the concrete operand type — substitution-aware, alias-normalizing, depth-guarded, with `List`/`Optional`/`Tuple`/`Record`/`Union`/`Function` arms — then `resolve_conditional_witness` threads the recursively-resolved component dicts via `App(TyApp(Var(witness), arg), dict)`. Hole bindings keep their `AliasApply` shape so nested parametric aliases (`Pair (Pair Int)`) re-resolve correctly. THIR records `ThirDeclKind::Witness.param_bounds`, type-checks witness fields against instantiated method sigs, flags overlapping conditional witnesses (`ConflictingWitness`) via param-normalized keys, and reports a self-referential target whose bound names the same constraint (`RecursiveWitness`). The eval `type_key` expands parametric `AliasApply` targets (depth-guarded). Resolves at direct and indirect polymorphic call sites without `UnresolvedWitness` (Phase 13).
- [x] **Cross-module witnesses + orphan rule**: `import.rs` / `export.rs` have no constraint/witness handling.
- [x] **`derive` synthesis**: structural witness synthesis for the equality family (`eq`/`==`, `neq`/`!=`). THIR `check_witnesses` rejects derive on non-derivable constraints, requires every structural component to have a same-constraint witness (or be a builtin leaf), and refuses non-equality required methods (`DeriveUnsupportedMethod`). TLC `lower_decl` synthesizes the dict: records fold field equality with `&&`, tuples/unions match on shape, `neq`/`!=` is the negation of structural equality, component witnesses dispatch via `GetField`. Runs on the TLC eval and compile paths (Phase 12).
- [x] **Method-level type params** (`<A,B>` on individual methods): preserved in HIR/THIR and elaborated through TLC with explicit method polymorphism (Phase 14).

Verification gate: `cargo test --workspace` includes spec-shaped parser, HIR, THIR, and semantic facade tests for every v0 chapter. ✅

## Phase 2: TLC (Type Lambda Calculus) ✅

Goal: produce a fully-elaborated, polymorphism-explicit IR from completed THIR. TLC is only produced when THIR type checking succeeds. It is the clean input contract for all compilation stages downstream.

The full TLC design is specified in [`docs/tlc-core.md`](tlc-core.md).

**Done:**

- [x] `crates/general/tlc/` (`zutai-tlc`) exists (~3 700 lines of production + test code).
- [x] TLC IR: `TyLam`/`TyApp`, `VariantT(Row)`, `Singleton(Lit)`, `Variant(label, e)`, kind annotations (`Kind` enum), `Row`/`EffRow`, NbE normalizer with fuel-bounded β-reduction and alias unfolding.
- [x] THIR→TLC lowering: constraint solving, zonking, let-generalization, call-site instantiation (explicit `TyApp` replaces the `instantiation` stub). TLC Phase 0 ("close the live hole") is complete.
- [x] `zutai-semantic` exposes `TlcModule` alongside THIR output.

**Done (TLC sub-roadmap Phases 3–5):**

- [x] **Phase 3 — Row kind + `RVar`**: add `RVar(TlcTypeVar)` to `Row`; add `Row` kind to `Kind`; lower THIR open-record/union row tails to `RVar`; make `subst` capture-avoiding (currently sound only because all type arguments are closed). After this phase, DC will see only flattened closed rows with no `RVar`.
- [x] **Phase 4 — Effect rows**: fully wire `eff` on `Fun`; add the eraser pass that sets `eff = REmpty` before DC emission. Nearly free for v0 (all programs are pure, so the eraser is a no-op); the field already exists in the IR and gives v1 effects a type-level hook at no downstream cost.
- [x] **Phase 5 — Dictionary-passing + eval migration**: elaborate constraint witnesses as implicit `Lam(dict, …)` / `Record` parameters in TLC, eliminating `UnresolvedWitness` for indirect bounded calls. Per Decision 0002 in `docs/tlc-core.md`, this phase triggers migration of `zutai-eval` from THIR to TLC (new `eval_tlc.rs` walker). The THIR walker remains as a regression oracle during the transition.

Verification gate: TLC modules for all v0 spec examples have no free `TypeVar`s, no `RVar` in closed-type position, correct `VariantT`/`Singleton` nodes, correct `TyLam`/`TyApp`/`ForAll` structure, dictionary arguments explicit at every polymorphic call site.

## Phase 3: Dataflow Core ✅

Goal: lower the completed TLC to a Dataflow Core graph where sharing, laziness, and recursion are structurally explicit.

- [x] Add crate `crates/general/dataflow/` (`zutai-dataflow`).
- [x] Implement the `DataflowGraph` IR as specified in `docs/dataflow-core.md`.
- [x] Implement the TLC→DC lowering pass in `zutai-dataflow::lower`:
  - [x] Tree-to-graph conversion: local bindings lowered once; all references share a single `NodeId`.
  - [x] Global-to-`GlobalRef` conversion: top-level references become `GlobalRef` nodes.
  - [x] Recursive definitions: body may produce `GlobalRef` nodes pointing back to the same global (cycles); these are valid and expected.
  - [x] Multi-clause functions: desugar into `Lambda + Match`.
  - [x] Polymorphic functions: `TyLam` → `TyLam` node; call-site `TyApp` → `TyApp` node.
- [x] Implement the DC validation pass (invariant checking in debug builds).

Verification gate: unit tests lower TLC for all v0 language forms and assert correct graph structure (sharing, SCC detection, type consistency). ✅

## Phase 4: ANF Lowering ✅

Goal: convert the Dataflow Core graph into Administrative Normal Form — a linear schedule where every sub-expression is named by a `let` or `letrec` binding.

- [x] Write `docs/anf.md` (design spec, to be done at the start of this phase).
- [x] Add crate `crates/general/anf/` (`zutai-anf`).
- [x] Implement SCC analysis on the global dependency graph.
- [x] Implement a topological sort of SCCs.
- [x] Implement node-to-ANF lowering: one fresh name per non-trivial sub-expression.
- [x] Emit `let` for non-recursive SCCs; emit `letrec` for recursive or mutually-recursive SCCs.

Verification gate: ANF-lowered modules for all v0 forms are well-formed (every name defined before first use; `letrec` only where cycles exist in DC).

## Phase 5: SSA and LLVM IR ✅

Goal: compile ANF to SSA form and emit LLVM IR.

- [x] Add crate `crates/general/ssa/` (`zutai-ssa`).
- [x] Lower ANF functions to basic-block SSA: introduce phi-nodes for branches, eliminate nested lets into straight-line code within blocks.
- [x] Add crate `crates/general/codegen/` (`zutai-codegen`).
- [x] Emit LLVM IR as text (no inkwell/llvm-sys dependency for v0; generates `.ll` files directly).
- [x] Represent v0 values as `i64` (tagged union approach): integers stored directly, compound values (records, tuples, lists, closures, text) heap-allocated with pointer cast to `i64`.
- [x] Map Zutai's structural laziness (unreachable DC nodes = dead code) to LLVM dead-code elimination; do not emit thunk machinery.
- [x] Map `letrec` to LLVM IR functions with mutual direct-call structure.

Verification gate: SSA and codegen crates compile; unit tests cover all v0 language forms (literals, function calls, lambdas, records, tuples, lists, match/if, binary ops, variants, coalesce); `cargo test --workspace` passes (898 tests).

## Phase 6: CLI Compilation ✅

Goal: make `zutai-cli` a usable compiler for `.zt` files.

- [x] Replace the single positional mode with subcommands:
  - [x] `parse <path>` — print AST or parse diagnostics.
  - [x] `check <path>` — run parse, HIR, THIR, and semantic diagnostics (THIR output; no TLC needed).
  - [x] `compile <path> [-o output]` — compile through the full pipeline (semantic → TLC → DC → ANF → SSA → LLVM IR text).
  - [x] `dataflow <path>` — print the Dataflow Core graph (debugging aid).
- [x] Add output rendering for diagnostics with source locations.
- [x] Keep diagnostics source-located through the semantic facade.

Verification gate: CLI integration tests cover successful `.zt` compile, check-only invocation, dataflow output, parse errors, semantic errors, and `-o` flag for output files. `cargo test --workspace` passes (908 tests).

## Phase 7: v1 Parser Frontend (surface syntax only)

Goal: parse the v1 surface constructs in `docs/v1_spec/` into AST/CST. Scope is the parser frontend only — lexer, AST, parser, diagnostics, tests. Lowering these forms through HIR/THIR to type-checked programs is a separate, later effort (HIR/THIR `TypeKind` has no row-variable representation yet, though TLC IR already does via `RVar`).

Already parsed during the v0 cycle (no work needed): constraint declarations, witness declarations, `derive`, bounded type params (`<A: Eq + Show>`), kinded type params (`<F :: Type -> Type>`), parenthesised operator method names. The v1 keywords `select`, `perform`, `handle`, `with`, `resume` are already lexed but not yet parsed into AST nodes.

- [x] **B1 — Ellipsis token + row tails**: lex `...` (`SyntaxKind::DotDotDot`); add anonymous (`...`), named (`...Rest`), and union-spread (`...Shape`) tails to `TypeExpr::Record` / `TypeExpr::Union`; reject row tails that overlap declared fields. Foundation for B2.
- [x] **B2 — `select` projection**: `Expr::Select { receiver, fields }` (value position) and the type-level `select` form (type position); preserve field order; defer unknown-field checks to semantics.
- [x] **B3 — Algebraic-effects surface**: `Expr::Perform`, `Expr::Handle { expr, clauses }` with `with { value = …, op = … }`, `Expr::Resume`; effect-row syntax `! { fail E }` on function types in `TypeExpr`.
- [x] **B4 — Reflection builtins**: confirm `fields` / `schema` parse as ordinary application; add tests; introduce dedicated syntax only if required.

Verification gate: parser tests cover every example in `docs/v1_spec/01-row-polymorphism.md`, `04-metaprogramming.md`, and `05-effects.md`; the v1 keywords already lexed are exercised end-to-end through the AST.

Out of scope (follow-on "v1 semantics" milestone): extend HIR & THIR `TypeKind::Record`/`Union` with row tails; wire THIR→TLC to emit the existing `RVar`; type-check open rows, effects, and `select`.

## Post-v1 Frontend Roadmap

_Updated after Phase 7 completion. The v1 parser frontend is now surface-only complete; the next work is semantic lowering and typed behavior for the already-parsed forms. Do not add more parser surface area until these forms have HIR/THIR/TLC coverage._

### Phase 8: v1 HIR Lowering

Goal: lower parsed v1 syntax into resolved, source-preserving HIR with diagnostics, without type-dependent checking.

- [x] Add HIR representation for record and union row tails: anonymous `...`, named `...Rest`, and spread `...Shape`.
- [x] Resolve row variables from type parameter scopes and distinguish row variables from type aliases used as spreads.
- [x] Add HIR for value/type `select`, preserving selected field order.
- [x] Add HIR for function effect rows, `perform`, `handle ... with { ... }`, and `resume`.
- [x] Diagnose syntax-context errors before THIR: duplicate selected fields, duplicate explicit row fields, invalid row-tail placement, and `resume` outside operation handler clauses.

Verification gate: v1 parser examples lower through HIR with stable source spans and precise diagnostics; no raw parser-only v1 forms leak into semantic entry points.

### Phase 9: Row-Polymorphic THIR

Goal: type-check v1 open records/unions and row-polymorphic APIs.

- [x] Extend THIR `TypeKind::Record` / `TypeKind::Union` with row tails.
- [x] Add row-variable kinding for record rows and union rows.
- [x] Implement first-order row unification for closed rows, anonymous open rows, and named row tails.
- [x] Reject duplicate/overlapping explicit fields and row tails.
- [x] Type-check field access through open record/view types.
- [x] Type-check value-level `select receiver { fields; }` as closed record construction preserving requested order.
- [x] Type-check type-level `select Type { fields; }` after type-level normalization.
- [x] Require explicit annotations when row-polymorphic inference is not principal or obvious.

Verification gate: examples from `docs/v1_spec/01-row-polymorphism.md` parse, lower, and type-check through THIR with expected success/failure diagnostics.

### Phase 10: THIR→TLC Row Elaboration

Goal: elaborate THIR row-polymorphic types into TLC rows using the existing `RVar` and row-kind machinery.

- [x] Lower THIR open records/unions to TLC `Row` values.
- [x] Emit TLC `RVar` for named row tails.
- [x] Extend zonking/substitution coverage for row variables introduced in THIR.
- [x] Ensure closed-type positions contain no unresolved row variables after elaboration.
- [x] Preserve field order for `select` lowering.

Verification gate: semantic facade tests prove v1 row examples produce valid TLC with expected `RVar` use and no free type variables in closed positions.

### Phase 11: `select` Semantics and Compile Support

Goal: make `select` a typed, executable projection form for records and record type values.

- [x] Lower value-level `select` to record projection plus record construction.
- [x] Lower type-level `select` to closed record type construction after normalization.
- [x] Reject unknown selected fields with source-located diagnostics.
- [x] Compile value-level `select` through Dataflow Core, ANF, SSA, and LLVM when the input row is concretely known.

Verification gate: `check`, `run`, and `compile` cover successful selection, field ordering, and unknown-field failures.

### Phase 12: `derive` Synthesis

Goal: replace `derive` no-op witnesses with real structural witness synthesis.

- [x] Reject `Witness { derive: true }` when the constraint is not marked derivable.
- [x] Synthesize record witnesses field-by-field, resolving component witnesses.
- [x] Synthesize union witnesses by member shape and tuple-field comparison.
- [x] Emit synthesized witness bodies before TLC dictionary-passing.
- [x] Fail with precise diagnostics when any component type lacks the required witness.

Verification gate: derived `Eq`/`Ord`-shaped witnesses behave identically to hand-written witnesses in `check`, TLC eval, and compile paths where supported.

### Phase 13: Conditional Witnesses

Goal: support witnesses for parameterized types with bounds, such as `Eq @(List A) :: <A: Eq>`.

- [x] Fix parametric `AliasApply` targets in `type_key`.
- [x] Represent witness predicates with required type-parameter bounds.
- [x] Resolve witnesses recursively through type arguments.
- [x] Detect and report recursive or ambiguous witness search.

Verification gate: bounded witnesses for list-like aliases resolve at direct and indirect polymorphic call sites without `UnresolvedWitness`.

### Phase 14: Method-Level Type Params and Higher-Kinded Constraints

Goal: preserve and elaborate polymorphic constraint methods and constructor-kinded witnesses.

- [x] Preserve method-level type parameters (`<A, B>`) in HIR and THIR.
- [x] Elaborate polymorphic methods to TLC `TyLam` / `TyApp`.
- [x] Extend dictionary-passing to handle polymorphic methods.
- [x] Kind-check constraint targets of kind `Type -> Type`.
- [x] Support partial type application in witness targets, such as `Functor @(Result E)`.

Verification gate: `Functor`/`Foldable`-shaped examples from `docs/v1_spec/03-constraints.md` type-check and elaborate to TLC with explicit method polymorphism.

### Phase 15: Effect Typing (check-only first)

Goal: type-check algebraic effects while refusing execution/compilation until ordering semantics are implemented.

- [x] Represent function effect rows in THIR and TLC.
- [x] Kind and unify effect rows.
- [x] Type-check `perform` against the ambient or locally handled effect row.
- [x] Type-check standard aliases (`fail`, `warn`, `log`, `ask`) and dotted capability operations (`fs.read`).
- [x] Type-check `handle` so handled operations are removed and unhandled operations are forwarded.
- [x] Type-check `resume` result types and enforce the v1 one-shot rule.
- [x] Make `run`/`compile` reject effectful programs with precise unsupported-feature diagnostics until sequencing is designed.
  - Phase 16 later re-pointed `print` to `io.print`; during Phase 15 it intentionally stayed unchanged while effect typing landed first.

Verification gate: `check` accepts/rejects examples from `docs/v1_spec/05-effects.md`; `run` and `compile` refuse effectful programs explicitly rather than miscompiling them.

### Phase 16: Effect Evaluation and Compilation Design

Goal: define and implement explicit ordering for effectful computations without breaking Zutai's lazy pure core.

- [x] Specify forcing and sequencing rules for `perform`, `handle`, `with`, and `resume`.
- [x] Decide whether effects lower through a dedicated IR marker, Dataflow Core extension, or ANF sequencing boundary.
- [x] Implement TLC reference evaluation for handled effects after the ordering model is written.
- [x] Extend compile pipeline only after interpreter behavior is deterministic and test-covered.
- [x] Reintroduce I/O as `perform io.print` over a `Console`/`IO` effect with a handler, then remove or re-point the interim prelude `print` builtin (`zutai_hir::BUILTIN_VALUE_NAMES`).

Verification gate: effect examples run deterministically under the reference interpreter and have matching compiled behavior before LLVM support is claimed.

### Phase 17: Reflection Builtins (`fields` / `schema`)

Goal: implement compile-time reflection over normalized type values.

- [x] Add compiler-known builtins for `fields T` and `schema T` while keeping their surface syntax as ordinary application.
- [x] Implement record reflection first, then union reflection after row/variant representation is stable.
- [x] Define the exact `fields` result shape, including embedded `Type` values.
- [x] Define the serializable `schema` output shape.
- [x] Decide whether open rows are rejected initially or encoded explicitly in schema output.

Verification gate: examples from `docs/v1_spec/04-metaprogramming.md` evaluate to deterministic compile-time values; schema output is ordinary serializable data.

### Backend Support Policy for v1 Features

Each v1 feature must declare one of these support levels before landing:

1. **Check-only** — parser, HIR, THIR, and diagnostics work; `run`/`compile` reject with precise unsupported-feature diagnostics.
2. **Reference interpreter** — TLC evaluation works; compiler backend may still reject.
3. **Full compile support** — Dataflow Core, ANF, SSA, and LLVM emission are implemented and tested.

Recommended initial policy:

- Row polymorphism: full compile support after THIR→TLC normalizes rows to concrete record/union shapes.
- `select`: full compile support early because it lowers to projection plus record construction.
- `derive`: full support after synthesized witness bodies feed existing dictionary-passing.
- Effects: check-only first; interpreter second; LLVM last.
- `fields` / `schema`: compile-time/reference support first; full compile support only when outputs are ordinary values after elaboration.

## Near-Term Implementation Order

- [x] **Finish THIR** — complete (lambda, match, optional access, HM polymorphism, constraints/witnesses).
- [x] **TLC Phase 3** — row kind + `RVar`; capture-avoiding `subst`; open-record/union lowering.
- [x] **TLC Phase 4** — effect-row eraser (v0 is pure; this is mostly mechanical).
- [x] **TLC Phase 5 + eval migration** — dictionary-passing elaboration; add `eval_tlc.rs` as the compiler-path walker. Named-method constraint dispatch and comparison-operator witness dispatch are correct for direct and bounded calls on the THIR oracle, TLC walker, and import-free TLC→Dataflow path.
- [x] **Dataflow Core** — new crate `crates/general/dataflow/`; TLC→DC lowering per `docs/dataflow-core.md` (spec is complete and buildable).
- [x] **ANF lowering** — new crate `crates/general/anf/`; write `docs/anf.md` first; SCC analysis, topological sort, let/letrec introduction.
- [x] **SSA + LLVM IR** — new crates `crates/general/ssa/` and `crates/general/codegen/`; basic-block lowering; LLVM IR text emission (v0 uses i64 universal representation, no inkwell/llvm-sys dependency).
- [x] **CLI `compile` subcommand** — wire the full pipeline; add `check`, `compile [-o output]`, and `dataflow` subcommands with source-located diagnostics.
- [x] **v1 parser frontend** — Phase 7 above.
- [x] **v1 HIR lowering** — Phase 8 above.
- [x] **row-polymorphic THIR** — Phase 9 above.
- [x] **THIR→TLC row elaboration** — Phase 10 above.
- [x] **`select` semantics and compile support** — Phase 11 above.
- [x] **deferred constraint/witness milestones** — `derive` synthesis, conditional witnesses, method-level type params, and higher-kinded constraints; Phases 12–14 above.
- [x] **effect typing and execution model** — check-only first, then interpreter/compiler support after ordering is specified; Phases 15–16 above.
- [x] **reflection builtins** — `fields` / `schema`; Phase 17 above.

## Backlog

These are post-roadmap stabilization items, not unchecked roadmap milestones.

- **v0 spec conformance sweep** — run examples from `docs/v0_spec/` through `run` / `check`, then either fix stale examples or deliberately relax the parser; promote surviving examples into acceptance tests.
- **Diagnostic polish** — render structural details for record-vs-record type mismatches and row-tail errors instead of collapsing both sides to generic `record` labels.
- **TLC-first evaluator cutover plan** — define parity gates for constraints, optionals, imports, effects, and reflection boundaries; keep the THIR evaluator as the regression oracle until the differential suite is green.
