# Zutai Open Work

Open work is grouped by deferral horizon. Completed milestones and their
implementation detail live in `docs/ARCHIVED.md`; language design lives in
`docs/spec/v0/` (stable), `docs/spec/v1/`, `docs/v2_spec/`, and `docs/v3_spec/`.
New implementation phases should be added here when scoped.

## Status (2026-06-27)

v1 is semantically and natively complete; v2 is largely native (four of five
features lower natively, universe levels erase before the backend); v3 Track 1
and its scoped follow-ups are complete. The v1/v2 backend-closing tracks, the
escaping-effect residual-ABI spike (Phase 35, no-go), the conservative GC
(Phase 34, now default-on), V2-A, V3-G1…G5 (the full generators/streams spine),
cross-module polymorphism (single- and multi-type, XM-1…3), V3-G6 (importable
`stream.zt` module), the `unfold` + `empty` stream combinators (V3-G2
residuals, the latter on a first-class `BindingRef` instantiation site), the
`List`-interop subset (`toList`/`fromList`/`takeList`, V3-G2 residual), V3-G6
import ergonomics, and the resource-lifetime contract for effectful generators
have all landed — see `docs/ARCHIVED.md`.
**This closes V3-G2 and the scoped V3-G4 follow-ups.**

**The GC residual is retired:** conservative default-on mark-sweep is the committed endpoint; precise/moving GC, a lazy-backend write barrier, and other-target collector expansion are no longer active milestones.

## Previous milestone — Resource lifetime for effectful generators (landed 2026-06-27)

Resource lifetime for effectful generators **landed 2026-06-27** — see
`docs/ARCHIVED.md` "Resource lifetime for effectful generators". Support level:
**reference-interpreter support with explicit backend rejection** for
non-`io.print` resource effects. The granting handler's dynamic extent is the
single owner of resource-backed stream lifetime; normal full consumption, partial
consumption, cooperative cancellation, and cross-boundary abort through nested
`finally` handlers are covered by oracle tests. Lazy escapes refuse when forced
outside the grant, and native lowering of non-`io.print` resource-backed cells is
gated. Non-goals remain: no asynchronous/preemptive cancellation, no ambient
filesystem, clock, network, or randomness iteration, no host iterator abstraction,
no cell-level finalizers, and no native resource-effect lowering unless the
backend contract is explicitly reopened.

