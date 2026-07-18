# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-18)

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

The only open milestone is:

1. **Demand-gated language boundary review.** Admit a proposal only when it
   satisfies the investment-policy template above and defines a concrete
   semantic rule, refusal model, migration risk, and executable gate.

## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
