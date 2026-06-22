# Generator and Yield Syntax

Status: accepted shell. Phase 29 implements a pure generator expression that
desugars to the current `Stream A ≡ List A` representation; richer generator
semantics remain future work.

## Decisions

- Implemented syntax: `stream { yield expr; yield expr; }`.
- `stream` is contextual. It starts a generator only when the following block
  begins with `yield`; otherwise `stream` remains an ordinary identifier and
  `stream { field = value; }` remains normal function application.
- The implemented shell desugars each yielded expression to an element of a
  `Stream` value. In the current compiler, `Stream A` lowers transparently to
  `List A`, so pure finite generators evaluate through the existing lazy list
  machinery.
- Generator bodies use normal expression typing and effect rows. Resource-backed
  generation still requires ordinary capability parameters/effect rows, and
  unsupported residual host operations keep rejecting before backend erasure.
  No second effect system or host iterator abstraction is introduced.
- Future work must still settle cancellation/finalization, resource lifetime,
  interaction with infinite/lazy streams, and whether richer `yield` forms are
  expression syntax or handler sugar.

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

## Remaining non-goals for this shell

- No ambient filesystem, environment, clock, randomness, or network iteration.
- No second iterator abstraction beside `Stream`.
- No stateful, infinite, cancellation-aware, or finalization-aware generator
  runtime beyond finite pure `yield` sequences.
