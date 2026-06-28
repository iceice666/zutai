# Zutai Language Specification v3

v3 builds on the v0 core language, the v1 deferred feature set, and the v2
deferral horizon. This directory tracks language features beyond the v2 scope.

Start with the [V3 roadmap](02-roadmap.md) for the sequenced plan, the
backend-compatibility invariants that constrain V3 design, and the demand-gated
reserved boundaries.

Current topics:

- [V3 roadmap](02-roadmap.md) — sequenced generator/stream spine (Track 1), the
  codata-not-lazy-lists decision, and the reserved design boundaries (Track 2).
- [Generator and yield syntax](01-generators.md) — implemented `stream { yield ...; }`,
  codata `Stream`, richer `yield`, effectful-generator boundaries, and remaining
  non-goals.
