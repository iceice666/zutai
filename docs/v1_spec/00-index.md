# Zutai Language Specification v1 — Deferred Features

These features build on the v0 language and are planned beyond the v0 core.

v1 inherits v0 surface syntax entirely. Only new semantic constructs are introduced: row tails (`...`, `...Rest`), `@T` witness targets, `!` effect rows, `derive`, `select`, `fields`, and `schema`. All function clauses use `| pat = body` and all type parameters use `<A>`, exactly as in v0.

## Row Polymorphism

- [Row polymorphism](01-row-polymorphism.md) — open records, named row tails, open unions, selective projection

## Type-Level Computation Extensions

- [Type-level computation extensions](02-type-level-computation.md) — advanced type functions, kind annotations, universe levels, type normalization guidance

## Constraint System

- [Constraints](03-constraints.md) — named behavioral interfaces, witnesses, superconstraints, derive, higher-kinded constraints

## Metaprogramming

- [Metaprogramming](04-metaprogramming.md) — compile-time reflection, schema reification

## Algebraic Effects

- [Algebraic effects](05-effects.md) — typed effect rows, `perform`, `handle`, `with`, `resume`, and explicit capabilities
