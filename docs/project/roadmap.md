# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-16)

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

Goal: make the existing stable language pleasant and safe to use across local
packages before adding any new surface area.

Milestones:



## Mid-term: backend parity and reproducible native builds

Goal: make native output a boring deployment target for the stable language
subset already accepted by the frontend.

Milestones:


Deferred here: optimizing laziness beyond the current Dataflow Core sharing
model. Profile real programs first; do not add thunk machinery or memoization
layers speculatively.

## Long-term: application ergonomics on the stable core

Goal: prove Zutai can carry real applications without weakening `.zti` inertness,
purity, typed host effects, or the TLC/Dataflow Core boundary.

Milestones:

1. **Standard-library ergonomics pass.** Expand documented stdlib examples and
   fixtures around records, tagged unions, streams, `FromData`, `derive`, HTML,
   CSS, and host capabilities. The work is library/API polish unless a concrete
   program demonstrates a language gap. Acceptance: examples compile or refuse
   at their documented support level through CLI, LSP diagnostics, and the
   reference interpreter where applicable.
2. **Self-hosted website as integration workload.** Treat the browser kernel and
   web bundle path as a full-stack regression target: local packages, stdlib
   imports, prerendered HTML hydration, retained-tree reconciliation, events,
   keyed lists, controlled inputs, and live reload. Acceptance: focused native
   tests plus the wasm-browser hydration scenario cover the same fixture, with
   manual browser checks only where WebDriver is unavailable.
3. **Demand-gated language boundary review.** Revisit reserved boundaries only
   with a motivating program and a concrete semantic rule. Each proposal must
   name parser impact, HIR/THIR/TLC impact, Dataflow Core/runtime impact,
   refusal behavior, and migration risk before it can become a scheduled
   milestone.


## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
