# Zutai Open Work

Open work is grouped by deferral horizon. Completed milestones and their
implementation detail live in `docs/ARCHIVED.md`; language design lives in
`docs/spec/v0/` (stable), `docs/spec/v1/`, `docs/v2_spec/`, and `docs/v3_spec/`.
New implementation phases should be added here when scoped.

## Status (2026-07-07)

The post-V3 readiness audit **landed 2026-06-28** (see `docs/ARCHIVED.md`
"Post-V3 readiness audit"). v1 semantics are complete, and native support is
release-ready inside the documented AOT envelope; v2 is largely native (four of
five features lower natively, universe levels erase before the backend); v3
Track 1 and its scoped follow-ups are complete. The
audit reconciled user-facing docs and support levels with the landed baseline,
and confirmed an executable coverage anchor for every
materially supported feature. The GC residual is retired (conservative
default-on mark-sweep is the committed endpoint). Track 2 remains
demand-gated and must not be implemented unless a concrete program forces one
of the reserved core-design boundaries.

The source-prelude / stdlib usability row landed through slice H:
B (small function prelude), C (minimal `List` verbs), D (Optional helpers),
E (Result and Validation helpers), F (Numeric helpers), G (Text helpers), and
H (Comparator helpers) **landed 2026-06-28** (see `docs/ARCHIVED.md` for the
per-slice summaries). This is stdlib work, not Track 2, and does not reopen any
core language boundary. The 2026-06-30 explicit stdlib expansion added
`stdlib.config`, `stdlib.reflect`, `stdlib.list`, `stdlib.data`, and
`stdlib.validate` as embedded opt-in modules; see `docs/ARCHIVED.md` for the
milestone summary. The 2026-07-01 native library artifact mode
(`compile --emit=lib`) and runtime `serde_json` bridge are also archived; they
do not leave an active native backlog item. The 2026-07-07 scoped filesystem IO
foundation (`Reader`/`Writer`, explicit `stdlib.fs`, text line/read-write host
ops) is also archived and leaves append/seek/binary/async IO intentionally
unscoped. Track 2 remains demand-gated.

## Native/interpreter parity backlog

_No active native/interpreter parity backlog items._ The optional-field `Maybe`
envelope gap landed 2026-06-30 and the explicit `stdlib.fs` helper-import
native lowering gap landed 2026-07-07; both are archived in `docs/ARCHIVED.md`.

## Source prelude / stdlib status

_No active source-prelude/stdlib usability milestone is scoped._ Slices B-H
landed 2026-06-28 and the explicit stdlib expansion landed 2026-06-30; both are
archived in `docs/ARCHIVED.md`: small function helpers, minimal
ambient/importable `List` verbs, explicit `stdlib.optional`, `stdlib.result`,
`stdlib.num`, `stdlib.text`, `stdlib.cmp`, explicit `stdlib.fs`, and the
explicit `stdlib.config`, `stdlib.reflect`, `stdlib.list`, `stdlib.data`, and
`stdlib.validate` modules.

Deferred/non-goals after stdlib usability: non-tail generator `yield from`,
cross-module witness native ABI, all Track 2 boundaries, and generic
witness-dispatched `compare`.

## Tooling / test-harness backlog

_No open tooling backlog items. The native-link test race under
`cargo test --workspace` and the release slice R0 CLI acceptance pack both landed
2026-06-28; see `docs/ARCHIVED.md` "Native-link test race fix" and "Release
acceptance pack (release slice R0)"._

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
- **Rejecting unhandled or ungranted residual host effects** at the backend is
  the committed strict-AOT behavior. Raw-cell effectful generators have native
  parity for supported custom effects, ambient `io.print`, and standard host
  operations that lower through the host boundary under an explicit grant.
- **Annotation-required inference** where row/constraint inference is not
  principal is specified behavior (`docs/spec/v1/01-row-polymorphism.md`
  "Extended Inference").
- **Non-matchable cross-module witness exports** stay native-gated: concrete
  imported witnesses and structurally matchable conditional witnesses lower
  natively through extern witness tables, while higher-kinded or otherwise
  non-dispatchable exported witness shapes still reject before Dataflow Core
  rather than silently dropping dispatch state.

## Deferred beyond v2 (v3+)

Now sequenced in the **V3 roadmap** (`docs/v3_spec/02-roadmap.md`). Summary:

- **Track 1 — generators and streams. ✅ Complete (V3-G1…G5, 2026-06-25).**
  `Stream A` is **codata** (demand-driven step+seed), not a memoizing lazy list,
  so it stays inside strict+TCO and the write-barrier-free GC. The full spine
  landed: G1 (codata representation) → G2 (stdlib API) → G3 (richer `yield`) → G4
  (effectful generators, reference-interpreter) → G5 (GC keeps unbounded pipelines
  bounded). The G2 residuals (`empty`/`unfold`, `List` interop, importable `.zt`
  packaging) have all landed, as has G4 finalization (the `finally` handler
  clause, 2026-06-26), the ergonomic effectful-stream type (call-site effect-row
  inference + the `StreamEff` alias, 2026-06-27), cooperative cancellation
  (aborting the granting handler, 2026-06-27), and resource lifetime as the
  granting-handler dynamic-extent contract (2026-06-27).
- **Track 2 — reserved design boundaries (demand-gated, not a backlog)**
  (`docs/v2_spec/00-index.md` "Deferred beyond v2"): GADT-style local type
  equalities and the coercion/cast core node (an explicit non-goal,
  `tlc-core.md` §10), impredicative instantiation (v2 higher-rank polymorphism
  stays predicative), unforgeable capability tokens (v2 capabilities carry
  advisory authority), and nominal recursive types (v2 recursive types are
  equirecursive). Build one only when a concrete need drives it.
