# Zutai Archive

This document tracks implemented status, validation notes, archived decisions,
and completed milestones. Open milestones and TBD items live in
[`TBD.md`](TBD.md). Language design still comes from `docs/spec/v0/` for v0 and
`docs/spec/v1/` for deferred v1 features.

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

_Last updated: 2026-06-23 (language specs, Unicode XID, evaluator/backend hardening) and
2026-06-24 (Phase A: `.zt`/`.zti` native module-import lowering)._

- Immediate mode parses `.zti` data through selectable parser backends
  (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and
  elaborates complete programs to TLC.
- THIR covers v0 plus implemented v1/v2/v3-adjacent semantics:
  row-polymorphic records/unions, `select`, constraints/witnesses, `derive`,
  method-level type params, higher-kinded constraints, algebraic-effect typing,
  higher-rank annotation checking, predicative inference, guarded recursive type
  aliases, stream-backed generator sugar, and standard host capability/effect-row
  checking.
- TLC covers row variables, effect rows, explicit dictionary passing, witnessed
  operator lowering, source effect markers, higher-rank `ForAll`/`TyLam`/`TyApp`
  elaboration, CPS elaboration for handled effects, equirecursive alias identity,
  runtime `io.print` lowering through ordinary TLC function values, and residual
  host-effect grant gating before Dataflow Core.
  Constraint-attached derive recipes are stored through Syntax/HIR/THIR and
  drive specialized TLC Show/Ord dictionary synthesis; `witness C @T` reflects
  resolvable dictionaries using the same concrete/conditional lookup as implicit
  method dispatch. Runtime type reflection includes `fields`, `variants`, and
  `schema` views.
- THIR and TLC carry internal universe levels for surface `Type`. Explicit level
  syntax is still unsupported; level-polymorphic type constructors default to
  the lowest consistent universe and erase before runtime/backend lowering.
- Dataflow Core, ANF, SSA, and LLVM IR text emission exist and are test-covered.
  Record/tuple access is slot-indexed; union construction now uses dense
  per-union tags; ambient `io.print` lowers to a runtime `HostPrint` path;
  granted v2 host operations lower to explicit `HostOp` nodes through
  Dataflow/ANF/SSA/LLVM/runtime; recursive and generic recursive aliases lower
  to finite cyclic `DfTyId` graphs; codegen emits static descriptors for
  `zutai.show`; `@main` renders through the type-directed runtime display path
  and rejects function / `Type` results. **`.zti` data imports and `.zt` pure
  value/function imports compile natively** via one-arena Dataflow Core merge
  (Phase A): imported modules are lowered into the same graph under a `$dep{idx}$`
  namespace prefix; the root references the dep's module-value global
  (`$dep{idx}$$value`). Modules that export typeclass witnesses are rejected before
  DC by the witness gate (cross-module witness dispatch is still interpreter-only).
- `compile --emit=llvm|obj|bin` selects LLVM text, object, or native binary
  output. Object/binary modes invoke `llc`/`clang`, link `libzutai_rt`, emit
  actionable diagnostics when the host toolchain is absent, and produce
  PIE-capable Linux binaries without `-no-pie`.
- `zutai-eval` has both the THIR oracle and TLC evaluator. Differential coverage
  includes constraints, optionals, `.zti` imports, `.zt` imports, imported
  functions, transitive imports, imported witness dictionaries, record update,
  config overlay, effects, reflection/type-value boundaries, polymorphic
  curried helpers, repeated nested destructures, and name-sorted record display.
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
- [x] **v0 spec conformance sweep** — code fences from `docs/spec/v0/` are
  extracted and routed through `check`/`run` for `.zt` survivors and the immediate
  parser for `.zti` survivors; stable survivors are promoted to acceptance tests.
- [x] **Diagnostic polish** — record-vs-record type mismatches render source-like
  record shapes, including optional fields and row tails; row-tail spread
  overlaps report the spread source and existing/incoming shapes.
- [x] **TLC-first evaluator cutover** — default evaluation runs through TLC for
  executable value programs; THIR remains the explicit regression oracle and
  runtime `Type`/reflection boundary.

## Completed milestones, newest first

### Track B: Host-capability entry boundary ✅

_Completed 2026-06-24. Implements the v2 spec §"Entry Boundary"
(`docs/v2_spec/02-host-capabilities.md`): a program may declare the host
capabilities it needs as its entry parameter and the host supplies them. The six
standard host ops already ran end-to-end via direct `perform` (the CLI grants
`HostEffectSet::ALL`); the only gap was the idiomatic `main :: { caps } -> Result`
shape, rejected as "entry returns a function" with no way to obtain a capability
value._

- **Mechanism.** A TLC pass `apply_entry_capabilities`
  (`crates/general/tlc/src/entry.rs`), run in `lower_thir` so the interpreter
  (`run`) and native (`compile`) paths share it, applies the entry to synthesized
  advisory capability tokens while its leading parameter is capability-shaped (an
  `Opaque` capability type, or a closed record of them), iterating so curried
  `FsRead -> Env -> R` parameters are all supplied. The entry then has the
  `Result` type the backend renders, and its granted host effects lower to the
  existing `HostOp`/`HostPrint` path.
- **Advisory tokens.** Capability values are never inspected (authority is the
  effect row, not the value), so each token is a `0` literal stamped with the
  capability's opaque type. DC `Lit` validation is a no-op, so the literal kind
  need not match the opaque type; codegen emits an `i64 0` the program never
  reads. A non-capability parameter stops the supply, leaving a genuinely
  function-valued entry rejected as before.
- **`main` symbol fix.** Escaped the reserved C entry symbol in codegen `mangle`:
  a user binding named `main` mangled verbatim to `@main`, redefining
  `define i32 @main`. `$` cannot occur in a source identifier (UAX #31), so the
  rename `main` → `main$user` is collision-free with both source names and the
  `$dep…` witness scheme. A pre-existing latent bug, surfaced because
  function-named entries used to be rejected before code generation.
- **Validation.** Run-vs-compile parity tests for single, record, and curried
  capability entries (reading a temp file through the boundary); plus a
  user-function-named-`main` regression and a non-capability-entry rejection
  test. 1567 workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Phase A: Cross-function effect handler-passing ✅

_Completed 2026-06-24. Closes a run-vs-compile parity gap in algebraic-effect
lowering: a `perform` reached only through a call to a separate effectful
function — handled at the call site — previously ran in the interpreter but was
refused by the native backend ("algebraic effects remain after TLC lowering")._

- **Mechanism (inline-specialization).** A pre-pass in `lower_thir`
  (`crates/general/tlc/src/lower/effects/inline.rs`) beta-reduces fully-saturated
  direct calls to monomorphic, non-recursive, effectful top-level functions into
  their call sites (`f a` → `let p = a in body`, capture-avoiding via binder
  freshening; arguments `let`-bound, never substituted, in left-to-right order).
  The relocated `perform` becomes lexically enclosed by the call site's handler,
  and the existing trusted lexical CPS elaborator discharges it. Sound because
  the interpreter resolves handlers dynamically at perform time (closures carry
  no handler stack), so a directly-applied static lambda inlined at its call
  site reproduces the exact call-time handler stack.
- **CPS over `Case`.** `cps` now handles `Case` by reifying the post-case
  continuation as a join lambda each arm invokes once — needed because pattern
  and curried lambdas lower to `Lam(scrut, Case(…))` / tuple-pattern `Case`s, and
  it also repairs a pre-existing eligibility/`cps` mismatch (eligibility accepted
  a `Case`-bodied handle expression but `cps` left its `perform` residual).
- **Gate + DCE.** Inlined-away effectful callees are dead-code-eliminated from
  `module.decls` (fixpoint), and the residual-effect gate's effectful-function-
  type clause is scoped to types reachable from `reachable_exprs` so an
  inlined-away callee's orphaned `Fun(…!{e})` type (la_arena never frees nodes)
  no longer falsely rejects. This also closed a latent parity bug: an *unused*
  effectful top-level declaration now compiles (matching the interpreter) instead
  of being rejected for merely existing.
- **Stays gated (refused, never miscompiled).** Recursive/mutually-recursive
  effectful callees (SCC guard), polymorphic (`TyLam`) and higher-order effectful
  values, partial applications, and effects escaping the entry boundary.
- **Validation.** Differential `COMPILED_EFFECT_FIXTURES` (single-arg, curried,
  resuming, arg-effect ordering, chained, two-call-site, unused-decl) confirm
  run==compile parity; gate-rejection tests confirm recursion and higher-order
  refuse. Gate stack: 1562 workspace tests pass; `cargo fmt` and `cargo clippy`
  clean.

### Phase D: Open-union match lowering ✅

_Completed 2026-06-24. Closes the Track 1 Phase D item in `TBD.md` and the
open-union half of the v1 row-polymorphism gap. A polymorphic match over a
`<Rest>`-tailed open union now type-checks and compiles with parity, completing
native row polymorphism._

- **Finding.** An anonymous `...` open-union match (`RowTail::Open`) already
  type-checked, ran, and compiled (union matches are tag-dispatched). Only the
  named `<Rest>` form (`RowTail::Param`) failed — at type-checking, with
  "type mismatch: expected union, found #dev". Unlike record selects (Phase C),
  union dispatch is by tag, not by slot, so there is no slot hazard and no
  monomorphization is needed.
- **Fix** (`crates/general/thir/src/lower/types/match_.rs`, `union_rows_match`):
  the rigid `RowTail::Param(p)` case required the found tail to equal the row
  variable (`ft == RowTail::Param(p)`), rejecting a closed member pattern
  (`#dev`, tail `Closed`). It now also accepts a closed found whose members are
  all explicit members of the rigid open union (`extras.is_empty() && (ft ==
  Closed || ft == Param(p))`) — such a value/pattern is a valid case of the
  union; a different rigid tail or an open/flexible found stays rejected as not
  provably covered. The backend's existing tag dispatch lowers the match
  unchanged. Exhaustiveness is unaffected: a `<Rest>` match still needs a
  wildcard (the rigid tail has unknown members).
- Tests: `compiled_open_union_rest_match_matches_oracle` and
  `open_union_rest_match_without_wildcard_is_non_exhaustive` (cli),
  `rest_tailed_union_match_typechecks` (thir). Gate stack: 1560 workspace tests
  pass; `cargo fmt` and `cargo clippy` clean.

### Phase C: Open-row select lowering ✅

_Completed 2026-06-24. Closes the Track 1 Phase C item in `TBD.md` and the
open-row-select half of the v1 row-polymorphism gap. Open-row field reads
(`getN :: { n : Int; ...; } -> Int = x => x.n`) now compile natively with
parity; the field's runtime slot is recomputed for each concrete record._

- **The bug.** A record field's slot is its name-sorted rank, so it depends on
  the sibling fields. An open-row parameter `{ n; ... }` views `n` at slot 0, but
  a concrete `{ extra; n }` puts `n` at slot 1 — the slot-based backend would read
  the wrong field. Previously gated before Dataflow Core.
- **Row-erased monomorphization** (`crates/general/tlc/src/monomorphize.rs`,
  `monomorphize_open_row_selects`, run from the CLI on the backend module before
  Dataflow Core): a concrete-argument call to an open-row-selecting function is
  inlined to `let param = arg in clone(body)`, substituting the parameter's row
  variable by the argument's *extra* fields throughout the cloned body's types.
  The inlined field selects then see the concrete record type and DC computes the
  correct slot. The concrete field set is read from the argument expression (a
  record literal names its fields; the call-site *type* is the open parameter
  view and cannot be used). The cloned body's top type is overridden with the
  call's result type (a clause `Case` body is recorded with the surrounding
  function type — tolerated in lambda-body position, but it would otherwise become
  the entry type). Fully-inlined declarations are dropped (reachability fixpoint).
- **Gate** (`open_row_select_reason`, `crates/general/dataflow/src/lib.rs`) is now
  reachability-scoped (`reachable_exprs`) — the arena retains inlined-away exprs,
  so an arena-wide scan would falsely reject. Genuinely-polymorphic open-row
  selects (a function applied to a still-open argument) stay gated by design; the
  interpreter, which resolves fields by name, runs them and is the oracle.
- Tests (`crates/cli/tests/cli.rs`): `compile_open_row_select_lowers_to_llvm`,
  `compile_bin_open_row_select_matches_oracle`,
  `compile_open_row_select_discriminates_slot_per_concrete_record`,
  `compile_unspecializable_open_row_select_stays_gated`; flipped
  `open_row_select_lowers_after_monomorphization` and the host-grant variant in
  `crates/general/dataflow/src/tests.rs`. Gate stack: 1556 workspace tests pass;
  `cargo fmt` and `cargo clippy` clean.

### Phase 33: Uncurrying / known-call optimization ✅

_Completed 2026-06-24. Closes the Track 2 Phase 33 item in `TBD.md` and the
uncurrying prerequisite of the deferred GC trajectory. On accumulator loops the
calling-convention churn — one closure + one arg-tuple per curried call — is
eliminated entirely; values are unchanged._

- **New SSA op `CallKnown { func, args, tail }`** (`crates/general/ssa/src/lib.rs`):
  a direct multi-argument call to a named worker, emitted by codegen as
  `[musttail] call i64 @func(args…)` (`crates/general/codegen/src/instr.rs`).
- **Uncurrying pass** (`crates/general/ssa/src/uncurry.rs`, run from `lower_anf`
  after lowering, before TCO): from the ANF module it recovers each top-level
  curried function's arity and fully-applied body (peeling nested lambdas, past
  erased `TyLam`s), generates a multi-parameter worker (`$uncurried`) by
  re-lowering that body with every argument as a direct SSA parameter, and
  rewrites every *saturated known-call chain* — a root `ApplyClosure` whose
  closure resolves to a known function (directly or through a materialized
  `Alias { GlobalClosure }`), followed by single-use result→closure links
  totalling `arity` (links need not be consecutive — ANF computes the next
  argument between applications) — into one `CallKnown` of the worker, deleting
  the now-dead intermediate applications. The original curried function is kept
  for value-use and partial application.
