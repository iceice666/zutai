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
- **Escaping-effect residual-ABI spike (Phase 35) closed 2026-06-24** — see
  `docs/ARCHIVED.md` "Phase 35". The `Free Op A` encoding is proven to lower over
  the cyclic `DfTyId` types and match the oracle for the monomorphic closed-row
  recursive case, at ~2× the allocation of the CPS path; the decision is that
  strict-AOT-rejects stays the committed backend behavior.
- **Phase 34 (conservative mark-sweep GC) landed opt-in 2026-06-24** — see
  `docs/ARCHIVED.md` "Phase 34". A zero-ABI conservative collector behind
  `ZUTAI_GC` keeps accumulator peak-committed flat (1 MiB) as work grows 8×; the
  committed default stays leak-by-default, so nothing changed by default. Only
  the precise/moving (Cheney) endgame and a lazy backend remain deferred.
- **v2 is largely native.** The five v2 features landed at
  check-plus-reference-interpreter level (Phases 24–28); four also lower
  natively — recursive types (cyclic type descriptors), host capabilities
  (`HostOp` lowering + the Track B entry boundary), derive recipes
  (fold-or-reject reflection), and higher-rank polymorphism (rank-2 lambda-arg
  compile parity, `compiled_rank2_lambda_arg_matches_oracle`). The fifth feature,
  universe levels, gained its **explicit surface syntax** in milestone V2-A
  (`$ℓ` / `<$l>`, front-end-only, erases before TLC); levels still erase before
  the backend, so no new native lowering is required.
- 1609 workspace tests pass.

**Can we move to v2?** We already have — Phases 24–28 and Track B are v2 work,
and four of the five v2 features lower natively. There is no v1 native-backend
blocker left. The small **v2 tail** (V2-A, explicit universe syntax) **landed
2026-06-24** (`docs/ARCHIVED.md` "V2-A"), so all five v2 features now have
surface syntax. The **escaping-effect residual-ABI spike** (Phase 35) **closed
2026-06-24** with a no-go on delivery (`docs/ARCHIVED.md` "Phase 35"). Phase 34
(conservative GC) **landed opt-in 2026-06-24** (`docs/ARCHIVED.md` "Phase 34").
The **V3 roadmap** is now written (`docs/v3_spec/02-roadmap.md`) and its first
phase **V3-G1 (codata `Stream` representation)** **landed 2026-06-25**
(`docs/ARCHIVED.md` "V3-G1"); the next phase is G2 (stdlib `Stream` API).

## Active milestone — none

V3-G1 (codata `Stream` representation) **landed 2026-06-25** — see
`docs/ARCHIVED.md` "V3-G1". `Stream A` is now demand-driven codata
(`Unit -> StreamCell A`); finite generators fold correctly and infinite
`unfold`/`take` terminates, on both the interpreter and native backend.

**V3-G2 (stdlib `Stream` API) landed via the prelude 2026-06-25** — see
`docs/ARCHIVED.md` "V3-G2". The core combinators (`cons`, `singleton`, `map`,
`filter`, `take`, `drop`, `fold`, `uncons`) are ambient prelude functions,
native-compiled and oracle-checked, with the prelude acting as a fallback (user /
constraint names win). The next V3 phase is **G3** (richer `yield`); G4
(resource-backed / effectful generators — currently rejected) and G5 (GC
default-on for unbounded streams) follow.

**G2 residuals** (do not block G3): `empty`/`unfold` (type-inference edge cases);
the `List`-interop subset `take -> List`/`toList`/`fromList` (needs source-level
list construction the language lacks); and the **importable `.zt` module**
packaging, which is blocked natively by cross-module polymorphism (scoped below).
The importable packaging is the originally-preferred form — pick it up once
cross-module polymorphism lands.

## Cross-module polymorphism

