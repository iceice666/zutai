# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-13)

### Finish the typed macro kernel

The first staging/decoder slice has landed locally: `Code A`, hygienic
`quote`/`splice`, a bounded reducer for pure helpers and nested splices, generic
type-checked witness records, and structural `FromData` synthesis. Before
recording the milestone as complete:

- complete pattern-driven pure recipe evaluation and surface fuel exhaustion
  as a source diagnostic;
- add typed rank-2 field/variant descriptors and the compile-time record
  builder to `stdlib.reflect`;
- route `FromData` through that generic recipe API instead of the provisional
  TLC structural synthesizer;
- fix LLVM/native execution of nested derived-record decoders; primitive and
  flat-record binaries run, while nested record decoding still crashes in the
  generated program;
- add expansion definition/request locations to macro diagnostics;
- finish malformed-staging, effect, fuel, recursion, open-row, and residual
  metadata coverage.

The syntax-stabilization pass consolidated the former numbered specifications
by language area and promoted every parser-accepted surface form into the stable
specification. A construct may still have a deliberately narrower execution
envelope. Those limits are support levels, not future language versions.

## Stable-syntax change policy

New surface syntax is not accepted as a speculative placeholder. A syntax
change must include:

- a stable-spec and language-manual update;
- parser coverage and a source-located diagnostic for malformed forms;
- HIR, THIR, and TLC semantics, or a precise documented refusal point;
- reference-interpreter and native support statements; and
- acceptance evidence for every support level claimed.

Compatibility spellings already accepted by the parser remain part of the
stable surface until an explicit deprecation and migration decision removes
them.

## Intentional support boundaries

These are specified behavior, not backlog items:

- Higher-kinded instantiation remains check-only; evaluation and native compile
  refuse unsupported HKT execution.
- Reflection is compile-time. Supported reflection folds before Dataflow Core;
  residual reflection and `Type`-valued program results are rejected.
- Unhandled or ungranted residual host effects are rejected by strict AOT.
- Non-principal row and constraint inference requires explicit annotations.
- Non-matchable cross-module witness exports remain native-gated.
- Non-tail `yield from` remains unsupported; tail delegation is stable.

## Open design question: GitHub-style remote dependencies

The current package model (see
[local packages](../spec/07-modules/modules.md#local-packages)) is
local-path-only: a `zutai.zti` manifest declares dependencies as
package-relative filesystem paths. There is no registry, no remote fetch, no
version solving, and no lockfile. This is deliberate — package resolution
must finish before general-mode name resolution or type checking can start,
so remote fetch is IO the compiler does not currently perform.

A Go-modules-style alternative has been proposed: address a dependency by its
repository location (e.g. a `github.com/...` path) plus a git ref, using git
itself as the distribution mechanism instead of a central package registry.
This fits Zutai's existing "no registry" posture better than an npm-style
model would, but it is not yet a scheduled milestone. Before it becomes one,
the following need concrete answers:

- **Manifest shape.** Dependency entries currently take `{ alias; path; }`. A
  remote entry needs at least `{ alias; url; rev; }` (or `tag`); decide
  whether `path` and `url` are mutually exclusive variants of one dependency
  shape or visibly distinct forms.
- **Fetch and cache.** Something must git-clone/shallow-fetch into a local
  module cache (analogous to `$GOPATH/pkg/mod`) before typed work starts,
  since package resolution is synchronous and pre-typecheck today. This is
  new process/network IO the compiler does not currently do; decide which
  crate owns it and how it stays gated to native builds (see the Wasm point
  below).
- **Version identity.** Choose between pinning an exact commit (simplest,
  most reproducible, closest to the current local-path model), tag-based
  semver ranges with Go-style minimal version selection (more ergonomic, needs
  a solver), or something narrower. `compilerCompatibility` already exists for
  compiler-version gating and is orthogonal to dependency version selection.
- **Lockfile.** Reproducible builds need resolved commit hashes recorded
  somewhere durable; the current spec explicitly excludes lockfiles from the
  local-path model, and a remote model likely cannot skip this the way local
  paths do.
- **Wasm / portable bundles.** Package sources are carried pre-resolved into
  portable web bundles today; the Wasm kernel does no filesystem or network
  lookup (see "Standard-library imports" in the modules spec). Remote fetch
  must stay a native/CLI-side concern that completes before bundle
  construction, never something the Wasm kernel performs itself.
- **Trust surface.** Arbitrary git fetch during compilation is a supply-chain
  surface npm and Go have both had incidents with; decide whether fetched
  sources need pinning-by-hash verification before being treated as trusted
  input to name resolution.

Not scheduled. Add a concrete milestone here only when a real
multi-repository Zutai program needs it.

## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
