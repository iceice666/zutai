# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

## Deferred to v2 (see `docs/v2_spec/`)

_Scoped 2026-06-22. Order is dependency-aware; when a phase completes, move a
short support-level summary to `docs/ARCHIVED.md` and leave unfinished follow-up
here._


## Deferred to v3 (see `docs/v3_spec/`)

### Phase 29: Stream-backed generator syntax

Source of truth: [`v3_spec/01-generators.md`](v3_spec/01-generators.md).

- Design generator/yield syntax only after `Stream` is specified.
- Desugar generator forms to `Stream` or effect handlers that produce `Stream`;
  do not introduce a second effect system.
- Preserve explicit host capabilities; no ambient filesystem, environment,
  clock, randomness, or network iteration.
- Acceptance: examples that use pure generators type-check and evaluate through
  `Stream`; resource-backed examples require capability parameters and effect
  rows; unsupported residual host operations keep rejecting before backend
  erasure.

## Deferred beyond planned v3 work

GADT-style local type equalities and the coercion/cast core node (an explicit
non-goal, `tlc-core.md` §10), impredicative instantiation, unforgeable
capability tokens, and nominal recursive types remain unassigned to the active
v2/v3 roadmap.