- **Tuple scalar-replacement** (`scalar_replace_tuples`): the multi-parameter
  clause `n acc => …` desugars to `match (n, acc) { … }`, building an arg-tuple
  each call; a tuple used only as the base of constant-slot `Select`s is replaced
  by direct aliases to its elements and dropped, removing the surviving
  per-call arg-tuple inside the worker.
- **TCO** (`crates/general/ssa/src/tco.rs`): a return-position `CallKnown` is
  marked `musttail` when its argument count equals the caller's parameter count
  (matching all-`i64` signature), so a self-recursive worker loops in O(1) stack.
- Measured on `HEAP_STRESS_SRC` (`sum 4000 0`): `tuple 4001 → 0`,
  `closure/raw 4001 → 0` (12003 → 4001 heap objects, ~2/3 reduction); result
  unchanged (`8002000`). Regressions:
  `compile_emit_bin_uncurried_accumulator_drops_call_churn` (cli),
  `uncurrying_collapses_saturated_recursive_call` (ssa). Gate stack: 1554
  workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Phase B: Conditional cross-module witnesses ✅

_Completed 2026-06-24. Closes the "Conditional cross-module witnesses" gap of the
v1 native-backend constraints/witnesses item in `TBD.md`. Imported parametric
witnesses (`Eq @(List A)`, `Eq @(Pair A)`, `Eq @(Optional A)`) now dispatch on
both the native backend and the `eval_tlc` interpreter, with differential parity._