**Cooperative cancellation — landed 2026-06-27** (see `docs/ARCHIVED.md` "V3-G4
follow-up: cooperative cancellation for effectful generators"). Cancellation —
signalling a generator to stop mid-stream — needs no new runtime: it is **handler
abort** (a clause returning without `resume`) reused as a control signal. A
consumer performs a cancelling op (`perform stop acc`); the *granting* handler
(bearing the generator's effects + `finally`) gives it an aborting clause
(`stop = \r. r;`), so the suspended tail is never forced again and the handler's
`finally` finalizes the resource. A follow-up replaced the temporary
cross-boundary refusal guard with explicit finalizer unwinding: an abort that
crosses an inner `finally`-bearing handler now runs the inner teardowns
inner-to-outer before completing, and resumed effects remain unaffected; the later
resource-lifetime milestone closed the scoped V3-G4 lifetime follow-up.

**Ergonomic effectful-stream type — landed 2026-06-27** (see `docs/ARCHIVED.md`
"Ergonomic effectful-stream type: call-site effect-row inference + `StreamEff`").
The two pieces the spec named as scoped both landed: **call-site effect-row
inference** (a pure or concretely-effectful argument now unifies against an
instantiated open-row parameter — `effect_rows_unify`/`effect_rows_match` solve a
flexible `RowTail::Infer` into the new `RowSolution::Effect`, mirroring union/record
rows, instead of exact-tail matching) and the **`StreamEff A e` ambient/importable
prelude alias** naming the supported V3-G4 idiom (`StreamEff A {}` ≡ `Stream A`).
Parity-or-refuse held (the pure-`Stream`-alias annotation still refuses; native
lowering of the effect stays refused). Residual (pre-existing, not new): an
*imported* `StreamEff` applied as a parametric constructor across a module boundary
refuses cleanly — the row-param type constructor is outside the "Applied imported
type constructors" envelope; the ambient form is the supported path. **This closes
the scoped V3-G4 effectful-stream type follow-up.**

**Native effect parity — landed 2026-06-26** (see `docs/ARCHIVED.md` "Native
effect parity — reified delimited-continuation lowering"). The native backend now
compiles the handled algebraic effects it previously refused, by reifying the
interpreter's runtime continuation model into generated TLC
(`reify_residual_effects`), reversing the Phase 35 "no-go" on the user's explicit
request. Recursive & mutually-recursive callees, higher-order effectful values,
partial application, effectful builtin operands, and `finally` all compile
natively and match the `eval_tlc` oracle; the lexical CPS fast path is unchanged.

**Native effect-parity residual gates — landed 2026-06-26** (see `docs/ARCHIVED.md`
"Native effect-parity residual gates closed"). The two conservative gates surfaced
by the pressure test now compile natively and match the oracle: an *inline*
partial-application passed as a higher-order argument (`applyTo (addP 5)`) is
eta-expanded to a lambda value whose body is reified at the call site, and an
effectful function stored in a *record field* (`box.f 7`) is discovered through the
wrapper, has its field type rewritten to `… -> Computation`, and is called through a
`GetField`-headed reified call. The two paths compose (`applyTo (box.f 5)`). Anything
beyond the new envelope (a wrapper observed outside the handle scope, multi-shot
resume, polymorphic effectful values) still refuses cleanly — parity-or-refuse held.

**Effectful generators (V3-G4) — the supported idiom now compiles natively
(2026-06-26).** `stream { yield perform … }` consumed strictly under a handler
(the *raw-cell-type* idiom the docs designate as supported) now lowers and matches
the oracle (`compiled_effectful_generator{,_ordering}_matches_oracle`). The reify
pass stores the deferred `perform` as strict `Computation`-DATA in the cell's
effectful field — the carrier goes on the *field*, not the demand thunk, so
`Computation` stays monomorphic — builds a scope-local `Cell'` with that field
typed `Computation`, and the consumer `bind`s it; the cell is produced strictly and
purely, so demand order and early termination (unforced tails never fire) follow
the interpreter exactly (the interpreter is also strict-at-force for effectful
modules — `tlc_module_can_defer_aggregates` is `false`). See `reify.rs`
(`detect_eff_codata`, `build_cell_primes`, `reify_cell_body`).

Not parity gaps: a **recursive/infinite** effectful generator is a pure-typed
top-level producer function that performs, which the interpreter itself rejects
(`tick not in effect row`); a **conditional** effectful generator type-errors on
the interpreter too. **Polymorphic (`TyLam`) effectful values** and **open effect
rows** likewise need polymorphic effect *execution* the interpreter refuses.
Narrow residual: the same generator written with the parametric prelude `Stream`
alias (rather than a raw cell type) runs on the interpreter but stays gated —
its cell type is a *type application* (`StreamCell Int`) the monomorphic detection
does not yet recognize.

**V3-G4 follow-up: `finally` finalization clause — landed 2026-06-26** (see
`docs/ARCHIVED.md` "`finally` finalization clause"). A `finally = expr` handler
clause runs a teardown once when a `handle` reduces to its final value (normal
completion *or* handler abort), in the outer row — the resource-finalization
primitive for effectful generators. It fires even when a consumer stops early
(partial `take`), because a deferred effect is charged to whoever forces it under
the granting handler, so the handler's extent bounds the resource. The teardown
runs on both the interpreter and, as of the native-effect-parity work, the
native backend (`desugar_finally` → outer-row sequencing).

**V3-G4 follow-up: open effect-row tails (check-only foundation) — landed
2026-06-26** (see `docs/ARCHIVED.md` "Open effect-row tails"). Effect-row
annotations now accept an open row tail — `! { ops; ...e }` (a row variable),
`! { ...e }`, or `! { ... }` (anonymous open) — mirroring the existing
record/union row-tail syntax. A `...e` naming an in-scope type parameter lowers to
a rigid `RowTail::Param` (threaded by exact-tail unification); anonymous `...`
lowers to `Open`; an effect-row `...Shape` spread of a named type is refused
precisely. This is the **expressibility foundation** for an effectful-stream type;
it is *check-only* — a row-polymorphic effect signature checks and lowers cleanly,
and execution stays gated by the existing residual-effect gate.

The **ergonomic effectful-stream type** landed 2026-06-27 (call-site effect-row
inference + the `StreamEff` ambient/importable alias — see `docs/ARCHIVED.md`).
**Cooperative cancellation** and the **cross-boundary finalizer unwinding**
follow-up landed 2026-06-27 too (consumer-driven mid-stream termination over the
abort + `finally` machinery, with inner finalizers run on cross-boundary aborts).
The later resource-lifetime milestone closed the remaining scoped V3-G4 follow-up.

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

**G4 follow-ups:** *finalization* landed as the `finally` handler clause
(2026-06-26), the *ergonomic effectful-stream type* landed as call-site
effect-row inference + the `StreamEff` alias (2026-06-27), *cooperative
cancellation* landed as aborting-the-granting-handler (2026-06-27), and
*resource lifetime* landed as the granting-handler dynamic-extent contract
(2026-06-27) — see `docs/ARCHIVED.md`. No scoped G4 follow-ups remain.

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
