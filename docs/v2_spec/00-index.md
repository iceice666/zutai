# Zutai Language Specification v2 — Deferred Features

These features build on the v0 core and the v1 deferred-feature layer. They have
landed at check-plus-reference-interpreter level (`docs/ARCHIVED.md` Phases
24–28); native-backend parity varies per feature, and remaining backend gaps are
tracked in [`TBD.md`](../TBD.md).

v2 inherits all v0 and v1 surface syntax. It introduces user-defined recursive
data types, host capabilities beyond `io.print`, user-defined `derive` recipes
with witness reflection, internal universe levels, and higher-rank polymorphism.
This directory is the language-design source of truth for these features; each
chapter's Support Level section records its current implementation status.

## Recursive Types

- [Recursive types](01-recursive-types.md) — user-defined recursive and mutually
  recursive data types, guardedness, generic recursive types, equirecursive
  equality

## Algebraic Effects — Host Capabilities

- [Host capabilities](02-host-capabilities.md) — filesystem, environment, clock,
  and randomness as capability-gated effects; capability values, the entry
  boundary, handler interception, and advisory authority

## Constraint System — Derive Recipes

- [Derive recipes](03-derive-recipes.md) — user-defined compile-time derivation,
  the `witness` reflection primitive, and reflection completeness

## Type-Level Computation — Universe Levels

- [Universe levels](04-universe-levels.md) — internal `Type0 : Type1`
  stratification, cumulativity, level inference and polymorphism

## Polymorphism

- [Higher-rank polymorphism](05-higher-rank-polymorphism.md) — nested quantifiers
  in annotations, bidirectional checking, predicative inference

## Deferred beyond v2

- **GADT-style local type equalities** and the coercion/cast core node — an
  explicit design boundary (see [`tlc-core.md`](../tlc-core.md) §10).
- **Impredicative instantiation** — a type variable instantiated with a
  polymorphic type; v2 higher-rank polymorphism stays predicative (see
  [higher-rank polymorphism](05-higher-rank-polymorphism.md)).
- **Unforgeable capability tokens** with value-to-operation authority
  enforcement; v2 capabilities carry advisory authority (see
  [host capabilities](02-host-capabilities.md)).
- **Nominal recursive types** with distinct identity that do not unfold
  structurally; v2 recursive types are equirecursive (see
  [recursive types](01-recursive-types.md)).