**Single-instantiation cross-module generics landed 2026-06-25** — see
`docs/ARCHIVED.md` "Cross-module polymorphism (single-type)". A module exporting a
polymorphic value (`id :: <A> A -> A`, or a record of generic combinators) and
used at a **single concrete type per program** now compiles natively and matches
the interpreter. The fix exploited the untagged-i64 ABI (D-0002): a parametric
value is compiled once and is bit-identical across instantiations, so the
Dataflow structural validator was taught to accept a use type that is a sound
`TyVar`-instantiation of a generic dependency global (`is_instantiation_of` in
`validate/refs.rs`), instead of ICEing on the `Fun(TyVar,TyVar)` vs `Fun(Int,Int)`
mismatch. Multi-type use is **cleanly rejected** (a THIR type error, never an ICE).

### Residual — multi-type cross-module generics (for the importable stdlib)

The import boundary still has *no* polymorphism representation: `export.rs`
flattens `TypeVar`/`ForAll` to `ImportedType::Unknown` (`export.rs:135,141`) and
`import.rs` interns `Unknown` as a *fresh inference variable* (`import.rs:53`), so
an imported generic is monomorphized by its first use — using it at two types is a
type error. Lifting this (needed only when one program uses an imported generic at
several types, e.g. an importable `Stream` stdlib with `map` at both `Int` and
`Text`) needs the boundary-scheme rework:

- **XM-1 — Boundary scheme representation.** Quantified variant on `ImportedType`
  (bound type-var ids + body), with round-trip.
