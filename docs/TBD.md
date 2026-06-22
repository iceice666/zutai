# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

## Deferred to v2 (see `docs/v2_spec/`)

_Scoped 2026-06-22. Order is dependency-aware; when a phase completes, move a
short support-level summary to `docs/ARCHIVED.md` and leave unfinished follow-up
here._

### Phase 28: Derive recipes and witness reflection

Source of truth: [`v2_spec/03-derive-recipes.md`](v2_spec/03-derive-recipes.md).

- Add the compile-time `witness C @T` reflection primitive, using the same
  coherence and resolution rules as implicit dictionary passing.
- Store user-attached derive recipes on constraints and run each recipe once per
  `(constraint, type)` derivation request under the type-level fuel bound.
- Unify type-value reflection and TLC derive synthesis so recipes can consume
  `fields`, new `variants`, and `schema` output, including open rows and
  recursive named back-references.
- Reify recipe results into witness dictionaries, type-check them against the
  constraint method signatures, and report missing component witnesses,
  fuel-exhausted recipes, or ill-typed generated witnesses at the derivation
  request.
- Keep the built-in structural equality derivation as the default for derive
  constraints without an attached recipe; user recipes override only their own
  constraint family.
- Acceptance: `Show` and lexicographic `Ord` recipes derive witnesses for records
  and unions; missing component witnesses produce localized compile errors;
  recursive types derive through recursive witness bindings; generated method
  bodies are specialized at compile time rather than interpreting reflection data
  at runtime.

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
