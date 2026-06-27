# Generator and Yield Syntax

Status: richer `yield` implemented (V3-G3). Phase 29 introduced the finite
generator shell; V3-G1 made `Stream A` demand-driven codata; V3-G3 lets `yield`
appear under conditionals and recursion. Effectful generators run under a
granting handler at **reference-interpreter level** (V3-G4); native lowering of
their effects stays refused by committed design.

## Decisions

- Implemented syntax: a `stream { … }` block is a sequence of **statements**:
  - `yield expr;` — emit one element.
  - `yield from expr;` — *delegating yield*: splice every element of the
    sub-`Stream` `expr`. Supported in **tail position** (the block's final
    statement, with the terminal `#nil` continuation) — the canonical
    recursive/loop generator. A non-tail delegating yield is reported
    (`NonTailYieldFrom`), never miscompiled, because the codata cell has no
    shared append.
  - `if cond then { … } [else { … }]` — *conditional yield*: the branches are
    themselves generator-statement blocks, so a guard may emit, skip, or recurse.
    A missing `else` yields nothing on the false branch.
- `stream` is contextual. It starts a generator only when the following block
  begins with `yield` (the classic shell) or a guarding `if` (a
  conditional/recursive generator); otherwise `stream` remains an ordinary
  identifier and `stream { field = value; }` stays normal function application.
  To force application of a block whose head is `if`, parenthesise:
  `stream ({ if … })`.
- **Open question settled: richer `yield` is statement syntax desugared by
  continuation-passing onto the V3-G1 codata cell, not handler sugar.** A
  generator block lowers against its *continuation* (the stream that follows):
  `yield e` conses a `#cons { head = e; tail = <rest> }` thunk, a conditional
  yields per branch sharing that continuation, and a tail `yield from s` is the
  stream `s` itself. There is no second iterator abstraction and no effect/handler
  machinery; the result steps identically to the equivalent `unfold`. (A function
  whose body is such a generator binds a *prefix* of its parameters — ordinary
  currying — with the codata `Unit` supplied by the desugared thunk.)
- Generator bodies use normal expression typing and effect rows. Resource-backed
  generation still requires ordinary capability parameters/effect rows, and
  unsupported residual host operations keep rejecting before backend erasure.
  No second effect system or host iterator abstraction is introduced.
- **Finalization landed** as a `finally` handler clause (V3-G4 follow-up, see
  `docs/ARCHIVED.md` "`finally` finalization clause"): `handle e with { …;
  finally = teardown; }` runs `teardown` once when the handle reduces to a value
  (normal completion *or* abort), in the outer row. Because a deferred effect is
  charged to the consumer that forces it under the granting handler, the handler's
  extent bounds the resource, so `finally` fires even when a consumer stops early.
  Interpreter-only; native compilation of a finally-bearing handle is refused.
- **Ergonomic effectful-stream type landed** (V3-G4 follow-up, 2026-06-27, see
  `docs/ARCHIVED.md`): call-site effect-row inference (a pure/concrete argument
  unifies against an instantiated open-row parameter) plus the `StreamEff A e`
  ambient/importable alias naming the supported idiom (`StreamEff A {}` ≡
  `Stream A`). Built on the open-effect-row-tail foundation (2026-06-26).
- **Cancellation landed** (V3-G4 follow-up) as consumer-driven mid-stream
  termination over the existing abort + `finally` machinery (see "Cancellation"
  below): a consumer performs a cancelling operation whose *granting*-handler
  clause aborts (returns without `resume`), stopping the generator mid-stream and
  running that handler's `finally`. Cancellation that aborts *across* an inner
  `finally`-bearing handle — which would skip the inner teardown — is refused
  (`CancelAcrossFinalizer`), never silently leaked. Interpreter-only. Still open:
  general resource lifetime, and whether a general (non-tail) delegating yield is
  worth a shared codata `append`.

## Design intent

Generators are language syntax for building or adapting pure `Stream` values,
not a replacement for the standard-library `stream` module. Pure generator
examples should type-check and evaluate through the same `Stream` semantics as
source-defined stream pipelines.

Resource-backed generation must remain capability-typed. A generator that reads
from a host resource, observes time, samples randomness, or later consumes a
network source needs ordinary capability parameters and effect rows. Residual
host operations that are not handled or granted must keep rejecting before
backend erasure.

## Effectful generators (V3-G4)

Support level: **check + reference-interpreter + native** (raw-cell-type idiom, as
of 2026-06-26). An effectful generator runs on the interpreter when its effects
are *granted*, and the supported idiom — `stream { yield perform … }` consumed
strictly under a handler over a **raw cell type** (not the pure `Stream` alias) —
now also compiles natively and matches the oracle (`reify.rs`
`detect_eff_codata`/`build_cell_primes`/`reify_cell_body`; see `docs/ARCHIVED.md`
"Native effect parity"). The reify pass stores the deferred `perform` as strict
`Computation`-data in the cell's effectful field (carrier on the field, not the
demand thunk), so the cell is produced strictly and the effect fires when the
consumer `bind`s it — matching the interpreter, which is also strict-at-force for
effectful modules. Recursive/conditional effectful generators are rejected on both
paths (a pure-typed producer cannot `perform`); a generator typed through the
parametric prelude `Stream` alias remains a narrow native residual.