- **Structural witness pattern.** A new arena-independent
  `zutai_thir::WitnessPattern` (`crates/general/thir/src/witness_pattern.rs`,
  `export_witness_pattern`) captures a conditional witness target with parameter
  holes. `WitnessExport.conditional` (`crates/general/semantic/src/import.rs`)
  carries the pattern plus per-parameter component-constraint names across the
  import boundary.
- **Native backend.** The CLI gate is narrowed: a parametric witness export with
  a buildable pattern is allowed; only a non-matchable target (e.g. higher-kinded)
  still routes to the interpreter (`extern_witness_tables`,
  `crates/cli/src/commands/mod.rs`). The TLC root lowerer matches the pattern
  against the concrete call-site THIR type (`match_witness_pattern`, mirroring
  `unify_env`), recovers each parameter, and emits the dep-namespaced witness
  global (`$dep{idx}${constraint}$w{binding_id}`) applied via `TyApp`/`App` to the
  recursively-resolved component dicts (`try_extern_conditional_witness`,
  `crates/general/tlc/src/lower/witness_resolve.rs`). Component dicts resolve by
  constraint *name* against the extern tables, so a component constraint the
  importer never declares (`Eq @(List A) :: <A: Show>`) still resolves.
- **Interpreter.** `eval_tlc` collects each dep's conditional witness function
  values and concrete dicts, then instantiates an imported conditional witness on
  demand for every concrete dispatch key the root needs (string-template match of
  the pattern against the `structural_witness_key` dispatch key, then applies the
  witness function to recursively-materialized component dicts), registering the
  result into `operator_witnesses` so the existing `imported_method` dispatch finds
  it (`crates/general/eval/src/tlc_entry.rs`).
