# Zutai V3 Roadmap

Status: planning. _Last updated: 2026-06-24._

V3 builds on the v0 core, the v1 deferred set, and the v2 deferral horizon. This
document turns the V3 holding area into a sequenced plan: it names the one active
V3 feature track (generators and streams), records the design decision that keeps
that track inside the committed backend, and restates the reserved design
boundaries as demand-gated — features to build only when a concrete need arises,
not a backlog to burn down.

The generator *shell* (Phase 29) and the opt-in conservative collector
(Phase 34) have landed; both are prerequisites that made the Track 1 spine below
implementable. **Track 1 is complete: V3-G1…G5 all landed 2026-06-25**
(`docs/ARCHIVED.md`). `Stream A` is demand-driven codata with a builtin source
prelude (G1), the core combinator API ships as ambient prelude functions (G2),
richer `yield` (conditionals + tail recursion) desugars onto the codata cell (G3),
effectful generators run under a granting handler at reference-interpreter level
(G4), and the conservative collector keeps unbounded stream pipelines bounded
(G5; the collector was subsequently flipped **on by default**, `ZUTAI_GC=0` to opt
out — a D-0008 reversal, see `docs/ARCHIVED.md`). Track 2 stays demand-gated.

## Backend-compatibility invariants

Every V3 design must respect the decisions the v1/v2 backend already committed to.
These are constraints, not open questions; a V3 feature that violates one is out
of scope until that decision is explicitly reopened.

- **Strict evaluation + tail-call optimization** (Phase 31) is the committed
  execution model. **No lazy thunk-update backend**: a memoizing thunk is a
  mutation (old→young pointers) and forces a write barrier (`TBD.md` Phase 34,
  "Lazy backend not taken").
- **Write-barrier-free garbage collection** (Phase 34): the conservative
  mark-sweep collector stays write-barrier-free only while the heap is
  write-once. V3 features must not introduce heap mutation that would require a
  barrier, and must not reopen D-0002 (untagged `i64`).
- **Equirecursive types**, **predicative higher-rank polymorphism**,
  **advisory capability authority**, and **fold-or-reject reflection** all carry
  forward unchanged.
- **Strict-AOT rejection of *unhandled* effects** carries forward — but, as of
  2026-06-26, *handled* effects the lexical CPS path could not discharge
  (recursive, higher-order, partially-applied effectful values; effectful builtin
  operands; `finally`) now compile natively via the reify pass
  (`reify_residual_effects`), reversing the Phase 35 no-go. Genuinely unhandled
  effects, polymorphic/open-row effect *execution* (also refused by the
  interpreter), and effectful generators stay refused. See `docs/ARCHIVED.md`
  "Native effect parity".
- **Kernel invariant "equality = normalization"** (`tlc-core.md` §10): no V3
  surface feature may force a coercion/cast core node.

The practical consequence for V3: laziness and infinite sequences must be
expressed as **codata** (demand-driven step functions), not as mutable lazy
cells. This single constraint shapes the entire generator track.

## Track 1 — Generators and streams (active spine)

The one articulated V3 feature. Today `stream { yield e; … }` desugars to a
`Stream A` value, but `Stream A ≡ List A` — fully strict, finite only
(`docs/stdlib/stream.md`, `docs/v3_spec/01-generators.md`). Richer generators
require a real stream representation first.

### Decision: streams are codata, not lazy lists

A `Stream A` is a **step function plus a seed** (an `unfold`-shaped cell),
demanded one element at a time through `uncons`/`take`. It is *not* a
memoizing lazy list.

- **Why.** Demand-driven codata has no shared thunk to update, so it needs no
  write barrier — it stays inside strict+TCO and the write-barrier-free GC. A
  memoizing-thunk stream would force the lazy backend that was explicitly not
  taken. The existing stdlib surface already points this way: `unfold :: (S ->
  Optional { item : A; next : S; }) -> S -> Stream A` and `uncons :: Stream A ->
  Optional { head : A; tail : Stream A; }`.
- **Cost accepted.** Codata recomputes rather than memoizes: a stream consumed
  twice steps twice. This is the deliberate trade for backend compatibility;
  callers that need sharing materialize with `toList`.
- **Infinite streams become representable.** An `unfold` with a non-terminating
  seed is a genuine infinite stream; `take`/`uncons` bound the demand. This is
  the first time unbounded sequences reach the backend — i.e. garbage-collector
  gate condition (a).

### Phases

Each phase keeps all earlier behavior working and is gated on `zutai-eval`
oracle parity (a wrong value is worse than a refused one).

- **V3-G1 — Codata `Stream` representation. ✅ Landed 2026-06-25.** `Stream A` is
  now `Unit -> StreamCell A` (a unit-thunk over a `#nil`/`#cons` cell). Finite
  `stream { yield … }` desugars to nested thunks; infinite `unfold`/`take`
  terminates. No ABI change. Delivered via a builtin source prelude (G1-P).
  Observed by forcing/folding (`s ()`), not as a list. See `docs/ARCHIVED.md`
  "V3-G1". (Acceptance met on interpreter and native backend.)
- **V3-G2 — Stdlib `Stream` API. ✅ Landed (prelude) 2026-06-25.** Core
  combinators `cons`, `singleton`, `map`, `filter`, `take`, `drop`, `fold`,
  `uncons` ship as **ambient prelude functions** (`.zt` over the codata cell),
  native-compiled and oracle-checked. The prelude is a *fallback* (user /
  constraint names of the same spelling win) and per-module reachability-gated.
  See `docs/ARCHIVED.md` "V3-G2". *Residuals — all since landed:* `empty`/`unfold`
  (2026-06-25); the importable `.zt` module packaging (V3-G6, 2026-06-25); and the
  `List`-interop subset (`toList`/`fromList`/`takeList`, 2026-06-26 — scalar list
  bridge primitives over the builtin `List`, native + oracle-checked). **V3-G2 is
  complete.**
