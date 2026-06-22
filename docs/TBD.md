# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

## Deferred to v2 (see `docs/v2_spec/`)

_Scoped 2026-06-22. Order is dependency-aware; when a phase completes, move a
short support-level summary to `docs/ARCHIVED.md` and leave unfinished follow-up
here._


### Phase 25: Recursive type aliases and equirecursive equality

Source of truth: [`v2_spec/01-recursive-types.md`](v2_spec/01-recursive-types.md).

- Resolve top-level type aliases in SCC binding groups so definition order does
  not matter for mutually recursive aliases.
- Permit guarded self, mutual, and generic recursive aliases under records,
  unions, tuples, lists, optionals, and function arrows; reject bare/non-productive
  alias cycles before type-level evaluation.
- Lower aliases by reference (`TyVar` / `TyLamK` / `TyApp`) rather than eager
  expansion, and extend NbE/type equality with fuel-bounded equirecursive
  unfolding.
- Carry recursive type identity through Dataflow Core `DfTyId`s and static runtime
  type descriptors; reflection emits finite named back-references rather than
  infinite expanded shapes.
- Acceptance: recursive `Tree`, mutually recursive `Expr`/`Args`, and generic
  `Tree A` examples check, evaluate finite values, render through `run` and
  compiled output, and compare via the built-in structural equality derivation;
  unguarded cycles keep rejecting with a productivity diagnostic; cyclic runtime
  values remain unsupported/non-goal.

### Phase 26: Higher-rank polymorphism

Source of truth: [`v2_spec/05-higher-rank-polymorphism.md`](v2_spec/05-higher-rank-polymorphism.md).

- Extend type syntax/HIR/THIR to preserve nested quantifiers in annotation
  positions such as `(<A> A -> A) -> R`, including constrained quantifiers.
- Add bidirectional checking that pushes written higher-rank expected types into
  lambda/function arguments. Inference remains predicative and rank-1; the
  compiler never synthesizes an unannotated higher-rank type.
- Elaborate nested `ForAll` positions to existing TLC `ForAll`/`TyLam`/`TyApp`
  machinery, including dictionary passing for constrained higher-rank arguments.
- Acceptance: the `applyId` and constrained `showBoth` examples type-check and
  run; insufficiently annotated higher-rank uses request an annotation; attempts
  at impredicative instantiation such as `List (<A> A -> A)` reject precisely.

### Phase 27: Host capabilities beyond ambient `io.print`

Source of truth: [`v2_spec/02-host-capabilities.md`](v2_spec/02-host-capabilities.md).

- Add opaque standard capability types and declarations for `FsRead`, `FsWrite`,
  `Env`, `Clock`, `Rng`, and explicit `IoPrint`, plus standard operations
  `fs.read`, `fs.write`, `env.get`, `clock.now`, and `rng.next`.
- Thread capability values as ordinary parameters while effect rows continue to
  state which host operations may occur. Authority is advisory only;
  unforgeable capability tokens remain beyond v2.
- Extend the `run`/native entry boundary so the host grants requested
  capabilities, dispatches residual granted operations, and rejects ungranted
  residual host operations as boundary errors. Ambient `io.print` remains
  source-compatible.
- Preserve handler interception: source handlers can mock or discharge host
  operations before the boundary.
- Acceptance: capability-parameter programs type-check only when their effect
  rows mention the performed operation; source handlers can make `fs.read` pure;
  granted operations have `run`/compiled-output parity; ungranted filesystem,
  environment, clock, and randomness operations reject before unsafe backend
  erasure.

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

## Deferred beyond v2

GADT-style local type equalities and the coercion/cast core node (an explicit
non-goal, `tlc-core.md` §10), impredicative instantiation, unforgeable
capability tokens, and nominal recursive types. See `v2_spec/00-index.md`
"Deferred beyond v2".
