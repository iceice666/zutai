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
- Still open: *cancellation* (signalling a generator to stop mid-stream), general
  resource lifetime, the ergonomic effectful-stream *type* (its expressibility
  foundation — open effect-row tails in annotations — landed 2026-06-26 check-only,
  see `docs/ARCHIVED.md`; the remaining step is call-site effect-row inference plus
  the `StreamEff` alias), and whether a general (non-tail) delegating yield is worth
  a shared codata `append`.

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

Support level: **check + reference-interpreter**. An effectful generator runs on
the interpreter when its effects are *granted*; native lowering of those effects
stays refused (the committed strict-AOT-rejects boundary — `Phase 35`,
`docs/spec/v1/05-effects.md`).

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
below); *cancellation* and general resource lifetime remain open.

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

## Remaining non-goals

- No ambient filesystem, environment, clock, randomness, or network iteration.
- No second iterator abstraction beside `Stream`.
- No *cancellation*-aware generator runtime (signalling a generator to stop
  mid-stream). Finalization landed as the `finally` handler clause (V3-G4
  follow-up); cancellation and general resource lifetime stay open (V3-G4+).
- No general (non-tail) `yield from`: only tail delegation lowers; a non-tail
  splice is refused pending a shared codata `append`.

Pure infinite/recursive generators *are* now supported (V3-G1 codata + V3-G3
richer `yield`): `range lo hi = stream { if lo < hi then { yield lo; yield from
range (lo + 1) hi; } }` type-checks and evaluates — interpreter and native — to
the same `Stream` the equivalent `unfold` produces.
