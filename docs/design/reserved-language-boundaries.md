# Reserved Language Boundaries

This document records deliberate non-goals that constrain future Zutai design.
They are not a numbered release plan and do not make currently accepted syntax
provisional. A boundary moves only when a concrete program justifies the added
type-system or runtime cost.

## Committed invariants

- `.zti` remains inert data.
- `.zt` remains pure, lazy, and typed; host interaction is explicit through
  effects and granted capabilities.
- THIR remains error-tolerant and source-preserving. TLC remains fully
  elaborated and is the only input to Dataflow Core.
- Dataflow Core represents sharing and recursion structurally. New syntax must
  elaborate into the existing core unless a separately justified kernel change
  is approved.
- Evaluation never proceeds from incomplete typed IR.
- The native runtime uses conservative default-on collection. A new feature
  must not silently require a moving collector or write barrier.
- Type equality is normalization. Features that need proof-directed coercion
  must justify a new trusted core node explicitly.

## Stable generator decision

`Stream A` is demand-driven codata, not a memoizing lazy list. The stable
`stream { yield ...; }` surface desugars onto that representation. Tail
`yield from` is supported; non-tail delegation is deliberately refused.
Effectful generators run within the dynamic extent of a granting handler, and
`finally` provides cleanup for supported handled-effect shapes.

See the [generator specification](../spec/10-generators/generators.md) for the
language contract and [the history index](../history/README.md) for implementation provenance.

## Demand-gated boundaries

### GADT-style local equalities

Branch-local equalities such as `a ~ Int` require proof-directed casts and a
coercion node in the typed core. They would replace the current “equality is
normalization” invariant with a larger trusted kernel. Do not add GADT syntax
without a concrete use case and a core soundness design.

### Impredicative instantiation

Higher-rank annotations are supported, but inference remains predicative. A
type variable cannot be inferred as a polymorphic type. Impredicative inference
would lose principal types and predictable decidability.

### Unforgeable capability authority

Capabilities currently make host access explicit, locally auditable, and
mockable, but their authority is advisory. Binding a particular value to
authority over a particular operation needs new typing and runtime rules; it is
not merely an implementation hardening task.

### Nominal recursive types

Recursive aliases are structural and equirecursive. Nominal recursion would add
distinct type identity and explicit construction boundaries. Do not layer it on
top of aliases without a concrete interoperability or abstraction need.

## Promotion rule

When a real program requires one of these boundaries, add a scoped milestone to
[the roadmap](../project/roadmap.md) with the motivating program, semantic rule, parser impact,
IR impact, refusal behavior, and acceptance gates. Do not assign a speculative
language-version number.