The mechanism reuses the existing effect machinery rather than a new effectful
codata type. A `yield perform op …` defers the operation into a *lazy cell
field*; the effect is therefore charged to whoever **forces** that field, not to
the constructor. So the supported idiom is:

- the producer performs effects in its cells (`stream { yield perform tick (); }`);
- a consumer that **strictly forces** each element declares the effect in its own
  row (`sumEff :: (Unit -> Cell) -> Int ! { tick … }`, forcing heads with
  `h + …`); and
- the whole consumption runs under a granting handler (`handle (sumEff gen) with
  { tick = \_. resume 5; }`).

Boundaries (each refused, never miscompiled):

- **No handler / pure consumer.** The effect escapes the ambient row → type error
  (`effectful_generator_without_a_handler_is_rejected`).
- **Pure `Stream A` annotation.** Typing an effectful producer as the pure alias
  `Stream A = Unit -> StreamCell A` is rejected — the deferred effect cannot
  satisfy the pure thunk the alias demands. Effectful streams are not the pure
  `Stream` alias and do not interoperate with the pure prelude combinators.
- **Lazy escape.** A consumer that *returns* an unforced effectful head (instead
  of forcing it under the handler) hits a runtime "unhandled effect" — a refusal,
  consistent with the spec's demand-driven ordering of pure data construction.
- **Native.** Any generator whose cells carry a non-`io.print` effect is refused
  by the residual-effect gate (`compile_effectful_generator_stays_gated`).

Resource host effects (`fs.read`, networking, clocks, randomness) therefore reach
only the interpreter, behind an explicit handler that grants them; they have no
native path. *Finalization* is supported via a `finally` handler clause (see
below), and *cancellation* via the same handler's aborting clause (see
"Cancellation"). General resource lifetime remains open.

### Finalization: the `finally` handler clause

A `handle e with { …; finally = teardown; }` runs `teardown` exactly **once** when
the handle reduces to its final value — both on normal completion and on handler
abort (a clause that returns without `resume`, discarding the continuation). The
teardown runs in the **outer** effect row (its own effects are not discharged by
this handler) and licenses no `resume`; its result is discarded.

Because a deferred generator effect is charged to whoever forces it under the
granting handler, the handler's dynamic extent already bounds the resource — so
`finally` fires precisely when consumption-under-the-handler ends, **including when
a consumer stops early** (a `take`-style partial fold of an effectful generator
still finalizes). The teardown is attached to the *handler*, not to the codata
`#cons` cell: a cell-level finalizer cannot work, since a dropped or recomputed
tail would never run it (or run it twice). Interpreter-only — native compilation
of a finally-bearing handle is refused with a precise diagnostic.

### Cancellation: aborting the granting handler

Cancellation — signalling a generator to stop mid-stream — needs no new runtime:
it is **handler abort** (a clause that returns without `resume`, discarding the
continuation) reused as a control signal. The supported idiom:

- the consumer performs a *cancelling* operation when it decides to stop
  (`perform stop acc`), carrying any accumulated result as the argument;
- the **granting handler** — the one bearing the generator's effects and its
  `finally` — provides an aborting clause for it (`stop = \r. r;`). Because the
  clause never resumes, the suspended generator tail is never forced again, so it
  stops mid-stream;
- that handler's `finally` then fires (finalization already runs on abort), so a
  cancelled generator finalizes its resource.

The accumulated result rides out on the cancelling operation's argument, so the
handle reduces to the value the consumer had computed at the cancellation point.

**Boundary — cancellation across a finalizer is refused.** Cancellation must abort
on the *same* handler that bears the `finally` (or one outside all relevant
finalizers). If a cancelling effect instead escapes an *inner* `finally`-bearing
handle to be aborted by an *outer* handler, honouring the abort would discard the
continuation carrying the inner `finally` and silently skip that teardown — a
resource leak. The interpreter refuses this with `CancelAcrossFinalizer` rather
than run the un-finalized program (parity-or-refuse: a refused program beats a
leaked resource). Resuming such an escaped effect is unaffected — the refusal is
abort-only. Full unwinding that runs inner finalizers on a cross-boundary cancel
is the remaining open item (general resource lifetime). Interpreter-only, like
`finally`.

## Remaining non-goals

- No ambient filesystem, environment, clock, randomness, or network iteration.
- No second iterator abstraction beside `Stream`.
- No *preemptive/asynchronous* cancellation runtime. Cooperative cancellation —
  a consumer aborting the granting handler to stop a generator mid-stream — landed
  over the abort + `finally` machinery (V3-G4 follow-up, see "Cancellation").
  What stays a non-goal is asynchronous interruption and full finalizer unwinding
  across a cross-boundary cancel; general resource lifetime stays open (V3-G4+).
- No general (non-tail) `yield from`: only tail delegation lowers; a non-tail
  splice is refused pending a shared codata `append`.

Pure infinite/recursive generators *are* now supported (V3-G1 codata + V3-G3
richer `yield`): `range lo hi = stream { if lo < hi then { yield lo; yield from
range (lo + 1) hi; } }` type-checks and evaluates — interpreter and native — to
the same `Stream` the equivalent `unfold` produces.
