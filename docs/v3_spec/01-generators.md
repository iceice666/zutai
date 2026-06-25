# Generator and Yield Syntax

Status: richer `yield` implemented (V3-G3). Phase 29 introduced the finite
generator shell; V3-G1 made `Stream A` demand-driven codata; V3-G3 lets `yield`
appear under conditionals and recursion. Resource-backed (effectful) generators
remain future work (V3-G4).

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
- Future work must still settle cancellation/finalization and resource lifetime
  for resource-backed (effectful) generators (V3-G4), and whether a general
  (non-tail) delegating yield is worth a shared codata `append`.

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

## Remaining non-goals

- No ambient filesystem, environment, clock, randomness, or network iteration.
- No second iterator abstraction beside `Stream`.
- No cancellation-aware or finalization-aware generator runtime (V3-G4+).
- No general (non-tail) `yield from`: only tail delegation lowers; a non-tail
  splice is refused pending a shared codata `append`.

Pure infinite/recursive generators *are* now supported (V3-G1 codata + V3-G3
richer `yield`): `range lo hi = stream { if lo < hi then { yield lo; yield from
range (lo + 1) hi; } }` type-checks and evaluates — interpreter and native — to
the same `Stream` the equivalent `unfold` produces.
