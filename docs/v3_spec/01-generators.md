# Generator and Yield Syntax

Status: deferred beyond v2; design intent and constraints only. This page does
not define final syntax or grammar productions.

## Decisions

- `yield`/generator syntax is deferred beyond v2.
- Any generator form must desugar to `Stream` or to algebraic effects/handlers
  that produce `Stream`; it must not introduce a second effect system.
- Generators are not ambient host iterators. Filesystem, environment, clock,
  randomness, and future network-backed generation require explicit
  capabilities.
- The v3 design must settle cancellation/finalization, resource lifetime,
  interaction with laziness, and whether `yield` is expression syntax or handler
  sugar before implementation.

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

## Non-goals for this shell

- No grammar productions for `yield`.
- No parser, HIR, THIR, evaluator, TLC, or backend implementation.
- No ambient filesystem, environment, clock, randomness, or network iteration.
- No second iterator abstraction beside `Stream`.
