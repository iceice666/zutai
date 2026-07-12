# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-12)

_No active core-language, syntax, native/interpreter parity, standard-library,
or tooling milestone is scoped._

The syntax-stabilization pass consolidated the former numbered specifications
by language area and promoted every parser-accepted surface form into the stable
specification. A construct may still have a deliberately narrower execution
envelope. Those limits are support levels, not future language versions.

## Stable-syntax change policy

New surface syntax is not accepted as a speculative placeholder. A syntax
change must include:

- a stable-spec and language-manual update;
- parser coverage and a source-located diagnostic for malformed forms;
- HIR, THIR, and TLC semantics, or a precise documented refusal point;
- reference-interpreter and native support statements; and
- acceptance evidence for every support level claimed.

Compatibility spellings already accepted by the parser remain part of the
stable surface until an explicit deprecation and migration decision removes
them.

## Intentional support boundaries

These are specified behavior, not backlog items:

- Higher-kinded instantiation remains check-only; evaluation and native compile
  refuse unsupported HKT execution.
- Reflection is compile-time. Supported reflection folds before Dataflow Core;
  residual reflection and `Type`-valued program results are rejected.
- Unhandled or ungranted residual host effects are rejected by strict AOT.
- Non-principal row and constraint inference requires explicit annotations.
- Non-matchable cross-module witness exports remain native-gated.
- Non-tail `yield from` remains unsupported; tail delegation is stable.

## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
