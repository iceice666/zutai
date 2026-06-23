# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

## Scheduled phases

Top-to-bottom is implementation priority. **Track 1** (lettered) continues the
Phase A v1-native-backend-closing series; **Track 2** (numbered) continues the
Phase 30–32 performance series. Per-feature support-level status for each gap is
detailed under "v1 native-backend lowering" and "Deferred to v2/v3" below.

All Track 1 (v1-native-backend) phases — A, B, C, D — have landed (2026-06-25;
see `docs/ARCHIVED.md`), as have Track 2 performance Phases 30–33. The only
remaining scheduled item is **Phase 34** (the GC collector), which stays gated on
a demonstrated GC workload.

### Phase 34 — Conservative mark-sweep GC (Track 2)

- **Gate.** Schedule only once a real GC workload exists (unbounded streams reach
  the backend, or accumulator garbage dominates after Phase 33).
- **Approach.** Zero-ABI **non-moving** mark-sweep as the bridge. A precise
  moving collector (Cheney endgame) needs root-finding over untagged `i64`
  (D-0002) — a shadow stack or stack maps, an ABI change beyond D-0008/D-0009 —
  so it is out of scope here. The lazy-backend path stays rejected
  (strict-plus-TCO is committed).
- **Depends on a demonstrated workload.** Phase 33 (the prerequisite uncurrying
  pass) landed 2026-06-25, so calling-convention churn no longer dominates;
  schedule a collector only once genuine user-data garbage (not call churn) does.

## Near-term hardening

_Both prior near-term items (per-layer forall-lambda typing; differential
value-rendering corpus) landed 2026-06-23 — see `docs/ARCHIVED.md` "Near-term
backend hardening: witness dispatch, open-row gate, corpus". No near-term
hardening items are currently open._

## v1 native-backend lowering

