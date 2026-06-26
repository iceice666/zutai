# Zutai Open Work

Open work is grouped by deferral horizon. Completed milestones and their
implementation detail live in `docs/ARCHIVED.md`; language design lives in
`docs/spec/v0/` (stable), `docs/spec/v1/`, `docs/v2_spec/`, and `docs/v3_spec/`.
New implementation phases should be added here when scoped.

## Status (2026-06-26)

v1 is semantically and natively complete; v2 is largely native (four of five
features lower natively, universe levels erase before the backend); v3 is
underway on the generators/streams spine. The v1/v2 backend-closing tracks, the
escaping-effect residual-ABI spike (Phase 35, no-go), the conservative GC
(Phase 34, opt-in), V2-A, V3-G1…G5 (the full generators/streams spine),
cross-module polymorphism (single- and multi-type, XM-1…3), V3-G6 (importable
`stream.zt` module), the `unfold` + `empty` stream combinators (V3-G2
residuals, the latter on a first-class `BindingRef` instantiation site), the
`List`-interop subset (`toList`/`fromList`/`takeList`, V3-G2 residual), and the
V3-G6 import-ergonomics follow-ups (embedded `stdlib.stream`, `Stream`/`Step`
type export, destructuring binding) have all landed — see `docs/ARCHIVED.md`.
**This closes V3-G2.**

## Active milestone — none

`empty` + `unfold` (V3-G2 residuals: the empty stream and the canonical codata
producer) **landed 2026-06-25** — see `docs/ARCHIVED.md` "V3-G2 residual: `unfold`
combinator" and "BindingRef instantiation site". `unfold` ships as an ambient
prelude combinator and an importable `stream.zt` export, taking a step function
returning a structural `Step S A` union (`#done`/`#yield { item; next }`) rather
than the builtin `Optional` (whose `#some` payload is a positional tuple that does
not compose with a record payload). `empty :: <A> Stream A` landed once a
polymorphic value referenced outside callee position instantiates per use — the
THIR `BindingRef` node now records its own instantiation (the fix that unblocked
any polymorphic value, not just `empty`).

V3-G6 (importable `stream.zt` module) **landed 2026-06-25** — see
`docs/ARCHIVED.md` "V3-G6". The codata `Stream` combinators are now a real
importable `.zt` module backed by one canonical source
(`crates/general/hir/src/lower/prelude/stream.zt`, exposed as
`zutai_hir::STREAM_MODULE_SRC`) that also feeds the ambient prelude via
`include_str!`; the ambient surface is unchanged and `s ::= import "stream.zt"`
exports the eight combinators (`s.map`, `s.fold`, …). The recursive `Stream`
codata type crossing the import boundary required a symmetric cross-module
global-ref compat fix in the Dataflow Core validator (sound under untagged-i64).
**This closes the last structural V3-G2 residual.** Remaining V3 work is the
demand-gated Track 2 boundaries and the open generator questions below.

**V3-G6 import-ergonomics follow-ups — landed (see `docs/ARCHIVED.md`
"Import ergonomics: embedded stdlib, type export, destructuring"):**

- **Stdlib-root resolution** shipped as an *embedded* stdlib: `import stdlib.stream`
  resolves to in-binary source (no install path, no subtree-confinement exception).
- **`Stream`/`Step` type export** shipped — both are now record fields on the
  importable module and are selectable/destructurable.
- **Selective / open import** shipped as a destructuring binding
  (`{ map; fold; } ::= s;`) reusing the select-field list syntax.

**Residual (open):**

- **Applied imported type constructors — landed 2026-06-26** (see
  `docs/ARCHIVED.md` "Applied imported type constructors"). A parametric imported
  type constructor can now be *applied* in an annotation (`x :: s.Stream Int`) for
  arbitrary user modules: `export_type_value` preserves the constructor's binder
  as `ImportedType::TypeCon` (recursive self-references stay bounded as
  `ConApply`), and the importer rebuilds it as a local parametric alias via
  synthetic bindings + a materialized `TypeAlias` decl. v1 refuses higher-kinded
  constructor parameters and (cleanly, via the runtime type-value gate) TLC
  evaluation of modules that export type values; both refusals are precise.
