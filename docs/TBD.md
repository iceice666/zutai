# Zutai Open Work

Open work is grouped by deferral horizon. Completed milestones and their
implementation detail live in `docs/ARCHIVED.md`; language design lives in
`docs/spec/v0/` (stable), `docs/spec/v1/`, `docs/v2_spec/`, and `docs/v3_spec/`.
New implementation phases should be added here when scoped.

## Status (2026-06-24)

Both backend-closing tracks have landed (see `docs/ARCHIVED.md`):

- **Track 1** — v1 native-backend, Phases A–D — complete. **Track 2** —
  performance, Phases 30–33 — complete.
- **v1 is semantically and natively complete.** Every feature parses,
  type-checks through THIR, elaborates to TLC, runs in the `zutai-eval`
  differential oracle, and lowers to native code. Row polymorphism is
  native-complete (open-row selects, Phase C; open-union matches, Phase D); the
  constraints/witnesses item dispatches natively including conditional
  cross-module witnesses (Phase B). Residual v1 items are by-design non-goals
  ("v1 residual" below), not unfinished work.
- **v2 is largely native.** The five v2 features landed at
  check-plus-reference-interpreter level (Phases 24–28); four also lower
  natively — recursive types (cyclic type descriptors), host capabilities
  (`HostOp` lowering + the Track B entry boundary), derive recipes
  (fold-or-reject reflection), and higher-rank polymorphism (rank-2 lambda-arg
  compile parity, `compiled_rank2_lambda_arg_matches_oracle`). The fifth feature,
  universe levels, gained its **explicit surface syntax** in milestone V2-A
  (`$ℓ` / `<$l>`, front-end-only, erases before TLC); levels still erase before
  the backend, so no new native lowering is required.
- 1600 workspace tests pass.

**Can we move to v2?** We already have — Phases 24–28 and Track B are v2 work,
and four of the five v2 features lower natively. There is no v1 native-backend
blocker left. The small **v2 tail** (V2-A, explicit universe syntax) **landed
2026-06-24** (`docs/ARCHIVED.md` "V2-A"), so all five v2 features now have
surface syntax. One phase remains **active**: the **escaping-effect residual-ABI
spike** (Phase 35). Phase 34 (GC) stays gated and the v3 items below stay
deferred.

## V2 milestone — remaining work

V2-A (explicit universe-level syntax) **landed 2026-06-24** — see
`docs/ARCHIVED.md` "V2-A". The remaining active V2-adjacent work is the
escaping-effect residual-ABI spike (Phase 35, below). Phase 34 (GC) stays gated.

### Phase 34 — Conservative mark-sweep GC (runtime; Track 2, gated)

Decision (A), 2026-06-22: the native backend commits to **strict semantics plus
tail-call optimization** (Phase 31), with garbage collection **deferred**. The
compiled heap stays leak-by-default inside the capped thread-local arena
(Phase 30); the cap bounds the blast radius, and finite strict programs
terminate within it.

- **Gate — schedule only once a real GC workload exists**: unbounded streams
  reach the backend, or accumulator garbage dominates after Phase 33. The
  uncurrying prerequisite (Phase 33) landed, so calling-convention churn no
  longer dominates allocation (~2/3 of accumulator allocation was one arg-tuple
  + one closure per curried call, now removed); schedule a collector only once
  genuine user-data garbage — not call churn — does.
- **Approach — committed: zero-ABI conservative non-moving mark-sweep as the
  bridge.** Root-finding, not the algorithm, was the open gap: untagged `i64`
  values (D-0002) make conservative scanning ambiguous, and a precise moving
  (Cheney) collector would need a shadow stack or stack maps — an ABI change
  beyond D-0008/D-0009. Decision (2026-06-24): the bridge collector accepts
  conservative scanning (and the false retention it implies) precisely to avoid
  that calling-convention change; D-0002 is not reopened here. This feeds the
  `runtime-abi.md` D-0008 endgame (precise non-moving mark-sweep → generational
  Cheney copying), which stays write-barrier-free only while the heap is
  write-once.
- **Lazy backend not taken.** A lazy backend (thunk update = mutation =
  old→young pointers) would force a write barrier; strict-plus-TCO is committed.

Empirical basis (measured with `ZUTAI_HEAP_STATS`): before TCO the native stack
bound recursion (~10^5–10^6 frames, far below the ~10^7–10^8-object heap cap), so
GC could not help; after TCO deep tail recursion runs in O(1) stack and the heap
becomes the binding constraint, making GC a meaningful space optimization for
bounded-live / unbounded-allocation programs (accumulator loops).

### Phase 35 — Escaping-effect residual ABI (active spike, time-boxed)

