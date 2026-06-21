# Zutai Archive

This document tracks implemented status, validation notes, archived decisions,
and completed milestones. Open milestones and TBD items live in
[`TBD.md`](TBD.md). Language design still comes from `docs/v0_spec/` for v0 and
`docs/v1_spec/` for deferred v1 features.

## Compilation pipeline

```text
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

- **THIR** is the error-tolerant, source-preserving typed IR and the output of
  `check`.
- **TLC** is produced only after successful type checking. It has explicit
  polymorphism (`TyLam`/`TyApp`) and resolved inference variables.
- **Dataflow Core → ANF → SSA → LLVM** is the production AOT pipeline. Laziness
  and sharing are structural in Dataflow Core, not runtime thunks.
- **`zutai-eval`** is the reference semantics oracle. The default `run`/`repl`
  path is TLC-first for executable value programs; THIR remains the regression
  oracle and runtime `Type`/reflection boundary.

Design details: [`docs/tlc-core.md`](tlc-core.md),
[`docs/dataflow-core.md`](dataflow-core.md), [`docs/anf.md`](anf.md), and
[`docs/runtime-abi.md`](runtime-abi.md).

## Current baseline

_Last updated: 2026-06-21 after the Phase 19 effect backend representation decision._

- Immediate mode parses `.zti` data through selectable parser backends
  (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and
  elaborates complete programs to TLC.
- THIR covers v0 plus implemented v1-adjacent semantics: row-polymorphic
  records/unions, `select`, constraints/witnesses, `derive`, method-level type
  params, higher-kinded constraints, and algebraic-effect typing.
- TLC covers row variables, effect rows, explicit dictionary passing, witnessed
  operator lowering, source effect markers, reflection boundaries, and the
  TLC-first evaluator input contract.
- Dataflow Core, ANF, SSA, and LLVM IR text emission exist and are test-covered.
  Record construction, selection, update, tuple-pattern binding, and variant
  payload binding now use the slot-indexed runtime ABI. Native binary
  linking/execution remains [Phase 18 TBD](TBD.md#phase-18-runtime-and-abi) work.
- `zutai-eval` has both the THIR oracle and TLC evaluator. Differential coverage
  includes constraints, optionals, `.zti` imports, `.zt` imports, imported
  functions, transitive imports, imported witness dictionaries, record update,
  config overlay, effects, and reflection/type-value boundaries.
- `print` remains a prelude compatibility binding, but its type is now
  `Text -> Text ! { io.print : Text -> Text }`. The TLC evaluator represents it
  as `io.print`; source handlers can intercept it, and the host `run` boundary
  handles residual `io.print`.
- `compile` and `dataflow` deliberately reject config overlay builtins,
  reflection builtins, residual effect markers, and non-empty function effect
  rows until each has its backend lowering. Phase 19 fixes the effect lowering
  representation as pre-DC free-monad/CPS elaboration; implementation remains
  open.

## Validation notes

- Optional value syntax remains `T? = Optional T` with `#none` / `#some (v)`.
  Optional field access preserves physical presence as `Maybe T` with `#absent`
  / `#present (v)`, so `field? : T?` yields `Maybe (Optional T)`. `?.` works on
  both `Optional` and `Maybe`; `??` unwraps exactly one layer.
- v0 docs use parser-accepted typed bindings (`name :: Type = value`) and
  semicolon-terminated record/tagged patterns. Fixtures pin stale syntax
  rejections.
- `Int??` lexes as `Int` + `??`, not a double optional. Write `(Int?)?` for a
  nested optional.

## TBD

Open milestones and unresolved work live in [`TBD.md`](TBD.md). When a TBD item
finishes, move a short status summary into "Completed milestones, newest first"
and keep any unresolved follow-up in `TBD.md`.

## Archived backlog decisions

These closed stabilization items stay here so old risk decisions remain visible.
New unresolved work should become an open milestone/TBD item in `TBD.md`.

- [x] **v0 spec conformance sweep** — code fences from `docs/v0_spec/` are
  extracted and routed through `check`/`run` for `.zt` survivors and the immediate
  parser for `.zti` survivors; stable survivors are promoted to acceptance tests.
- [x] **Diagnostic polish** — record-vs-record type mismatches render source-like
  record shapes, including optional fields and row tails; row-tail spread
  overlaps report the spread source and existing/incoming shapes.
- [x] **TLC-first evaluator cutover** — default evaluation runs through TLC for
  executable value programs; THIR remains the explicit regression oracle and
  runtime `Type`/reflection boundary.

## Completed milestones, newest first

### Phase 19: Effect backend representation ✅

- Algebraic effects will lower before Dataflow Core through a free-monad/CPS
  encoding over existing TLC/DC constructs (`Variant`, `Record`, `Lam`, `App`,
  `Match`, and `Letrec`), not through explicit effect nodes in DC or ANF.
- Dataflow Core, ANF, SSA, and LLVM remain pure. Function effect rows may be
  erased only after the elaboration eliminates residual `Perform`/`Handle`/
  `Resume`; current compile/dataflow gates remain correct until that lowering
  lands.
- Host capability scope and the lowering implementation remain open Phase 19
  work.

### Phase 18 D-0004: Slot-indexed records ✅

- TLC→DC resolves canonical record slots by lexicographically sorted field names;
  witnessed-method dictionary access records its slot during THIR→TLC lowering.
- DC/ANF/SSA pass integer slots to LLVM. `Select`, `RecordUpdate`, record/tuple
  patterns, and variant payload binding no longer use field-name hashes.
- Runtime support uses the existing slot-keyed `zutai-rt` helpers. D-0004 is
  verified by IR-text slot assertions plus `zutai-rt` record round-trip tests;
  native binary parity remains Phase 18 D-0010 work.

