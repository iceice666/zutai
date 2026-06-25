# Zutai Open Work

Open work is grouped by deferral horizon. Completed milestones and their
implementation detail live in `docs/ARCHIVED.md`; language design lives in
`docs/spec/v0/` (stable), `docs/spec/v1/`, `docs/v2_spec/`, and `docs/v3_spec/`.
New implementation phases should be added here when scoped.

## Status (2026-06-25)

v1 is semantically and natively complete; v2 is largely native (four of five
features lower natively, universe levels erase before the backend); v3 is
underway on the generators/streams spine. The v1/v2 backend-closing tracks, the
escaping-effect residual-ABI spike (Phase 35, no-go), the conservative GC
(Phase 34, opt-in), V2-A, V3-G1/G2, and cross-module polymorphism (single- and
multi-type, XM-1…3) have all landed — see `docs/ARCHIVED.md`. 1609 workspace
tests pass.

## Active milestone — none

The next V3 phase is **G3 (richer `yield`)**; G4 (resource-backed / effectful
generators — currently rejected) and G5 (GC default-on for unbounded streams)
follow. See `docs/v3_spec/02-roadmap.md`.

**G2 residuals** (do not block G3): `empty`/`unfold` (type-inference edge cases);
the `List`-interop subset `take -> List`/`toList`/`fromList` (needs source-level
list construction the language lacks); and the **importable `.zt` module**
packaging. The importable packaging is the originally-preferred form; it was
blocked natively by cross-module polymorphism, which has now landed, so it is
ready to pick up.

## GC residual — future / gated

The conservative mark-sweep collector landed opt-in (Phase 34, archived). Still
future / gated:

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
  decision — `Stream A` is **codata** (demand-driven step+seed), not a
  memoizing lazy list, so it stays inside strict+TCO and the write-barrier-free
  GC — and sequences it as V3-G1 (codata `Stream` representation) → G2 (stdlib
  `Stream` API) → G3 (richer `yield`) → G4 (resource-backed generators) → G5
  (GC default-on for unbounded streams). G1/G2 landed; resume at **G3**.
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