- **`import` as a destructure RHS — landed 2026-06-26** (see `docs/ARCHIVED.md`
  "`import` unified as an expression"). `import` is now an expression atom and the
  dedicated `name :: import source` declaration form was removed, so a plain import
  binding is `name ::= import …` and members destructure in one binding:
  `{ map; fold; } ::= import stdlib.stream;`. The source stays a literal, so
  resolution remains pure and static.

## Previous milestone — V3-G5 (landed 2026-06-25)

V3-G5 (GC keeps unbounded stream pipelines bounded) **landed 2026-06-25** — see
`docs/ARCHIVED.md` "V3-G5". Acceptance met: `fold (+) 0 (take n (countFrom 1))`
over an infinite generator holds peak committed flat at 1 MiB for `n = 100k` and
`800k` (leak-by-default grows 34 → 269 MiB), with correct output and stress
soundness. G5 first landed with the collector opt-in; the default was then flipped
to **GC on by default** (`ZUTAI_GC=0` opts out) — see `docs/ARCHIVED.md` "GC
default-on (D-0008 reversal)". **V3 Track 1 (generators & streams) is complete.**

**G4 follow-ups (open):** cancellation/finalization and resource lifetime for
effectful generators; an ergonomic effectful-stream *type* (the supported idiom
uses the raw cell type, not the pure `Stream` alias).

**Other G2 residuals:** all landed. The importable-module residual closed with
V3-G6, `unfold` + `empty` shipped 2026-06-25, and the `List`-interop subset
(`toList`/`fromList`/`takeList`) shipped 2026-06-26 — see `docs/ARCHIVED.md`
"V3-G2 residual: List interop". **No V3-G2 residuals remain.**

(`unfold` landed as an ambient + importable combinator taking a `Step S A` union
(`#done`/`#yield { item; next }`) — the builtin `Optional`'s `#some` payload is a
positional tuple that does not compose with a record payload at the surface.
`empty :: <A> Stream A` landed once `BindingRef` became a first-class
instantiation site, so a polymorphic value referenced outside callee position
instantiates per use. See `docs/ARCHIVED.md`.)

## GC residual — future / gated

The conservative mark-sweep collector (Phase 34) is now **on by default** where the
stack scan is supported (`ZUTAI_GC=0` opts out) — see `docs/ARCHIVED.md` "GC
default-on (D-0008 reversal)". Still future / gated:

- **Precise/moving endgame.** The `runtime-abi.md` D-0008 endgame (precise
  non-moving mark-sweep → generational Cheney copying) stays deferred: it needs a
  shadow stack or stack maps (a calling-convention change beyond D-0008/D-0009),
  which the conservative bridge collector exists specifically to avoid. D-0002
  (untagged `i64`) is not reopened.
- **Lazy backend not taken.** A lazy backend (thunk update = mutation =
  old→young pointers) would force a write barrier; strict-plus-TCO is committed.
- **Other-target root finding.** The conservative stack scan is wired up for
  macOS (`pthread_get_stackaddr_np`) and Linux (`pthread_getattr_np`); other
  targets leave the collector off (leak-by-default, even with the new default-on)
  until their stack-bounds path
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

- **Track 1 — generators and streams. ✅ Complete (V3-G1…G5, 2026-06-25).**
  `Stream A` is **codata** (demand-driven step+seed), not a memoizing lazy list,
  so it stays inside strict+TCO and the write-barrier-free GC. The full spine
  landed: G1 (codata representation) → G2 (stdlib API) → G3 (richer `yield`) → G4
  (effectful generators, reference-interpreter) → G5 (GC keeps unbounded pipelines
  bounded). The G2 residuals (`empty`/`unfold`, `List` interop, importable `.zt`
  packaging) have all landed. Open follow-ups: cancellation/finalization and
  resource lifetime for effectful generators; an ergonomic effectful-stream type.
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
