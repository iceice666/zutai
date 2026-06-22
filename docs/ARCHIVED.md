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

_Last updated: 2026-06-22 after closing Phase 24 universe-level foundation._

- Immediate mode parses `.zti` data through selectable parser backends
  (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and
  elaborates complete programs to TLC.
- THIR covers v0 plus implemented v1-adjacent semantics: row-polymorphic
  records/unions, `select`, constraints/witnesses, `derive`, method-level type
  params, higher-kinded constraints, and algebraic-effect typing.
- TLC covers row variables, effect rows, explicit dictionary passing, witnessed
  operator lowering, source effect markers, CPS elaboration for handled effects,
  and runtime `io.print` lowering through ordinary TLC function values.
- THIR and TLC carry internal universe levels for surface `Type`. Explicit level
  syntax is still unsupported; level-polymorphic type constructors default to
  the lowest consistent universe and erase before runtime/backend lowering.
- Dataflow Core, ANF, SSA, and LLVM IR text emission exist and are test-covered.
  Record/tuple access is slot-indexed; union construction now uses dense
  per-union tags; ambient `io.print` lowers to a runtime `HostPrint` path;
  codegen emits static descriptors for `zutai.show`; `@main` renders through the
  type-directed runtime display path and rejects function / `Type` results.
- `compile --emit=llvm|obj|bin` selects LLVM text, object, or native binary
  output. Object/binary modes invoke `llc`/`clang`, link `libzutai_rt`, emit
  actionable diagnostics when the host toolchain is absent, and produce
  PIE-capable Linux binaries without `-no-pie`.
- `zutai-eval` has both the THIR oracle and TLC evaluator. Differential coverage
  includes constraints, optionals, `.zti` imports, `.zt` imports, imported
  functions, transitive imports, imported witness dictionaries, record update,
  config overlay, effects, and reflection/type-value boundaries.
- `print` remains a prelude compatibility binding, but its type is now
  `Text -> Text ! { io.print : Text -> Text }`. TLC lowers the builtin value to
  a runtime-dispatching function; source handlers can intercept `io.print`, and
  the host `run`, `compile`, and `dataflow` paths dispatch ambient `io.print` at
  runtime instead of replaying compile-time captured output.
- `compile` and `dataflow` no longer fold effectful entry programs through the
  evaluator before Dataflow Core. Residual non-`io.print` effect markers and
  unsupported effect rows stay gated by `residual_effect_reason` /
  `zutai_dataflow::try_lower_tlc`; `io.print`-only function rows lower through
  the runtime `HostPrint` path.
- `compile` and `dataflow` still fold renderable compile-time reflection
  programs through the THIR type-value evaluator before Dataflow Core.
  Reflection combined with effectful code remains rejected so AOT reflection does
  not consume host effects at compile time.
- Supported full config-overlay calls lower before Dataflow Core: patch-first
  `overlay`/`overlayDeep` applications with record-literal patch values become
  ordinary record updates, and required nested records merge recursively.
- Unsupported residual overlay forms, optional nested-record deep overlays,
  reflection combined with effectful code, non-`io.print` residual effects,
  unsupported effect rows, function entries, and `Type` entries still reject
  before DC.

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
- CLI native binary coverage includes primitive, record, tuple, union, text,
  atom, and posit entry values; the Linux PIE matrix is verified with
  `llc -relocation-model=pic` and `clang -pie`.

## Open work pointer

Open milestones and unresolved work live in [`TBD.md`](TBD.md). When an item
finishes, move a short status summary into "Completed milestones, newest first"
and keep any unresolved follow-up in `TBD.md`.

## Archived backlog decisions

These closed stabilization items stay here so old risk decisions remain visible.
New unresolved work should become an open milestone/TBD item in `TBD.md`.

- [x] **Compiler entry-type gate cleanup** — CLI `compile` and `dataflow`
  reject final runtime `Type` values before TLC→DC/LLVM lowering, including raw
  `type Int` entries and alias-value entries such as `MyInt :: type Int; MyInt`.
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

### Phase 24: Universe-level foundation ✅

- Internal universe levels now flow through THIR kind checking and TLC kind
  lowering. Surface syntax still exposes only `Type`; explicit level annotations
  remain unsupported.
- Type constructors and higher-kinded constraints are level-polymorphic with
  cumulativity and lowest-consistent defaulting, so ordinary v1 type-level/HKT
  programs remain accepted while `Pair Int Type` checks at a higher inferred
  universe.
- Type-level fuel still bounds normalization only; universe-circular definitions
  produce a dedicated kind diagnostic. Runtime erasure and backend output for
  ordinary value programs remain unchanged.

### Phase 23: Effect CPS lowering and runtime dispatch ✅

- General source effects now lower through handler-passing CPS before Dataflow
  Core. `perform`/`handle`/`resume` and explicit sequence markers are eliminated
  into ordinary TLC `Lam`/`App`/`Case`/`Variant`/`Record`/`GetField`/`Let`/
  `Letrec` structure when the supported handled-effect subset is fully covered.
- The CPS pass supports forwarding unmatched operations to enclosing handlers,
  multiple operations per row, nested handler scopes, direct-return clauses, and
  source handlers for `io.print`. Unsupported residual effects and open or
  unsupported effect rows still fail at the TLC→DC safety gate.
- Ambient `io.print` is no longer evaluated at compile time. Direct,
  higher-order, function-valued, branch-dependent, and sequence-dependent print
  uses lower through the runtime `HostPrint` path across Dataflow Core, ANF, SSA,
  LLVM, and the native runtime ABI.
- The old AOT effect fold and host-print capture plumbing were removed:
  `fold_aot_effects`, `fold_effect_value_to_source`,
  `eval_tlc_analysis_capture_io`, the effect duty of `value_to_source`, and
  `emit_llvm_with_host_prints` / `host_prints` replay. Reflection over effectful
  code remains rejected until reflection folding moves behind runtime effect
  lowering.
- Phase 23 closed with a CLI differential harness that compares every compiled
  effect fixture's stdout (final value render plus ordered print sequence)
  against the `eval_tlc` oracle. The language/runtime docs now distinguish the
  implemented handler-passing CPS form from the deferred free-monad data encoding
  that needs recursive or nominal Dataflow Core types.

### Phase 22: Reflection AOT lowering ✅

- CLI `compile` and `dataflow` remove the reflection-gate exit for renderable
  reflection programs by evaluating `fields`/`schema` at compile time and
  re-lowering the folded backend literal before Dataflow Core.
- `schema` on closed records, payload unions, plain enums, and empty records
  compiles to LLVM/native output and renders the serializable reflection shape.
  Typed empty-list bindings preserve the schema shape when folded values contain
  empty `fields` / `variants` lists.
- Raw `fields` outputs that would render embedded `Type` values reject with the
  existing Type-result compile diagnostic; reflection combined with effectful
  code remains refused rather than dropping host effects.

### Native codegen hardening: PIE-safe executable output ✅

- Linux object emission now uses `llc -filetype=obj -relocation-model=pic`, and
  Linux binary linking requests `clang -pie` instead of `-no-pie`.
- Codegen no longer emits `ptrtoint (ptr @...)` constant expressions for static
  descriptor, text, atom, closure, or `@main` addresses. Static globals use
  pointer-typed LLVM fields, and functions materialize static addresses with
  instruction-form `ptrtoint ptr @... to i64`.
- CLI native binary coverage includes primitive, record, tuple, union, text,
  atom, and posit entry values; the Linux PIE matrix passed in a Linux aarch64
  container with `llc`/`clang`.

### Phase 21: Config-overlay AOT lowering ✅

- `overlay` and `overlayDeep` now use the spec's patch-first order, so
  `defaults |> overlay patch` type-checks and evaluates through both reference
  evaluators.
- THIR→TLC lowers supported full applications with record-literal patch values to
  ordinary `RecordUpdate` expressions before Dataflow Core. `overlayDeep`
  recursively lowers required nested-record patches; unsupported residual overlay
  forms and optional nested-record deep overlays remain backend-gated rather than
  producing partial native semantics.
- CLI tests cover `check`/`run`, LLVM/dataflow record-update lowering, native
  shallow and deep overlay binaries, and static unknown-field/type-mismatch
  patch diagnostics.

### Phase 20: Effects AOT lowering (superseded by Phase 23) ✅

- This milestone was the old closed-entry bridge before Phase 23. CLI
  `compile`/`dataflow` attempted a pre-DC fold for closed executable programs
  with fully handled effects by running the TLC semantics oracle on a 256 MiB
  worker stack, serializing the forced backend value to pure source, and lowering
  that pure TLC to Dataflow Core.
- Captured `io.print` host output was replayed in generated `@main` through the
  existing `zutai.text_from_global` / `zutai.print_text` runtime ABI; native
  binaries for `print "hello"` printed `hello` before rendering the final
  `"hello"` value.
- Residual/unfoldable effects, effectful function entries, and non-backend
  values remained rejected before Dataflow Core. Direct
  `zutai_dataflow::try_lower_tlc` still gated raw residual
  `TlcExpr::{Perform,Handle,Resume}` and non-empty function rows.
- Phase 23 superseded this approach: the AOT fold and host-print capture path are
  deleted, and supported effects now compile through runtime CPS/`HostPrint`
  lowering.

### Phase 18: Runtime and ABI ✅

- Runtime/codegen now use dense per-union variant tags in construction and type
  descriptors, with `Optional`/`Maybe` fixed at absent/none = 0 and
  present/some = 1.
- Codegen emits static descriptors from `DfTy`, including field/tag name
  strings, and `@main` calls `zutai.show` plus a trailing newline. Function and
  `Type` entry values are rejected with precise compile diagnostics.
- `compile --emit=llvm|obj|bin` writes LLVM text, assembles objects with `llc`,
  and links native binaries with `clang` against `libzutai_rt`. Missing host
  tools produce actionable diagnostics; object/binary tests skip when the
  toolchain is absent.
- v0 keeps the leak arena. Object headers still reserve high bits for future
  precise/generational GC layout IDs, and runtime descriptors provide the
  pointer-shape bridge for that later collector.

### Phase 19: Effect AOT boundary ✅

- Host authority beyond `io.print` is explicit capability passing only:
  filesystem, network, environment, time, and randomness stay out of the
  ambient prelude and must be represented by capability values plus effect rows.
- This boundary was later refined by Phase 23. General source effects still do
  not enter DC as `Perform`/`Handle`/`Resume`, but ambient `io.print` now has a
  narrow runtime `HostPrint` path across DC/ANF/SSA/LLVM. Unsupported residual
  effects and open/unsupported effect rows still reject before Dataflow Core;
  `try_lower_tlc` keeps the same no-silent-erasure safety role for direct
  library callers.

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
- Backend support now includes Phase 20 closed-entry effect folding; residual
  effectful functions and unfoldable effect values remain backend rejections.

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