- **Two latent bugs fixed.** (1) `alloc_virtual_binding` counted *downward* from
  `len+1`, so the third-plus virtual extern-witness global collided with a real
  `BindingId` (a `GlobalRef("ys")` got the placeholder empty-record type) — now
  counts upward, above the real range; also hardens the concrete-witness path for
  3+ imported witnesses at one site. (2) `structural_witness_key_subst` substituted
  only a top-level type variable, so a nested applied alias (`List (Pair Int)`)
  keyed as `[{fst:@N,...}]`; the key walk now threads the substitution env through
  every position (`structural_witness_key_env`), keying it `[{fst:Int,snd:Int}]`.
- Differential parity tests (`crates/cli/tests/cli.rs`):
  `compile_zt_imported_conditional_pair_witness_matches_oracle`,
  `…_list_…`, `…_nested_…`, `…_optional_…`, `…_cross_constraint_component_…`,
  `…_digit_suffix_record_…`. Gate stack: 1552 workspace tests pass; `cargo fmt`
  and `cargo clippy` clean.

### Interpreter oracle consistency: record equality + imported-method dispatch ✅

_Completed 2026-06-24. Two latent interpreter (TLC-evaluator) correctness bugs found
while probing cross-module conditional witnesses; both fixed independently of the
still-open Phase B native conditional dispatch._

- **Record equality was nondeterministic.** `Value`'s `PartialEq` (the default
  `run`/TLC path) sorted record fields by the field-name string's POINTER ADDRESS
  (`n.as_ref() as *const str`), so two records' independently-allocated name `Rc<str>`s
  sorted in unrelated orders and the zipped comparison mismatched fields. `{fst=1;snd=2}
  == {fst=1;snd=2}` flipped `true`/`false` across runs (ASLR/alloc-order entropy). Fixed
  to sort by name CONTENT (`crates/general/eval/src/value.rs`), matching the THIR oracle
  `values_equal`. Regression: `record_equality_is_order_independent`.
