# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-08-25)

Syntax discovery and broad platform construction are complete. Zutai already
has a stable surface plus reference evaluation, native AOT, packages, LSP,
typed staging, browser execution, reflection, and data codecs. Their existence
does not create independent expansion roadmaps: future work defaults to
correctness, diagnostics, security, portability, and concrete data,
configuration, validation, or transformation workloads.

No optimization is scheduled. The recorded website build/toolchain outlier
still requires focused profiling before it can justify compiler or runtime
work.

## Investment policy

### Maintain by default

- fix correctness, soundness, security, data-loss, and portability defects;
- preserve documented stable syntax and support-level contracts;
- improve diagnostics and explicit refusal behavior;
- improve a demonstrated inert-data, validation, transformation, or
  serialization workflow.

### Demand-gated

New syntax, host operations, native targets, package source kinds, browser
abstractions, type-system or metaprogramming capabilities, runtime/collector
requirements, and standard-library modules require roadmap promotion. Existing
implementation alone is not evidence for further generalization.

Every proposed milestone must state:

1. the concrete program and blocked workflow;
2. how the request serves data, configuration, validation, or transformation;
3. why a library, tooling change, or host adapter is insufficient;
4. parser, HIR, THIR, TLC, Dataflow Core, runtime, and compatibility impact;
5. support levels and refusal behavior for unsupported shapes;
6. an executable validation gate and the permanent maintenance obligation.

### Frozen without workload evidence

- browser-framework and package-ecosystem expansion;
- IDE feature completeness beyond correctness of existing operations;
- generic macro/staging and effectful-generator generalization;
- higher-rank or higher-kinded backend generalization; and
- optimization without focused profiling.

### Explicitly deferred

Package registries and version solvers, asynchronous or binary host IO, an
application HTTP/database framework, a general procedural-macro system, a
second iterator abstraction, optimizing laziness beyond the current sharing
model, general non-tail `yield from`, and speculative syntax are not scheduled.

## Demand-gated application qualification

Use concrete downstream applications to identify the smallest missing
capability in the stable data-transformation core. The existing independent
qualification application remains the acceptance baseline; a broader
application feature is not a goal by itself.

The open milestones, in order, are:

1. [Typed concurrency and temporal model checking](#typed-concurrency-and-temporal-model-checking)
2. [Demand-gated language boundary review](#demand-gated-language-boundary-review)

### Typed concurrency and temporal model checking

Extend the existing bounded explicit-state checker from state safety and
existential reachability to the smallest graph analysis needed for Slime OS
IPC, supervision, lifecycle, and authority models. The demonstrated BootState
model remains the compatibility baseline; the blocked workflow is expressing
unexpected deadlock, transition-authority rules, and "whenever P, eventually
Q" obligations with actionable counterexamples.

This is a typed validation workflow over ordinary pure `.zt` transition-system
values. A `.zt` library can construct predicates and model records, but it
cannot observe the complete reachable graph, detect strongly connected
components, or distinguish an exhausted state budget from a proof. The work
therefore belongs in the `zutai-model` host checker and `zutai model-check` CLI,
not in language syntax or an ambient host operation.

The host-decoded extension uses optional model fields so existing records remain
valid:

- `terminal` is a callable from state to `Bool`;
- `actionSafety` is a list of named curried callables over state, action, and
  successor state; and
- `leadsTo` is a list of named premise and consequence state predicates.

These names are model-checker data fields, not keywords or syntax. Function
application remains ordinary curried Zutai application.

Required scope:

- add the optional `terminal` predicate; a reachable state with no successors
  fails as an unexpected deadlock unless the predicate holds, and a state for
  which `terminal` holds must have no generated successors. Absence of the
  field preserves the current model contract; a non-empty `leadsTo` list
  requires `terminal`, so finite maximal paths have an explicit model-defined
  interpretation;
- add named `actionSafety` invariants over `(before, action, after)` and evaluate
  them on every generated transition before duplicate-state elimination;
- retain reachable graph edges, compute strongly connected components, and
  check named `leadsTo` obligations as `always (premise implies eventually
  consequence)` over every maximal path, without fairness assumptions;
- treat declared terminal states as stuttering states for temporal semantics;
  an unexpected deadlock fails before a temporal verdict, and state-budget
  exhaustion remains inconclusive;
- report deterministic shortest prefix counterexamples for deadlock and action
  failures, and deterministic prefix-plus-cycle lasso counterexamples for
  cyclic `leadsTo` failures; a terminal temporal failure ends at the declared
  terminal state;
- report distinct states, generated transitions, duplicates, maximum BFS
  depth, terminal states, unexpected deadlocks, SCC count, and largest SCC;
  counters describe the explored graph and never convert a bounded run into a
  proof; and
- preserve existing models, deterministic source-order traversal, first-order
  state/action refusal, expected safety violations, and exit-code behavior.

Semantic and compatibility boundary:

- `safety` remains the existing state-level `always` mechanism; this milestone
  adds no duplicate `always` property form and does not reinterpret current
  existential `reachability` obligations;
- each `leadsTo` obligation quantifies all reachable occurrences of `premise`
  and all maximal paths from each occurrence. In the finite explored graph, a
  violation is a path that avoids `consequence` and reaches either a declared
  terminal state or a reachable cycle that can remain outside `consequence`
  forever;
- new model fields are optional and decoded only by the host checker. Parser,
  HIR, THIR, TLC, Dataflow Core, SSA, LLVM, native runtime, and stable syntax
  receive no new node, lowering rule, or support claim; and
- unsupported callable shapes, non-Boolean predicate results, non-first-order
  states/actions, malformed transition records, and incomplete typed IR must
  refuse explicitly. Hash collisions, pruning, or state-budget exhaustion must
  never produce a safe verdict.

Explicit non-goals are weak or strong fairness, symmetry reduction, state
constraints that silently prune successors, partial-order reduction, monitor
automata, refinement checking, parallel or disk-backed exploration, compact
hash-only state identity, symbolic execution, SMT solving, refinement types,
proof terms, and tactics. Admit any of these only through a later workload-led
milestone with its own semantic and false-safe analysis.

Validation and maintenance:

- `cargo test -p zutai-model` covers deadlock/terminal boundaries, action
  invariants before deduplication, SCC classification, universal `leadsTo`
  semantics, deterministic finite and lasso counterexamples, malformed-model
  refusal, and state-limit inconclusiveness;
- `cargo test -p zutai-cli --test cli model_check` covers rendered reports and
  stable success, failure, inconclusive, and error exits;
- from the Slime OS repository root, `just bootstate_model_check` preserves the
  existing 4,744-state BootState verdicts before the downstream model adopts
  any new property; and
- the permanent obligation is deterministic, collision-safe verdicts and
  counterexamples across supported first-order values. Any future search or
  storage optimization must demonstrate verdict parity against these gates.

### Demand-gated language boundary review

Admit a proposal only when it satisfies the investment-policy template above
and defines a concrete semantic rule, refusal model, migration risk, and
executable gate.

## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