- **V3-G3 — Richer `yield`. ✅ Landed 2026-06-25.** `yield` now appears under
  conditionals (`if cond then { … } [else { … }]`) and recursion (tail
  `yield from`), settling the open question: richer `yield` is **statement syntax
  desugared by continuation-passing** onto the V3-G1 codata cell, not handler
  sugar — no second iterator abstraction. A non-tail `yield from` is refused
  (`NonTailYieldFrom`). The clause-arity check was relaxed to allow a body to
  bind a *prefix* of the parameters (uniform across clauses), so a generator
  function supplies the codata `Unit` from its desugared thunk. See
  `docs/ARCHIVED.md` "V3-G3". (Acceptance met on interpreter and native backend.)
- **V3-G4 — Resource-backed generators. ✅ Landed (reference-interpreter) 2026-06-25.**
  An effectful generator runs under a *granting handler* on the interpreter: a
  `yield perform op …` defers the operation into a lazy cell field, so the effect
  is charged to the consumer that strictly forces it — `handle (sumEff (stream {
  yield perform tick (); })) with { tick = \_. resume 5; }` evaluates. Without a
  handler the effect escapes and is refused; native lowering of the (non-`io.print`)
  effect stays refused by the committed strict-AOT-rejects boundary (Phase 35), so
  resource host effects (`fs.read`, networking, clocks, randomness) reach only the
  interpreter behind an explicit grant. No new effectful-codata type was added;
  the existing effect machinery carries it. See `docs/ARCHIVED.md` "V3-G4" and
  `01-generators.md` "Effectful generators". Finalization (`finally`) and
  cooperative cancellation (aborting the granting handler) have since landed;
  general resource lifetime remains open.
- **V3-G5 — GC for unbounded stream programs. ✅ Landed (acceptance) 2026-06-25.**
  With genuine unbounded streams reaching the backend (gate condition (a), met by
  V3-G1), the Phase 34 conservative collector keeps a long-running stream pipeline
  bounded: `fold (+) 0 (take n (countFrom 1))` over an infinite generator holds
  **peak committed flat at 1 MiB** for `n = 100k` and `n = 800k` under `ZUTAI_GC`
  (where leak-by-default grows 34 MiB → 269 MiB), with correct output and
  soundness under `ZUTAI_GC_STRESS`. *"Default-on":* G5 first landed with the
  collector opt-in; the default was then flipped to **GC on by default** (with a
  `ZUTAI_GC=0` opt-out, D-0008 reversal) — see `docs/ARCHIVED.md` "GC default-on".
  *Acceptance met:* a long-running `unfold` pipeline holds steady-state RSS flat
  under collection while producing correct output.

Open generator questions to settle within the track (carried from
`01-generators.md`): *finalization* landed as the `finally` handler clause
(2026-06-26), the *ergonomic effectful-stream type* landed as call-site
effect-row inference + the `StreamEff` alias (2026-06-27), and *cooperative
cancellation* landed as consumer-driven mid-stream termination over the abort +
`finally` machinery (2026-06-27, `docs/ARCHIVED.md`); general resource lifetime
for resource-backed generators remains open.

## Track 2 — Reserved design boundaries (demand-gated)

Inherited from `docs/v2_spec/00-index.md` "Deferred beyond v2." These are
*conscious stopping points with kernel-level cost*, not a backlog. Build one only
when a concrete language need drives it, and only as a deliberate core-design
change — never as an additive convenience.

- **GADT-style local type equalities / coercion-cast core node.** Reserved for
  *kernel* cost: admitting `a ~ Int` refinement retrofits a System F_C–style
  `Coerce(e, T, U)` node and abandons "equality = normalization"
  (`tlc-core.md` §10–11). *Un-reserve only if* a real program needs branch-local
  type refinement that no existing feature expresses.
- **Impredicative instantiation.** Reserved for *decidability*: instantiating a
  type variable with a quantified type loses principal types and makes inference
  undecidable. v2 higher-rank polymorphism stays predicative by choice.
  *Un-reserve only if* a concrete need outweighs decidable, predictable checking.
- **Unforgeable capability tokens.** v2 capability authority is advisory
  (possessing the value authorizes). Unforgeability is a *distinct* typing rule
  binding a specific value to authority over a specific operation. *Un-reserve
  only if* a security requirement needs enforced (not advisory) authority.
- **Nominal recursive types.** v2 recursive types are equirecursive (unfold
  structurally). *Un-reserve only if* distinct type identity is required for
  abstraction or error-quality reasons.

## Sequencing and entry point

Track 1 ran from **V3-G1** — the codata `Stream` representation, the keystone the
rest of the track hung off — through G2 (stdlib API), G3 (richer `yield`), G4
(effectful generators, reference-interpreter level), and G5 (GC keeps unbounded
stream pipelines bounded), each a contained phase with no ABI change. **Track 1
is complete.** Remaining V3 work is the demand-gated Track 2 boundaries and the
open generator question of general resource lifetime (finalization landed as the
`finally` clause 2026-06-26, the ergonomic effectful-stream type as call-site
effect-row inference + the `StreamEff` alias 2026-06-27, and cooperative
cancellation as aborting-the-granting-handler 2026-06-27) below. Track 2 stays
demand-gated.

When a V3 phase is scoped for implementation, add it to `docs/TBD.md` as the
active phase and move its summary to `docs/ARCHIVED.md` on completion.