- **Imported-method dispatch was type-unaware.** `imported_method_by_name` resolved an
  imported witness method by NAME only, ignoring the operand type: two same-method
  concrete instances (`Eq @Int` + `Eq @Bool`) were ambiguous → refused (`UnboundBinding`),
  and an imported conditional witness silently ran the wrong instance. The `GetField`
  node carries only the *generic* method scheme, so the concrete operand type is recorded
  at lowering in a new `TlcModule::dict_dispatch_keys` side table (operand `target_key`
  per constraint-method `GetField`, carried through effect elaboration's `alloc_like`).
  `imported_method` now dispatches type-directed: matching instance → correct value;
  no match (including parametric/conditional or abstract operand) → refuse, never a
  type-mismatched by-name pick. Fixes multi-instance concrete dispatch; makes imported
  conditional witnesses refuse cleanly instead of silently miscomputing. Regressions:
  `dispatch_imported_type_directed_witness_selection` (eval),
  `compile_zt_imported_multi_instance_witness_matches_oracle` (cli differential).
- Gate stack: 1546 workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Phase A: Module-import native lowering (.zti + .zt) + witness naming fix ✅

_Completed 2026-06-24. Closes the "Module imports" sub-item of the v1 native-backend
constraints/witnesses item in `TBD.md`; also fixes a conditional-witness dispatch segfault._

- **Phase A.a** (`05fa320`): `.zti` data imports lower inline to Dataflow Core
  constants. `ImportEnv { zti }` threaded through `Lowerer`; CLI builds the map
  from `analysis.import_values`.
- **Phase A.b + A.c**: `.zt` value/function imports — plus transitive and diamond
  chains — compile natively via one-arena Dataflow Core merge.
  - `ImportEnv` replaced by `ImportTarget` (Zti/Zt) + `ModuleInput` + `ProgramInput`.
  - `try_lower_tlc_with_host_grants_and_imports` → `try_lower_program_with_host_grants`.
  - `Lowerer` extended with `enter_module` (clears per-module `BindingId`/`TlcTypeId`
    caches; preserves shared arenas) and `lower_dep_module` / `lower_root_module`.
  - Dependency globals namespaced under `$dep{idx}$`; dep module-value synthetic
    global `$dep{idx}$$value`; collision-free under `mangle`.
  - CLI `collect_dep_analyses` DFS post-order; `build_module_imports` keyed by
    `Rc` pointer (not source string) for correct diamond dedup.
  - Witness gate: modules that export typeclass witnesses are rejected before DC
    (`IMPORT_WITNESS_REASON`); cross-module witness dispatch is still interpreter-only.
- **Witness instance naming** (`5b6d90d`): `Eq @Int` and `Eq @(Pair A)` both got HIR
  binding name `"Eq"`, causing DC to overwrite the concrete dict with the conditional
  TyLam. The conditional dispatch then passed the TyLam (a closure) as the concrete
  `Eq_A_dict`, leading to a `GetField` on a closure at runtime — segfault. Fixed in
  `collect_globals` by appending `$w{binding_id}` to `TopWitness` global names,
  making every witness instance unique. Adds `conditional_pair_witness` to
  `COMPILED_WITNESS_FIXTURES` (differential coverage for field-access inside conditional).
- Gate stack: 1540 workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Near-term backend hardening: witness dispatch, open-row gate, corpus ✅

_Completed 2026-06-23. Closes both "Near-term hardening" TBD items and advances
the v1 native-backend constraints/witnesses and row-polymorphism items._

- **Per-layer forall-lambda typing** (`crates/general/tlc/src/lower/expr.rs`).
  `lower_lambda` wrapped every TyLam/dict-`Lam` layer and the value-parameter
  peel with the lambda's full `outer_ty`. For a lambda checked against a rank-2
  annotation (`apply (\x. x)` where `apply :: (<A> A -> A) -> Int`) `outer_ty`
  is a `ForAll`, so the value-parameter peel never advanced and every value
  `Lam` was typed `ForAll`; the Dataflow Core structural validator (which
  requires a `Lam` type to be `Fun`) aborted with an ICE while the interpreter
  ran the same program. Now the forall/dict prefix is peeled one
  quantifier/arrow per layer, mirroring `lower/decl.rs`. Confirmed live, not
  just defensive. Commit `accf422`.
- **LLVM string quote escaping** (`crates/general/codegen/src/descriptors.rs`).
  `llvm_string_bytes` emitted a double quote as `\"` inside a `c"..."`
  constant, which closed the literal early and made `llc` reject the constant
  with a length mismatch. Any compiled program with a quote in a rendered text
  value failed to assemble. Emit the `\22` hex escape. Commit `3efa936`.
- **Differential value-rendering corpus** (`crates/cli/tests/cli.rs`). Expanded
  `COMPILED_SHOW_FIXTURES` to cover non-alphabetical records (flat/nested),
  user-union variants, nested tuples, text escaping, and negative integers so a
  compiled-vs-interpreter rendering divergence fails a test. Commit `a654c1a`.
- **Witness dict field-slot preservation** (`crates/general/tlc/src/lower/effects/rewrite.rs`).
  `elaborate_effects` rebuilds expressions through `alloc_like`, which copied
  `expr_types` and `spans` but not `dict_field_slots`. A `GetField` selecting a
  witness method by its sorted dictionary slot lost the slot during effect
  rewriting and the backend fell back to slot 0, dispatching the wrong method:
  `lt 1 2` against an `Ord` witness (sorted slots `gt=0`, `lt=1`) compiled to
  `gt` and returned `false` where the interpreter returned `true`. `alloc_like`
  now propagates `dict_field_slots`. A new `COMPILED_WITNESS_FIXTURES` corpus
  confirms native parity for two-method sorted-slot dispatch, derived record
  equality, a conditional list witness, and a method-level type parameter —
  evidence the prior "zero native support" note for witnesses was stale for
  these shapes. Commits `69e6758`, `608f5f1`.
- **Open-row field-select backend gate** (`crates/general/dataflow/src/lib.rs`,
  `crates/cli/src/commands/mod.rs`). Selecting a field from an open record type
  silently miscompiled: a parameter typed `{ n : Int; ...; }` hides its tail
  fields, so the slot computed from the view type disagreed with the concrete
  runtime layout (`getN { extra = 7; n = 5; }` returned 7 natively vs 5 in the
  interpreter). The slot-based record ABI carries no field names or offsets, so
  open-row select cannot lower soundly without row-erased specialization or
  runtime descriptors. `open_row_select_reason` now gates value-record
  `GetField` on open rows out of Dataflow Core in both `try_lower_tlc` paths,
  and the CLI surfaces it as a clean compile error. Dict-method selects and
  closed records are unaffected; the interpreter still evaluates open rows.
  Commits `b9012d6`, `347d82d`.
- **`variants`/`witness` reflection fold-or-reject** (`crates/general/semantic/src/lib.rs`,
  `crates/cli/src/commands/mod.rs`). The compile-time reflection gate only
  detected `fields`/`schema`, so `variants` reflection silently miscompiled to
  an empty result and the `witness C @T` reflection expression (a dedicated
  `WitnessReflect` THIR node) panicked the Dataflow Core structural validator
  with a `TypeMismatch` ICE. `reflection_builtin_program` also routes the
  run-time evaluator to the THIR oracle, which cannot dispatch through a witness
  dict, so widening it would have regressed `witness`/`variants` evaluation.
  Added `aot_reflection_program` (a superset covering `fields`/`schema`/`variants`/`witness`)
  used only by the `compile`/`dataflow` fold-or-reject gate; the routing detector
  stays at `fields`/`schema`, keeping `witness`/`variants` fold evaluation on the
  TLC path. Now `variants (Color)` and `(witness Show @Point).show p` fold to the
  interpreter's value, and a bare non-serializable `witness` dictionary is
  rejected cleanly instead of crashing the compiler. Commits `3033284`, `14aff1b`.
- **Module-import backend gate** (`crates/general/dataflow/src/lib.rs`). A
  compiled program that imports a `.zt`/`.zti` module crashed at runtime: TLC→DC
  lowers `TlcExpr::Import` to `DfNodeKind::Import`, which ANF turns into an
  `AnfExpr::Error` leaf, and the imported module is never lowered or linked.
  Programs segfaulted, and an imported operator witness silently dispatched to
  the builtin operator (compiled `1 == 1` against an imported
  `Eq @Int { (==) = \a b. false; }` returned true vs the interpreter's false).
  `import_reason` now gates any module containing an import out of Dataflow Core
  in both `try_lower_tlc` paths; THIR already rejected bare-binding imports as
  "unsupported feature: imports", and this catches record-valued imports that
  slipped past. Cross-module backend linking remains unimplemented; the
  interpreter still resolves imports. Commits `5a6d070`, `aad96ea`.
- Verification: `cargo fmt`; `cargo clippy --workspace --all-targets` (clean);
  `cargo test --workspace` (all pass). Touched-code coverage exceeds 85%; the
  only unhit added line is a defensive `else continue` guard in
  `open_row_select_reason`.

### Documentation spec tree merge ✅

- Merged the v0 and v1 language specifications under `docs/spec/` as
  `docs/spec/v0/` and `docs/spec/v1/`, with `docs/spec/README.md` as the
  versioned specification entry point.
- Updated repository docs, roadmap/archive links, v2 cross-links, the v0 spec
  conformance test fixture paths, and the local `zutai-language` skill routing
  to the new spec paths.
- Recent implementation docs now reflect Unicode XID names, canonical
  interpreter/backend record display, nested destructure SSA hardening,
  per-layer curried lambda typing, TLC evaluator tail-call trampolining, and the
  v1 native-backend backlog.

### Unicode identifiers, atoms, and field names ✅

- General mode (`.zt`) and immediate mode (`.zti`) now accept Unicode UAX #31
  XID names for binding identifiers, field names, and atom bodies, with `_`
  accepted explicitly and atom bodies additionally accepting `-`. Names compare
  by raw Unicode scalar sequence with no normalization. The standard and
  SIMD/NEON immediate parsers share the same behavior; general-mode parsing,
  lossless tokenization, diagnostics lookaheads, type checking, evaluation, and
  the CLI `run` path all cover Unicode bindings such as `café ::= 42`.

### Phase 32: TLC evaluator tail-call trampoline ✅

- The reference TLC evaluator (`crates/general/eval/src/eval_tlc`) is the
  default semantics oracle for executable value programs. It walks expressions
  in continuation-passing style and previously recursed on the host stack for
  every tail call, so deep tail recursion overflowed even the CLI's 256 MiB
  worker stack near depth ~6000 — far short of the backend, which now compiles
  the same recursion to constant-stack `musttail` calls (Phase 31).
- Added an `EvalControl::Tail { ev, id, env, resume }` variant and a `settle`
  driver loop that bounces tail positions instead of recursing, reusing the
  same trampoline shape the evaluator already used for algebraic-effect
  `Perform`/`resume`. `eval_control` is now a thin driver over a renamed
  `eval_step`; the eight tail positions (`TyLam`/`TyApp` bodies, `Let`/`Letrec`
  bodies, closure application, and both `eval_case` branch bodies) emit a `Tail`
  rather than a recursive `eval_control` call. Sub-expression evaluation (call
  arguments, scrutinees, operands) still recurses, bounded by expression nesting
  not recursion depth.
- Every site that matches on an `EvalControl` (`bind_rc`, `finish_top`,
  `handle_control`) settles it first, so a `Tail` never escapes into a matcher;
  effect semantics are unchanged.
- Effect: the default evaluator runs tail-recursive programs in constant
  host-stack space (`sum 1000000` now evaluates to `500000500000`, matching the
  compiled binary), so the differential oracle keeps pace with the backend at
  depth. Non-tail recursion is still O(depth) by nature, as in the backend. The
  secondary THIR walker (`eval`, used only for runtime `Type`/reflection
  programs) is a direct recursive tree-walker and retains the host-stack limit;
  it is exercised only at modest depth.
- Verification: `cargo fmt --check`; `cargo test --workspace` (1504 passed,
  including two new constant-stack regression tests at depth 100_000 on the
  default test-thread stack); `cargo clippy --workspace --all-targets`; manual
  differential check — interpreter and compiled `sum 1000000` agree.

### Phase 31: Backend tail-call optimization (`musttail`) ✅

- New SSA pass `crates/general/ssa/src/tco.rs`, run by `lower_anf`. **Return
  sinking** collapses tail-position `phi`-then-`return` join blocks into direct
  returns from each predecessor — an always-safe CFG cleanup applied to a
  fixpoint so nested tail matches peel from the outside in. **Tail marking**
  then flags any return-position `ApplyClosure` (last instruction whose result
  the block returns).
- Codegen emits marked calls as LLVM `musttail call`, which guarantees
  constant-stack tail recursion regardless of `llc` opt level. Added
  `tail: bool` to `SsaOp::ApplyClosure`.
- Marking is gated to two-parameter closure-code functions (`i64(i64, i64)`,
  matching the indirect callee type). The zero-parameter entry/thunks keep a
  plain `call`, because `musttail` requires matching caller/callee parameter
  lists — one extra stack frame, not unbounded depth.
- Effect: deep tail recursion that previously overflowed the native stack
  (`loop 5000000`, curried `sum 1000000 0`) now runs in O(1) stack, so the
  binding constraint flips from the native stack to the heap ceiling. This is
  the decision-(A) commitment: strict backend plus TCO, GC deferred (see
  `docs/runtime-abi.md` and `TBD.md`).
- The THIR reference interpreter still recurses on the host stack (no TCO), so
  the compiled backend now reaches greater depth than `run`; differential
  fixtures stay at modest depth (e.g. `sum 4000`).
- Verification: `cargo fmt --check`; `cargo test --workspace` (1502 passed);
  `cargo clippy --workspace --all-targets`; manual e2e — `loop 5000000` and
  `sum 1000000 0` exit 0 with correct values; a tight `ZUTAI_HEAP_MAX` now
  yields `heap limit exceeded` instead of SIGSEGV.

### Phase 30: Runtime heap ceiling and allocation telemetry ✅

- Replaced the global `LazyLock`/`Mutex` bump arena with a `thread_local`
  `Arena` of 1 MiB, 16-byte-aligned `Box<[u128]>` chunks: no hot-path lock, and
  per-thread arenas keep the `rlib` sound under a multi-threaded host.
- Heap ceiling (default 2 GiB; `ZUTAI_HEAP_MAX` accepts `k`/`m`/`g`/`0`/
  `unlimited`/`none`): allocating past the cap prints `zutai runtime error:
  heap limit exceeded …` and exits 1 instead of leaking until the OS OOM-kills
  the process. `nil` is now a process-static 16-byte-aligned object.
- `ZUTAI_HEAP_STATS=1` registers an `atexit` dump reporting total bytes and
  objects, average size, peak committed, the cap, and per-kind counts
  (record/tuple/cons/variant/text/closure-or-raw) from always-on
  relaxed-atomic counters.
- Documented in `docs/runtime-abi.md` (D-0008, "Memory model: thread-local
  bump arena, capped leak-by-default").
- Verification: arena unit tests; subprocess tests
  `crates/general/runtime/tests/heap_cap.rs` and `heap_stats.rs`; CLI e2e
  `compile_emit_bin_heap_stats_dump_reports_allocations`.

### Phase 29: Stream-backed generator syntax ✅

- Added contextual generator syntax `stream { yield expr; ... }`. It parses only
  when a `stream` block starts with `yield`, preserving ordinary `stream`
  identifier usage and record application, including `yield` as a record field.
- `Expr::Generator` is source-preserving at syntax/display/span boundaries and
  desugars during HIR lowering to the existing lazy list representation. No
  second effect system or iterator IR was introduced.
- `Stream A` is accepted as a standard one-argument type constructor and
  currently lowers transparently to `List A`; THIR type application, alias
  normalization, and universe-level computation all share that treatment.
- Generator body effects use the existing expression/list effect machinery:
  resource-backed examples require the same capability/effect-row declarations
  as ordinary expressions, and unsupported residual host operations still reject.
- Verification: `cargo fmt --check`; `cargo test --workspace` (1432 passed);
  `cargo clippy --workspace --all-targets`; `cargo llvm-cov nextest --workspace`
  (function coverage 87.91%; line coverage 81.16%).

### Phase 28: Derive recipes and witness reflection ✅

- Constraint declarations can carry `derive = <T> => ...` recipe bodies through
  Syntax, HIR, and THIR; recipe expressions are type-checked before TLC consumes
  the recipe marker.
- `witness C @T` is parsed, typed as a method-record dictionary, lowers to TLC
  dictionary resolution, and reports `WitnessReflectNotInScope` for unresolved
  dictionaries while accepting conditional witnesses such as `Eq @(List A)` for
  `witness Eq @(List Int)`.
- Type-value reflection now includes `variants` alongside `fields` and `schema`,
  returning union variant names and payload-field metadata with recursive
  `Type` back-references preserved as runtime type values.
- Built-in structural equality remains the default derive path. Constraint
  recipes synthesize specialized Show and lexicographic Ord witnesses for records
  and unions, including same-variant payload ordering and derived-dictionary
  reflection.
- Verification: `cargo fmt --check`; `cargo test --workspace` (1423 passed);
  `cargo clippy --workspace --all-targets`; `cargo llvm-cov nextest --workspace`
  (function coverage 87.89%; line coverage 81.11%).

### Phase 27: Host capabilities beyond ambient `io.print` ✅

- Standard host capability type names are seeded in the root scope:
  `FsRead`, `FsWrite`, `Env`, `Clock`, `Rng`, and explicit `IoPrint`. `Path`
  and `Instant` are accepted as standard text-shaped host boundary types.
- THIR effect rows recognize standard operations `fs.read`, `fs.write`,
  `env.get`, `clock.now`, and `rng.next`; capability values remain ordinary
  parameters and authority is advisory only.
- TLC keeps residual host effects explicit and rejects ungranted operations by
  default before TLC→DC lowering. CLI `run`, `dataflow`, and native/LLVM compile
  boundaries grant the standard host set and lower granted residual effects.
- Dataflow adds `HostOp`, ANF/SSA/codegen preserve it, and the runtime/evaluator
  dispatch filesystem read/write, environment lookup, clock, and deterministic
  RNG helpers. Ambient `io.print` remains source-compatible, and source handlers
  can still intercept host operations before the boundary.
- Verification: `cargo fmt --check`; `cargo test --workspace`; `cargo clippy
  --workspace --all-targets`; `cargo llvm-cov nextest --workspace` (function
  coverage 88.17%; line coverage 81.29%).

### Phase 26: Higher-rank polymorphism ✅

- Type syntax extended with nested quantifiers in annotation positions:
  `(<A> A -> A) -> R` and constrained `(<A: Show> A -> Text)`. Parser, HIR,
  THIR, and TLC all carry the `ForAll` node.
- Bidirectional checking pushes written higher-rank annotations into lambda and
  function arguments; inference remains predicative and rank-1.
- ForAll in structural non-argument positions (record fields, union variants,
  list element types, tuple items) rejects with an
  `UnsupportedFeature("impredicative type")` diagnostic.
- TLC elaboration adds explicit `TyLam`/`TyApp` at quantification points and
  `App` for constraint dictionaries at each higher-rank call site.
- `applyId` and constrained `showBoth` examples type-check and run through THIR
  and TLC.

### Phase 25: Recursive type aliases and equirecursive equality ✅

- Guarded recursive and mutually recursive aliases now check through THIR/TLC:
  recursive occurrences under records/unions carry alias identity instead of
  eager expansion, while bare/non-productive cycles still report alias-cycle or
  fuel diagnostics.
- Generic recursive aliases such as `Tree A` pre-register constructor arity,
  preserve universe levels through recursive alias applications, and compare via
  scoped equirecursive type matching without stale fixpoint state or variance
  shortcuts.
- Dataflow Core instantiates generic recursive aliases into finite cyclic
  `DfTyId` graphs; validation remains enabled in debug builds, LLVM descriptor
  emission gets finite back-references, and `check`, `run`, `dataflow`, and
  `compile --emit llvm` cover recursive `Tree`, mutual `Expr`/`Args`, generic
  `Tree A`, and structural equality examples.

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

- `docs/spec/v1/05-effects.md` now specifies sequencing for `perform`, `handle`,
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

