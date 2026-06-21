# Zutai Open Work

Implementation order: top to bottom. Each phase is additive and independently
testable, mirroring the archived milestone style. State support levels precisely.

## Phase 23: Effect CPS lowering (replace the AOT effect fold)

**Goal.** Compile algebraic effects through a genuine pre-DC elaboration so
effectful programs lower to native code whose effects fire **at runtime**, in
program order, with real host dispatch — instead of being interpreted away at
compile time and replayed as baked constants. Dataflow Core, ANF, SSA, and LLVM
stay pure: the elaboration removes every `Perform`/`Handle`/`Resume`/`Sequence`
marker before TLC→DC, so `residual_effect_reason`
(`crates/general/tlc/src/lib.rs`) returns `None` because the nodes are genuinely
gone, not pre-evaluated.

### Decision: CPS / handler-passing, not the free-monad data form

`docs/tlc-core.md` §9 specifies the encoding as "free-monad / CPS-style" as if
the two were interchangeable. They are not, at the **type** level:

- The free-monad form needs a self-referential type
  `Free Op A = { pure: A } | { impure: Op }` where `Op` carries
  `resume: R -> Free Op A`. Dataflow Core has no recursive / nominal / `Mu` type
  node: `DfTy::Union(Vec<DfUnionVariant>)` is a finite structural tree, and
  `lower_type` (`crates/general/dataflow/src/lower.rs`) memoizes a type id only
  *after* recursing into its members, so a self-referential `VariantT`
  stack-overflows. `Free Op A` is structurally infinite (the perform-spine
  length is a runtime quantity) and cannot be unrolled.
- The CPS / handler-passing form keeps recursion in the handler interpreter
  `letrec` + closures, which Dataflow Core already supports via `GlobalRef`
  back-edges → SCC → `letrec` (`docs/dataflow-core.md` §"Recursion"). With a
  fixed answer type per `handle` scope the answer type is not recursive, so CPS
  lands with **no new DC type node**.

CPS is therefore the lower-risk first cut, and it mirrors the structure of the
reference interpreter we already trust (see "Oracle" below). The free-monad data
form is deferred behind a DC recursive-type node (recorded at the end).

### Oracle: the elaboration is a defunctionalization, not a new design

`crates/general/eval/src/eval_tlc/effects.rs` is already a delimited-continuation
CPS interpreter: `EvalControl::{Value, Perform { op, arg, cont }}`,
`handle_control`, `bind_control` / `bind_rc`, `apply_value_clause`, `finish_top`,
with `EvalCont = Rc<dyn Fn(Value) -> Result<EvalControl>>`. The elaboration
reifies this exact machine as core terms. Every elaborated program MUST evaluate
to the same value and the same ordered effect sequence as the TLC oracle; that
oracle is the differential check for the whole phase.

### Current support level (to be replaced)

- `compile`/`dataflow` accept only **closed, fully evaluable** effectful entry
  programs, via the AOT fold (`fold_aot_effects` → `fold_effect_value_to_source`
  → `eval_tlc_analysis_capture_io` + `value_to_source`, on a 256 MiB worker
  thread, `crates/cli/src/commands.rs`).
- Effects are evaluated at **compile time**; `io.print` output is frozen and
  replayed as `@zutai.effect.print.N` constants in `@main`
  (`emit_llvm_with_host_prints` / `emit_main`, `crates/general/codegen/src/lib.rs`).
- Effectful **function** values, runtime-dependent effect ordering, and any
  capability beyond `io.print` are uncompilable: `value_to_source` returns
  `None` for closures/builtins, so only first-order data round-trips.
- `erase_effects` (`crates/general/tlc/src/erase.rs`) exists but is a v0 no-op
  exercised only by `tlc` unit tests; it is **not** on any pipeline path.

### Target support level

- Effectful programs — including effectful function values and handlers whose
  control flow depends on runtime data — compile through DC→ANF→SSA→LLVM with no
  compile-time evaluation.
- Ambient `io.print` is dispatched by a **runtime driver**, not baked.
- `residual_effect_reason` / `try_lower_tlc` stay as the no-erasure safety net;
  they pass naturally once the elaboration runs.

### 23.1 — CPS effect elaboration pass (TLC→TLC)

- New module `crates/general/tlc/src/lower/effects.rs`; entry
  `TlcModule::elaborate_effects(&mut self)`, run from the `lower_thir` pipeline
  before `erase_effects`.
- Selective CPS: pure subterms stay direct-style; subterms that are effectful
  (type carries a non-`REmpty` `Fun` row, or contain
  `Perform`/`Handle`/`Resume`/`Sequence`) are CPS-translated so the rest of the
  computation up to the nearest enclosing `handle` becomes the reified `resume`
  continuation, an ordinary `Lam`.
