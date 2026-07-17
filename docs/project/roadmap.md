# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-17)

The implemented baseline is no longer syntax discovery. Zutai already has one
stable surface with parser, HIR, THIR, TLC, reference-interpreter, native-AOT,
browser, package, LSP, typed-staging, reflection, and decoder coverage recorded
in [status](status.md) and [history](../history/README.md). Future work should
therefore improve confidence, portability, and real-program ergonomics without
adding speculative syntax.

Roadmap order follows dependency order: editor/package trust first, backend and
runtime confidence second, application-facing ergonomics third. A milestone moves
into implementation only when its refusal behavior, support level, and validation
gate are explicit.

## Near-term: package-aware editor and diagnostics hardening

Package-aware navigation, completion, stable diagnostics, canonical formatting,
native target portability, and the measured optimization gate are complete. New
near-term or backend work should be added only with an explicit refusal model,
validation gate, and workload evidence. No optimization is scheduled: the
recorded website build/toolchain outlier still requires focused profiling.


## Long-term: application ergonomics on the stable core

Goal: prove Zutai can carry real applications without weakening `.zti` inertness,
purity, typed host effects, or the TLC/Dataflow Core boundary.

The constraint-backed collection vocabulary is complete. Remaining milestones,
in order:

1. **Derived first-order data encoding.** If the qualification workload needs
   typed interchange back to a host or browser boundary, add a `ToData`-style
   constraint and structural derive builder mirroring the supported closed
   `FromData` shapes, without changing `.zti` or tagged-union syntax. Gate:
   `decode (encode value)` round trips scalars, atom singletons, lists, optionals,
   closed records, and closed unions through the evaluator, native JSON bridge,
   and browser bundle; open rows, tuples, recursive targets, fixed-width/posit
   scalars, opaque handles, functions, and `Type` values refuse at the derive
   request.
2. **Demand-gated language boundary review.** Revisit reserved boundaries only
   with a motivating program and a concrete semantic rule. Each proposal must
   name parser impact, HIR/THIR/TLC impact, Dataflow Core/runtime impact,
   refusal behavior, and migration risk before it can become a scheduled
   milestone.


Deferred across these phases: optimizing laziness beyond the current Dataflow
Core sharing model, general non-tail `yield from`, asynchronous or binary host
IO, a package registry/version solver, and new surface syntax without a concrete
program. These are candidates only after the relevant workload and measurement
gate identifies them as the smallest solution.

## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
