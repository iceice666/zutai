# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

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
features. None of this is currently scheduled; the project has deliberately
settled v1 at check-plus-interpreter and deferred native parity. Support
levels below cite the manual's "Implemented extensions beyond v0" table
(`docs/language-manual.md`).

Ranked by remaining work:

1. **Constraints / witnesses / derive (largest).** Dictionary passing already
   reaches the native backend: as of 2026-06-23 a compiled-vs-interpreter
   corpus (`COMPILED_WITNESS_FIXTURES`) confirms native parity for two-method
   sorted-slot dispatch, derived record equality, a conditional `List` witness,
   and a method-level type parameter (the dict field-slot was being dropped
   during effect rewriting — fixed in commit `69e6758`). Probing since confirmed
   native parity for operator-method witnesses (`==`/`<` direct and bounded),
   structural Show/Ord `derive` rendering, and union `derive` equality too, so
   the prior "zero native support" claim was broadly stale. Genuinely open
   shapes: **higher-kinded instantiation** stays check-only by design — eval and
   compile both refuse HKT execution (`unify.rs` "a refused check is the safe
   direction"), and a type-checking `mapTwice (\x. x) [1; 2;]` is consistently
   rejected at the type-check stage on both paths. **Imported witnesses used as
   invoked methods** is the real remaining gap: `.zt`/`.zti` imports without
   witness exports now compile natively via one-arena Dataflow Core merge (Phase
   A — see `docs/ARCHIVED.md`); modules whose imports export typeclass witnesses
   are rejected before DC by the witness gate (`IMPORT_WITNESS_REASON`), so
   there is no silent miscompile. Completing native witnesses means teaching the
   one-arena merge to thread imported witness dictionaries into the importing
   module's dispatch, then adding a backend HKT-dispatch story.

2. **Row polymorphism (large).** Parser/HIR/THIR/TLC carry row variables and
   the interpreter runs row-typed code as ordinary records/unions. Confirmed
   native today: concrete/closed value-level `select` and field access; open-row
   *passthrough* (a polymorphic function that returns its record argument without
   reading a field by slot); **union extension** (`...Shape` spreading an
   existing union into a new type — both spread-member and new-member tag
   dispatch compile with full parity, per `compiled_union_extension_matches_oracle`).
   Named row-tail *selects* — open-row field reads (`getHost :: <Rest> { host :
   Text; ...Rest; } -> Text = x => x.host`) — are **rejected** before Dataflow
   Core (`open_row_select_reason`, commit `b9012d6`) because the field's runtime
   slot depends on hidden tail fields the slot-based ABI cannot recover; that
   previously miscompiled silently. Open unions (a polymorphic match over a
   `...Rest`-tailed union type) remain check-plus-interpreter only: the
   type-checker rejects the open-union match expression on both `run` and
   `compile` paths so there is no backend parity gap. Completing this item means
   a sound open-row select lowering (row-erased specialization or runtime
   field-offset descriptors) plus a backend compile-test corpus for row-typed
   programs; until then the gate keeps the backend honest.

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

1. **Uncurrying / known-call optimization** — collapse saturated curried calls
   to direct multi-arg calls, removing the per-call closure+tuple churn.
2. **Collector**, only once a real GC workload exists (unbounded streams reach
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