Handled effects CPS-elaborate to ordinary functions/matches and lower natively,
including effects reached *through a call* to a monomorphic, non-recursive
effectful function (Phase A inline-specialization), plus ambient `io.print`
(runtime `HostPrint`). Still **rejected** (refused, never miscompiled) before
Dataflow Core: recursive/mutually-recursive effectful callees, polymorphic and
higher-order effectful values, partial applications, effects escaping the entry
boundary other than `io.print`, and open effect rows
(`docs/spec/v1/05-effects.md`).

A general residual-effect ABI for these genuinely-escaping effects is the open
gap. The framing has shifted: `tlc-core.md` §9 deferred the reified `Free Op A`
free-monad form because it needs recursive Dataflow Core types to represent an
unbounded perform spine, and DC types were finite structural trees. **Phase 25
lifted that prerequisite** — recursive types now lower to cyclic `DfTyId` graphs
— so the specific blocker §9 named no longer holds.

Decision (2026-06-24): this is no longer a binary in-scope question but a
**scoped spike** — evaluate whether a reified `Free Op A` encoding can lower over
the now-available cyclic DC types, taking the rejected list above as its target
scope. The spike is exploratory; it does not promise delivery, and the
strict-AOT-rejects path (see "v1 residual" below) remains the fallback if the
encoding proves too costly.

Promoted to an active phase (2026-06-24) because the representational blocker is
gone (Phase 25) and the design questions are settled (`tlc-core.md` §9) — what is
left is investigation, not more design. Run it independently of V2-A (no shared
crates). Time-box and gate on a written go/no-go:

- [ ] **Encode.** Express `Free Op A = { pure: A } | { impure: Op }` with
  `resume: R -> Free Op A` over a cyclic `DfTyId`; confirm the perform spine
  represents as a real DC value (no new TLC node).
- [ ] **Lower one case.** Take the simplest rejected case — a single
  recursive/self-tail effectful callee — through DC → ANF → SSA → native and run
  it against the `zutai-eval` oracle.
- [ ] **Cost it.** Measure allocation/dispatch overhead of the reified spine
  versus the handler-passing CPS path that already lowers, and note which
  rejected cases (polymorphic/higher-order effectful values, partial
  applications, open rows) the encoding does *not* reach.
- [ ] **Go/no-go.** Write the decision: either scope a delivery phase for the
  cases the encoding covers, or record that strict-AOT-rejects stays the
  committed behavior and close the spike. Either outcome lands in
  `docs/ARCHIVED.md`.

## v1 residual — by design, not gaps

Do not file these as missing native work:

- **Higher-kinded instantiation** (polymorphic dictionary passing) stays
  check-only by design — eval and compile both refuse HKT execution
  (`unify.rs` "a refused check is the safe direction"); a type-checking
  `mapTwice (\x. x) [1; 2;]` is rejected at the type-check stage on both paths.
- **Reflection** (`fields`/`variants`/`schema`/`witness`) is compile-time:
  `compile`/`dataflow` fold serializable reflection to backend constants and
  reject residual reflection (a raw `witness` dictionary or a `Type`-valued
  result) before lowering (`aot_reflection_program`). Fold-or-reject is the
  intended model.
- **Rejecting unhandled non-`io.print` effects** at the backend is the committed
  strict-AOT behavior.
- **Annotation-required inference** where row/constraint inference is not
  principal is specified behavior (`docs/spec/v1/01-row-polymorphism.md`
  "Extended Inference").

## Deferred beyond v2 (v3+)

- **Richer generators.** The finite `stream { yield …; }` shell landed
  (Phase 29); the richer-generator design is open
  (`docs/v3_spec/01-generators.md`).
- **Reserved design boundaries** past v2 (`docs/v2_spec/00-index.md` "Deferred
  beyond v2"): GADT-style local type equalities and the coercion/cast core node
  (an explicit non-goal, `tlc-core.md` §10), impredicative instantiation
  (v2 higher-rank polymorphism stays predicative), unforgeable capability tokens
  (v2 capabilities carry advisory authority), and nominal recursive types
  (v2 recursive types are equirecursive).

## Doc reconciliation (2026-06-24 audit)

Surfaced by the V1→V2 readiness audit; both resolved 2026-06-24:

- **Forward-dated ARCHIVED stamps.** Nine `2026-06-25` stamps in
  `docs/ARCHIVED.md` (the "Current baseline" Last-updated note plus eight
  completed-milestone entries) were one day ahead of the authoritative date;
  corrected to 2026-06-24.
- **Higher-rank support level.** `docs/v2_spec/05-higher-rank-polymorphism.md`
  "Support Level" understated support as reference-interpreter only; native
  rank-2 lambda-arg parity is in fact tested
  (`compiled_rank2_lambda_arg_matches_oracle`). Corrected.