### Phase 17: Reflection builtins (`fields` / `schema`) ✅

- `fields T` and `schema T` parse as ordinary applications to compiler-known
  builtins.
- Record and union reflection produce deterministic runtime type values / schema
  output through the THIR type-value evaluator.
- Open rows are handled explicitly at the reflection boundary.
- Compile/dataflow reject reflection builtins until their outputs are lowered to
  ordinary backend values.

### Phase 16: Effect evaluation and ordering model ✅

- `docs/v1_spec/05-effects.md` now specifies sequencing for `perform`, `handle`,
  operation clauses, `resume`, and sequence expressions.
- TLC evaluation supports handled effects with delimited continuations.
- `print` was re-pointed to `io.print`; host `run` handles residual `io.print`.
- Backend support intentionally stops at residual-effect gates. LLVM support is
  tracked separately in [Phase 19 TBD](TBD.md#phase-19-tbd-effect-lowering-past-tlc).

### Phase 15: Effect typing ✅

- Function effect rows are represented in THIR and TLC.
- Effect rows are kinded and unified.
- `perform` is checked against the ambient or locally handled effect row.
- `handle` removes handled operations and forwards unhandled operations.
- `resume` result types and one-shot usage are checked.
- `run`/`compile` originally rejected all effectful programs; Phase 16 later
  enabled TLC `run` while keeping backend rejection.

### Phase 14: Method-level type params and higher-kinded constraints ✅

- Method-level type parameters are preserved in HIR/THIR and elaborated to TLC
  `TyLam`/`TyApp`.
- Dictionary passing handles polymorphic methods.
- Constraint targets of kind `Type -> Type` are kind-checked.
- Partial type application in witness targets works, e.g. `Functor @(Result E)`.

### Phase 13: Conditional witnesses ✅

- Parametric witness targets such as `Eq @(List A) :: <A: Eq>` resolve
  recursively through type arguments.
- Witness search normalizes aliases, handles nested parametric aliases, and
  reports recursive or ambiguous search.

### Phase 12: `derive` synthesis ✅

- `derive` synthesizes structural equality-family witnesses for records, tuples,
  and unions.
- Non-derivable constraints and unsupported required methods are rejected.
- Synthesized witness dictionaries feed the existing TLC dictionary-passing path.

### Phase 11: `select` semantics and compile support ✅

- Value-level `select` lowers to record projection plus record construction.
- Type-level `select` lowers to closed record type construction after
  normalization.
- Unknown selected fields are rejected with source-located diagnostics.
- Concrete value-level `select` compiles through Dataflow Core, ANF, SSA, and
  LLVM IR text.

### Phase 10: THIR→TLC row elaboration ✅

- THIR open records/unions lower to TLC rows.
- Named row tails lower to TLC `RVar`.
- Zonking/substitution covers row variables.
- Closed-type positions contain no unresolved row variables after elaboration.

### Phase 9: Row-polymorphic THIR ✅

- THIR records and unions carry row tails.
- Row-variable kinding and first-order row unification support closed rows,
  anonymous open rows, and named row tails.
- Field access through open record/view types is checked.
- Non-principal row-polymorphic inference requires explicit annotations.

### Phase 8: v1 HIR lowering ✅

- HIR represents record/union row tails, value/type `select`, function effect
  rows, `perform`, `handle`, and `resume`.
- Row variables resolve from type-parameter scopes and are distinguished from row
  spread aliases.
- Syntax-context diagnostics catch duplicate selected fields, duplicate explicit
  row fields, invalid row-tail placement, and `resume` outside operation handler
  clauses.

### Phase 7: v1 parser frontend ✅

- Parser covers ellipsis row tails, value/type `select`, algebraic-effect
  surface syntax, and reflection builtin applications.
- Existing v1-adjacent constructs from the v0 cycle include constraints,
  witnesses, `derive`, bounded/kinded type params, and operator method names.

### Phase 6: CLI compilation ✅

- CLI subcommands: `parse`, `check`, `run`, `repl`, `compile`, and `dataflow`.
- `compile` runs semantic → TLC → DC → ANF → SSA → LLVM IR text.
- Diagnostics remain source-located through the semantic facade.

### Phase 5: SSA and LLVM IR ✅

- `crates/general/ssa/` and `crates/general/codegen/` exist.
- ANF lowers to basic-block SSA with phi nodes.
- Codegen emits LLVM IR text using an `i64` universal value representation for
  v0 and external posit helper declarations.

### Phase 4: ANF lowering ✅

- `docs/anf.md` and `crates/general/anf/` exist.
- Dataflow Core lowers through SCC analysis, topological sorting, and `let` /
  `letrec` introduction.

### Phase 3: Dataflow Core ✅

- `crates/general/dataflow/` exists.
- TLC lowers to a graph where locals are shared, globals are `GlobalRef`s, and
  recursion is explicit.
- Validation checks graph invariants in debug builds.

### Phase 2: TLC ✅

- `crates/general/tlc/` exists.
- TLC IR covers `TyLam`/`TyApp`, rows (`RVar`), singletons, variants, kinds,
  effect rows, NbE normalization, dictionary-passing elaboration, and witnessed
  comparison-operator lowering.
- `zutai-semantic` exposes `TlcModule` for complete analyses.

### Phase 1: THIR / LSP foundation ✅

- Parser, HIR, and THIR cover v0 syntax and source-located diagnostics.
- Implemented forms include optional access/defaulting, tuple/record patterns,
  match exhaustiveness, lambda lowering, no-signature function inference,
  predicative polymorphism, imports, constraints, witnesses, and operator
  witness dispatch.