v1 is **semantically complete**: every feature parses, type-checks through
THIR, elaborates to TLC, and runs in the `zutai-eval` reference interpreter
that serves as the differential oracle (`docs/ARCHIVED.md` "Current
baseline"). What is missing for "complete" by the v0 bar — full LLVM/native
lowering plus runtime — is the **backend half of the pipeline** for most v1
features. Track 1 native parity is now **scheduled** as Phases B/C/D under
"Scheduled phases" above — the project no longer defers v1 native parity
wholesale. The ranked detail below is the per-feature support-level reference,
citing the manual's "Implemented extensions beyond v0" table
(`docs/language-manual.md`).

Ranked by remaining work:

1. **Constraints / witnesses / derive (largest).** Dictionary passing already
   reaches the native backend: as of 2026-06-23 a compiled-vs-interpreter
   corpus (`COMPILED_WITNESS_FIXTURES`) confirms native parity for two-method
   sorted-slot dispatch, derived record equality, a conditional `List` witness,
   and a method-level type parameter (the dict field-slot was being dropped
   during effect rewriting — fixed in commit `69e6758`). As of 2026-06-25
   **conditional witnesses with inner field access** also compile and run
   correctly: `Eq @(Pair A) :: { eq = \p q. eq p.fst q.fst; }` was segfaulting
   because all instances of a constraint got the same HIR global name (the
   constraint name), so later instances overwrote earlier ones in
   `globals`; the fix appends `$w{id}` to each witness instance global name
   so every instance coexists. Probing confirmed
   native parity for operator-method witnesses (`==`/`<` direct and bounded),
   structural Show/Ord `derive` rendering, and union `derive` equality too, so
   the prior "zero native support" claim was broadly stale. Genuinely open
   shapes: **higher-kinded instantiation** stays check-only by design — eval and
   compile both refuse HKT execution (`unify.rs` "a refused check is the safe
   direction"), and a type-checking `mapTwice (\x. x) [1; 2;]` is consistently
   rejected at the type-check stage on both paths. **Imported witnesses — concrete
   instances only** — now dispatch natively as of 2026-06-25 (commit `d28bc5d`):
   `WitnessExport::binding_id` lets the CLI compute each dep's DC global name
   (`$dep{idx}${constraint}$w{binding_id}`); the TLC lowerer's extern witness table
   maps `(constraint_name, target_key_str)` to that global; the DC `Var` arm
   emits a `GlobalRef`. Differential tests confirm parity for `Eq @Int`, `Eq @Bool`,
   `Ord @Int` imports. **Conditional cross-module witnesses landed 2026-06-25**
   (Phase B): parametric instances like `Eq @(List A)` / `Eq @(Pair A)` /
   `Eq @(Optional A)` now dispatch natively and in the interpreter — the importer
   structurally matches the dep's exported `WitnessPattern` against the concrete
   call-site type, recovers each parameter, and applies the dep-namespaced witness
   global to recursively-resolved component dicts (see `docs/ARCHIVED.md`). The
   remaining check-only shape is **higher-kinded instantiation** (polymorphic
   passing), rejected at the type-check stage by design.

2. **Row polymorphism (large).** Parser/HIR/THIR/TLC carry row variables and
   the interpreter runs row-typed code as ordinary records/unions. Confirmed
   native today: concrete/closed value-level `select` and field access; open-row
   *passthrough* (a polymorphic function that returns its record argument without
   reading a field by slot); **union extension** (`...Shape` spreading an
   existing union into a new type — both spread-member and new-member tag
   dispatch compile with full parity, per `compiled_union_extension_matches_oracle`).
   Named row-tail *selects* — open-row field reads (`getN :: { n : Int; ...; } ->
   Int = x => x.n`) — **landed 2026-06-25** (Phase C): row-erased
   monomorphization inlines a concrete-argument call as `let x = arg in body`,
   substituting the row variable by the argument's extra fields so the field's
   slot is recomputed for the concrete record (`monomorphize_open_row_selects`).
   The `open_row_select_reason` gate is now reachability-scoped and still rejects
   genuinely-polymorphic open-row selects (a function applied to a still-open
   argument). Open-union matches (a polymorphic match over a `...Rest`-tailed
   union type) **landed 2026-06-25** (Phase D): unlike record selects they are
   tag-dispatched (no slot hazard, no monomorphization needed), so lifting the
   over-restrictive `union_rows_match` rigid-tail check — a closed member pattern
   is a valid case of a rigid open union — is the whole fix; the backend already
   lowered the tag dispatch. With this, the whole row-polymorphism item is
   native-complete.

3. **Residual-effect runtime (medium; partly a non-goal).** Effects a `handle`
   fully discharges are CPS-elaborated to ordinary functions/matches before
   TLC->DC and lower natively today; ambient `io.print` lowers to the runtime
   `HostPrint` path. Effects that escape to the entry boundary other than
   `io.print`, open effect rows, and effectful entry shapes the runtime ABI
   cannot display are **rejected** before Dataflow Core
   (`docs/spec/v1/05-effects.md` "Laziness and Ordering"). A general
   residual-effect ABI is the gap; whether it is in scope is itself a design
   decision — the strict AOT backend may keep rejecting unhandled effects.

4. **Explicit universe-level syntax (small; mostly v2).** THIR/TLC carry
   internal universe levels, but surface level syntax is unimplemented;
   level-polymorphic constructors default to the lowest consistent universe
   and erase before backend lowering (`docs/ARCHIVED.md` "Current baseline").
   Full stratification is the deferred v2 universe-levels design.

**By design, not gaps** — do not file these as missing native work:

- **Reflection** (`fields`/`variants`/`schema`/`witness`) is compile-time:
  `compile`/`dataflow` fold serializable reflection (`schema`, `fields`,
  `variants`, and `witness`-dispatch whose result is a data value) to backend
  constants and reject residual reflection (a raw `witness` dictionary, a
  `Type`-valued result) before lowering. Fold-or-reject is the intended model.
  As of 2026-06-23 this is enforced for all four forms — `variants` (was a
  silent empty-result miscompile) and the `witness C @T` expression (was a
  Dataflow Core ICE) now fold-or-reject through `aot_reflection_program`, so
  reflection is as native as it should be in fact, not just in intent.
- **Rejecting unhandled non-`io.print` effects** at the backend is the
  committed strict-AOT behavior, not an unfinished feature.
- **Annotation-required inference** where row/constraint inference is not
  principal is specified behavior (`docs/spec/v1/01-row-polymorphism.md`
  "Extended Inference"), not a backend gap.

## Deferred to v2/v3

### Garbage collection — deferred behind tail-call optimization

Decision (A), 2026-06-22: the native backend commits to **strict semantics plus
tail-call optimization** (Phase 31), with garbage collection **deferred**. The
compiled heap stays leak-by-default inside the capped thread-local arena
(Phase 30); the cap bounds the blast radius, and finite strict programs
terminate within it.

Empirical basis (measured with `ZUTAI_HEAP_STATS`):

- Before TCO the native stack was the binding constraint — recursion overflowed
  around 10^5–10^6 frames, far below the heap cap (~10^7–10^8 objects). GC could
  not help those programs; TCO could, and now does.
- After TCO, deep tail recursion runs in O(1) stack and the heap becomes the
  binding constraint, so GC is now a meaningful space optimization for
  bounded-live / unbounded-allocation programs (accumulator loops).
- ~2/3 of accumulator allocation is calling-convention overhead (one arg-tuple
  and one closure per curried call), not user data — so **uncurrying** would cut
  more allocation than GC for typical code.

Reordered trajectory (was: leak → mark-sweep → generational copying):

1. **Uncurrying / known-call optimization** (Phase 33) — **landed 2026-06-25**:
   saturated curried calls to a known function collapse to a direct multi-arg
   worker call and the clause arg-tuple is scalar-replaced, removing the per-call
   closure+tuple churn (`docs/ARCHIVED.md`).
2. **Collector** (Phase 34), only once a real GC workload exists (unbounded streams reach
   the backend, or accumulator garbage dominates). Root-finding, not the
   algorithm, is the open gap: untagged `i64` values (D-0002) make conservative
   scanning ambiguous, so a precise moving collector needs a shadow stack or
   stack maps — an ABI change beyond D-0008/D-0009. A conservative non-moving
   mark-sweep is the zero-ABI bridge; Cheney copying is the endgame and stays
   write-barrier-free only while the heap is write-once.
3. A lazy backend (thunk update = mutation = old→young pointers) would force a
   write barrier; that path is **not** taken — strict-plus-TCO is committed.

## Deferred beyond planned v3 work

GADT-style local type equalities and the coercion/cast core node (an explicit
non-goal, `tlc-core.md` §10), impredicative instantiation, unforgeable
capability tokens, and nominal recursive types remain unassigned to the active
v2/v3 roadmap.