- **XM-2 — Generalize on export.** `export.rs` emits the scheme (top-level
  `∀A. A->A`, and the higher-rank record-field case `{ id : ∀A. A->A }` — rank-2,
  within v2's predicative budget; confirm).
- **XM-3 — Instantiate on import (THIR).** Import interning yields a poly scheme;
  THIR instantiates each imported reference at its use site (fresh vars + `TyApp`).
  *Acceptance:* an imported generic used at **two concrete types** type-checks.
  (Native lowering is already handled by the single-type validator relaxation
  above, so no further Dataflow work is expected once THIR emits the per-use
  instantiation.)

## V2 milestone — remaining work

V2-A (explicit universe-level syntax) **landed 2026-06-24** — see
`docs/ARCHIVED.md` "V2-A". The escaping-effect residual-ABI spike (Phase 35,
below) **closed 2026-06-24** with a no-go on delivery. No active V2-adjacent
work remains; Phase 34 (conservative GC) **landed opt-in 2026-06-24**, with only
the precise/moving endgame deferred.

### Phase 34 — Conservative mark-sweep GC (LANDED opt-in 2026-06-24)

**Landed as an opt-in bridge collector** — see `docs/ARCHIVED.md` "Phase 34".
After the gate condition (b) was instrumented and shown met (the post-Phase-33
accumulator is O(n) garbage against an O(1) live set), a zero-ABI conservative
non-moving mark-sweep collector was built behind `ZUTAI_GC` /`ZUTAI_GC_STRESS`.
The committed default stays **leak-by-default** (D-0008): the collector is opt-in,
so no existing behavior changed. With it enabled, accumulator peak-committed stays
flat (1 MiB) as work grows 8× where the leak default grows ~linearly (2→13 MiB).

Residual GC work, still future / gated:

- **Precise/moving endgame.** The `runtime-abi.md` D-0008 endgame (precise
  non-moving mark-sweep → generational Cheney copying) stays deferred: it needs a
  shadow stack or stack maps (a calling-convention change beyond D-0008/D-0009),
  which the conservative bridge collector exists specifically to avoid. D-0002
  (untagged `i64`) is not reopened.
- **Lazy backend not taken.** A lazy backend (thunk update = mutation =
  old→young pointers) would force a write barrier; strict-plus-TCO is committed.
- **Other-target root finding.** The conservative stack scan is wired up for
  macOS (`pthread_get_stackaddr_np`) and Linux (`pthread_getattr_np`); other
  targets leave the collector off (leak-by-default) until their stack-bounds path
  lands.

### Phase 35 — Escaping-effect residual ABI (spike CLOSED 2026-06-24, no-go on delivery)

**Closed 2026-06-24.** The four sub-tasks below are done; the full go/no-go
rationale lives in `docs/ARCHIVED.md` "Phase 35". Summary: the `Free Op A`
encoding lowers over the cyclic `DfTyId` types and matches the oracle for the
monomorphic closed-row recursive/self-tail case (~2× the CPS path's allocation),
but does not by itself reach polymorphic/higher-order effectful values or open
rows. Decision: **strict-AOT-rejects stays the committed backend behavior**; the
encoding is de-risked and left ready to scope only if a real workload demands
native recursive effects.

Background: handled effects CPS-elaborate and lower natively (including effects
reached *through a call* to a monomorphic, non-recursive effectful function via
Phase A inline-specialization, plus ambient `io.print`); recursive/mutually-recursive
effectful callees, polymorphic/higher-order effectful values, partial
applications, and open effect rows stay **rejected** (refused, never
miscompiled) before Dataflow Core (`docs/spec/v1/05-effects.md`). Phase 25 lifted
the one representational blocker `tlc-core.md` §9 named (DC types were finite
trees; recursive types now lower to cyclic `DfTyId` graphs), which is what made
the `Free Op A` encoding worth a feasibility spike. The four sub-tasks, all
**done**:

- [x] **Encode.** Express `Free Op A = { pure: A } | { impure: Op }` with
  `resume: R -> Free Op A` over a cyclic `DfTyId`; confirm the perform spine
  represents as a real DC value (no new TLC node). *Done — recursive union with a
  `resume` function field lowers via the same equirecursive knot-tying as `Tree`.*
- [x] **Lower one case.** Take the simplest rejected case — a single
  recursive/self-tail effectful callee — through DC → ANF → SSA → native and run
  it against the `zutai-eval` oracle. *Done —
  `compiled_free_monad_spine_matches_oracle` in `crates/cli/tests/cli.rs`.*
- [x] **Cost it.** Measure allocation/dispatch overhead of the reified spine
  versus the handler-passing CPS path that already lowers, and note which
  rejected cases (polymorphic/higher-order effectful values, partial
  applications, open rows) the encoding does *not* reach. *Done — ~2× allocation
  (`ZUTAI_HEAP_STATS`); reaches only the monomorphic closed-row recursive case.*
- [x] **Go/no-go.** Write the decision: either scope a delivery phase for the
  cases the encoding covers, or record that strict-AOT-rejects stays the
  committed behavior and close the spike. Either outcome lands in
  `docs/ARCHIVED.md`. *Done — no-go on delivery; strict-AOT-rejects committed.*

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

Now sequenced in the **V3 roadmap** (`docs/v3_spec/02-roadmap.md`). Summary:

- **Track 1 — generators and streams (active spine).** The finite
  `stream { yield …; }` shell landed (Phase 29); the richer-generator design is
  open (`docs/v3_spec/01-generators.md`). The roadmap fixes the keystone
  decision — `Stream A` becomes **codata** (demand-driven step+seed), not a
  memoizing lazy list, so it stays inside strict+TCO and the write-barrier-free
  GC — and sequences it as V3-G1 (codata `Stream` representation) → G2 (stdlib
  `Stream` API) → G3 (richer `yield`) → G4 (resource-backed generators) → G5
  (GC default-on for unbounded streams). Start at V3-G1.
- **Track 2 — reserved design boundaries (demand-gated, not a backlog)**
  (`docs/v2_spec/00-index.md` "Deferred beyond v2"): GADT-style local type
  equalities and the coercion/cast core node (an explicit non-goal,
  `tlc-core.md` §10), impredicative instantiation (v2 higher-rank polymorphism
  stays predicative), unforgeable capability tokens (v2 capabilities carry
  advisory authority), and nominal recursive types (v2 recursive types are
  equirecursive). Build one only when a concrete need drives it.

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