- `perform op arg` → evaluate `arg`, then call the in-scope handler for `op`
  with the payload and the captured `resume` (mirrors the `handle_control`
  matched arm). `Sequence` → left-to-right monadic-bind chain (mirrors
  `bind_rc`). `handle expr with { value; ops }` → install handlers over the CPS
  translation of `expr`, with `value` defaulting to identity.
- Rewrite effectful `Fun(A, B, eff)` value and annotation types to their pure
  CPS-translated form; run `erase_effects` last to drop any residual row.
- Output contains only `Lam`/`App`/`Case`/`Variant`/`Record`/`GetField`/`Let`/
  `Letrec`; no effect marker node remains.
- Test: a single-op closed `handle … with { value; op }` + `resume` program
  elaborates to effect-free TLC and lowers through `lower_tlc` without the gate
  firing; the differential value matches the oracle.

### 23.2 — Forwarding, multi-op, nested handlers

- Unmatched-op re-injection: a handler removes the ops it names and forwards the
  rest (mirror the else branch of `handle_control`).
- Multiple operations per effect row; nested `handle` scopes; handler clauses
  that return directly vs. `resume` exactly once (the one-shot invariant is
  already typed in Phase 15).
- Test: a `parseOrDefault`-style forwarding program (`v1_spec/05-effects.md`),
  nested handlers, and a handler that returns without resuming — all differential
  against the oracle for value and effect sequence.

### 23.3 — Runtime effect driver (ABI + codegen)

- Replace constant replay with a runtime driver that dispatches residual ambient
  operations at runtime. The compiled entry threads the CPS computation; the
  runtime analogue of `finish_top` services `io.print` through the existing
  `zutai.text_from_global` / `zutai.print_text` ABI
  (`crates/general/runtime/src/lib.rs`).
- Codegen emits the driver in `@main` instead of baked `@zutai.effect.print.N`
  constants (`emit_main`, `crates/general/codegen/src/lib.rs`).
- This is what lets effectful functions exist as ordinary compiled values and
  lets effects execute at runtime in program order, rather than only closed
  entries folded at compile time. (Function / `Type` *entry* results stay
  rejected by `unsupported_entry_type_reason` — an orthogonal existing limit.)
- Test: native binary for `print "hello"` prints `hello` at runtime (parity with
  the current baked output), plus a program whose print order depends on a
  match/branch taken at runtime.

### 23.4 — Cutover: remove the AOT effect fold

- Delete `fold_aot_effects`, `fold_effect_value_to_source`, the effect duty of
  `value_to_source`, the 256 MiB effect worker, the `host_prints` parameter of
  `emit_llvm_with_host_prints` / `emit_main`, and `host_prints` plumbing through
  `run_compile` / `run_dataflow` (`crates/cli/src/commands.rs`,
  `crates/general/codegen/src/lib.rs`).
- Keep `residual_effect_reason` / `try_lower_tlc` as the safety net (defends
  against a future upstream change emitting an effect node the pass misses).
- Revisit the reflection+effects rejection (`commands.rs`): reflection still
  folds AOT (Phase 22), so decide whether reflection over effectful code stays
  rejected or runs reflection folding after effect elaboration.

### 23.5 — Differential verification + docs

- Differential harness in `zutai-eval` / CLI tests: every effect fixture's
  compiled output (value render + ordered print sequence) equals the `eval_tlc`
  oracle.
- Docs: rewrite `docs/tlc-core.md` §9 to specify the implemented CPS form and
  the deferred free-monad form; update `docs/dataflow-core.md` "Effect boundary",
  `docs/runtime-abi.md` (driver loop + op dispatch), and
  `docs/v1_spec/05-effects.md` (replace the "AOT support level is precise
  rejection" note). Move the superseded Phase 20 effects-AOT-fold summary into
  `docs/ARCHIVED.md` marked superseded and add the Phase 23 summary.

### Verification gate

`cargo fmt`, `cargo test --workspace`, and `cargo clippy --workspace
--all-targets` green throughout. Native effect binaries verified where the host
toolchain (`llc` / `clang` / `libzutai_rt`) is present, skipped with a diagnostic
otherwise.

### Deferred (recorded, not Phase 23): free-monad data form

The literal `docs/tlc-core.md` §9 free-monad encoding needs a Dataflow Core
recursive-type representation — a `Mu` / nominal type node, or knot-tying a
placeholder `DfTyId` in `lower_type` before recursing into members. Worth doing
only if a reified, optimizable effect tree is wanted; CPS (Phase 23) covers v1
effect semantics without it.

## Out of scope by v1 spec

Deliberately not milestones: host capabilities beyond `io.print`
(filesystem/network/environment/clock/randomness, reserved as non-ambient in
`v1_spec/05-effects.md`), user-defined `derive` recipes
(`v1_spec/03-constraints.md` marks post-v1), and universe-level enforcement
(`v1_spec/02-type-level-computation.md` states it as a "should"; type-level
evaluation is fuel-bounded so `Type : Type` is not a runtime-soundness risk).
