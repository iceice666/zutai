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

Goal: make the existing stable language pleasant and safe to use across a
multi-package project before adding any new surface area.

Milestones, in order:

1. **Package-wide references and safe rename.** Extend the package-aware
   definition path to find references across the root package and transitive
   dependencies. Rename may edit the root package and writable path dependencies;
   it must refuse builtins, ambient prelude bindings, generated bindings, and
   immutable locked-Git snapshots. Gate: a three-package fixture with shadowed
   names, imported value/type members, unsaved overlays, and an attempted rename
   into a locked dependency produces complete locations and no unrelated edits.
2. **Import-aware completion and workspace symbols.** Complete package aliases,
   public modules, and exported members from the same recorded graph used by
   checking; add workspace-symbol search over root and dependency modules.
   Results must respect visibility, shadowing, malformed-package fallback, and
   unsaved overlays. Gate: CLI/LSP package fixtures prove deterministic results
   and exact source locations without network access.
3. **Stable semantic diagnostic identities and fixes.** Give HIR, import, THIR,
   derive, and backend-gate diagnostics stable codes like the parser already has;
   preserve the same code, severity, primary source, and related locations in CLI
   and LSP rendering. Add a quick fix only where the replacement is unambiguous.
   Gate: the backend-refusal and cross-file diagnostic matrices assert codes and
   protocol payloads rather than message substrings alone.
4. **Canonical source formatting.** Add idempotent formatters for `.zt` and
   `.zti`, then expose them through the CLI and LSP. General-mode formatting must
   preserve comments and accepted compatibility spellings unless a separate
   compatibility decision approves a rewrite; immediate-mode formatting must
   preserve field/item order. Gate: parse-format-parse structural equality,
   comment/order preservation, second-pass byte equality, and the full
   specification/example/manifest corpus.

## Mid-term: backend parity and portable native builds

Goal: make native output a boring deployment target for the stable language
subset already accepted by the frontend.

Milestones, in order:

1. **Explicit native target model.** Replace host-derived target selection with
   a validated target descriptor used by LLVM preambles, data layouts, metadata,
   runtime-archive lookup, object assembly, and linking. Support the four current
   intended native pairs first: Linux/macOS on x86-64/AArch64. Gate: same-source
   object builds and deterministic metadata for every supported target; binary
   and library links run where a matching linker and runtime archive are present,
   and unavailable target/toolchain combinations refuse before writing a partial
   artifact.
2. **Imported higher-kinded witness execution.** Carry matchable constructor
   witnesses such as `Functor @List` and `Foldable @List` across package
   boundaries through TLC, Dataflow Core, and native codegen. Higher-order kind
   targets remain out of scope. Gate: path-package and locked-Git fixtures use
   one imported constructor witness at multiple element types and match the TLC
   evaluator; genuinely non-matchable exports keep the existing source-located
   refusal.
3. **General closed config-overlay lowering.** Remove the current call-shape gate
   for well-typed closed `Patch`/`DeepPatch` values, including computed and
   partially applied overlays plus optional nested records. Keep open rows and
   deletion semantics refused until independently specified. Gate: shallow/deep,
   qualified/destructured, computed, and optional-record cases match the
   reference evaluator through native binaries and shared-library JSON exports.
4. **Measured optimization gate.** Establish repeatable compile-time, runtime,
   allocation, and output-size baselines for the website, configuration/decoder,
   stream, and effectful service workloads. Profile before scheduling an
   optimization. The first optimization milestone must cite the measured
   bottleneck and preserve interpreter/native parity; no speculative thunk,
   memoization, CSE, or closure-specialization project enters the roadmap.

## Long-term: application ergonomics on the stable core

Goal: prove Zutai can carry real applications without weakening `.zti` inertness,
purity, typed host effects, or the TLC/Dataflow Core boundary.

Milestones, in order:

1. **Source modules for the existing host capability set.** Add explicit
   `stdlib.env`, `stdlib.clock`, `stdlib.rng`, and `stdlib.load` wrappers and effect
   aliases over the already implemented host operations. This adds no ambient
   authority and no new runtime operation. Gate: each module has handler-based
   mock coverage plus `run`/native parity, and one application fixture composes
   multiple capabilities through an explicit entry record.
2. **Independent application qualification.** Add a production-shaped workload
   distinct from the website: locked packages, typed inert configuration,
   validation, explicit filesystem/environment/network capabilities, and both
   binary and shared-library deployment. Its acceptance gate is package sync,
   editor analysis, interpreter execution, native output parity, deterministic
   metadata, and supported-target builds from one source graph. This workload is
   the evidence gate for the next two ergonomics milestones.
3. **Constraint-backed collection vocabulary.** After imported higher-kinded
   witnesses compile natively, use repeated abstractions found in the qualification
   workload to define explicit standard-library `Functor` and `Foldable`
   constraints and instances. Start as opt-in modules; changing ambient
   `map`/`fold` or comparator names requires a separate compatibility decision.
   Gate: List/Optional/Result package-boundary examples check, evaluate, and
   compile with coherent witness selection and no regression to specialized
   helpers.
4. **Derived first-order data encoding.** If the qualification workload needs
   typed interchange back to a host or browser boundary, add a `ToData`-style
   constraint and structural derive builder mirroring the supported closed
   `FromData` shapes, without changing `.zti` or tagged-union syntax. Gate:
   `decode (encode value)` round trips scalars, atom singletons, lists, optionals,
   closed records, and closed unions through the evaluator, native JSON bridge,
   and browser bundle; open rows, tuples, recursive targets, fixed-width/posit
   scalars, opaque handles, functions, and `Type` values refuse at the derive
   request.
5. **Demand-gated language boundary review.** Revisit reserved boundaries only
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
