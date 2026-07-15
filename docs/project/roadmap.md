# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-15)

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

1. **Package graph in the LSP.** `zutai-cli lsp` should resolve the same
   `zutai.zti` package graph as the CLI analysis path, not only same-document
   or ad-hoc file URIs. Go-to-definition, hover, references, rename, document
   symbols, completion, signature help, and quick fixes must preserve current
   error-tolerant THIR behavior when one imported module fails. Acceptance: an
   editor fixture with two local packages proves imported values, imported
   type-valued members, and unresolved dependency diagnostics all point at the
   same source locations as CLI analysis.
2. **Import provenance everywhere diagnostics cross a file boundary.** Package,
   module, witness, derive, and backend refusal diagnostics should carry both
   request and definition/import-chain locations when available. Acceptance:
   focused CLI/LSP tests cover unknown aliases, duplicate dependency aliases,
   package cycles, non-matchable witness exports, malformed derive recipes, and
   native-gated imports without losing the original use site.
3. **Deterministic package-analysis cache.** Reuse parsed and lowered package
   modules across CLI, LSP, web-bundle construction, and tests without changing
   language semantics. Cache invalidation must be keyed by source content,
   manifest content, stdlib identity, and compiler compatibility, not by ambient
   process state. Acceptance: repeated analysis of a package graph avoids
   duplicate work while still invalidating exactly the changed module and its
   dependents.

Deferred here: remote dependencies. Local packages are sufficient until a real
multi-repository Zutai program needs native fetch, cache, lockfile, and trust
rules.

## Mid-term: backend parity and reproducible native builds

Goal: make native output a boring deployment target for the stable language
subset already accepted by the frontend.

Milestones:

1. **Backend refusal matrix.** Convert every intentional support boundary into a
   small executable fixture: higher-kinded execution refusal, residual
   reflection refusal, unhandled host effects, ungranted capabilities,
   non-principal inference requiring annotations, non-matchable witness exports,
   and non-tail `yield from`. Acceptance: reference-interpreter and strict-AOT
   tests assert the same support statement documented in the manual and status
   page.
2. **Native value-shape parity.** Extend native execution coverage beyond the
   current primitive/record/tuple/union/text/atom/posit matrix to application
   shapes built from decoded `.zti` records, streams, effects-at-boundaries, and
   package imports. Acceptance: each fixture runs through parse -> HIR -> THIR
   -> TLC -> Dataflow Core -> ANF -> SSA -> LLVM and compares `zutai_entry_json`
   against the reference interpreter.
3. **Reproducible native artifacts.** Make the CLI expose enough build metadata
   to explain what was compiled: package roots, stdlib identity, compiler
   compatibility, target triple, relocation mode, and runtime ABI version.
   Acceptance: two clean builds of the same package graph produce explainable
   metadata and no hidden dependency on host working directory layout.

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

## Open design question: GitHub-style remote dependencies

The current package model (see
[local packages](../spec/07-modules/modules.md#local-packages)) is
local-path-only: a `zutai.zti` manifest declares dependencies as
package-relative filesystem paths. There is no registry, no remote fetch, no
version solving, and no lockfile. This is deliberate — package resolution must
finish before general-mode name resolution or type checking can start, so remote
fetch is IO the compiler does not currently perform.

A Go-modules-style alternative has been proposed: address a dependency by its
repository location (for example, a `github.com/...` path) plus a git ref, using
git itself as the distribution mechanism instead of a central package registry.
This fits Zutai's existing "no registry" posture better than an npm-style model
would, but it is not yet a scheduled milestone. Before it becomes one, the
following need concrete answers:

- **Manifest shape.** Dependency entries currently take `{ alias; path; }`. A
  remote entry needs at least `{ alias; url; rev; }` or `{ alias; url; tag; }`;
  decide whether `path` and `url` are mutually exclusive variants of one
  dependency shape or visibly distinct forms.
- **Fetch and cache.** Something must git-clone or shallow-fetch into a local
  module cache before typed work starts, since package resolution is synchronous
  and pre-typecheck today. This is new process/network IO; decide which crate
  owns it and how it stays gated to native builds.
- **Version identity.** Choose exact commits, tag pins, or semver ranges with a
  solver. `compilerCompatibility` already gates compiler compatibility and is
  orthogonal to dependency version selection.
- **Lockfile.** Reproducible remote builds need resolved commit hashes recorded
  somewhere durable; the current local-path model intentionally has no lockfile.
- **Wasm / portable bundles.** Package sources are carried pre-resolved into
  portable web bundles today; the Wasm kernel must continue to do no filesystem
  or network lookup.
- **Trust surface.** Arbitrary git fetch during compilation is a supply-chain
  surface. Decide whether fetched sources need pinning-by-hash verification
  before they become trusted input to name resolution.

Not scheduled. Add a concrete milestone here only when a real multi-repository
Zutai program needs it.

## Reserved design boundaries

The following are demand-gated non-goals rather than a sequenced roadmap:

- GADT-style local type equalities and a coercion/cast core node;
- impredicative instantiation;
- unforgeable capability tokens tied to operation authority; and
- nominal recursive types distinct from structural equirecursive aliases.

See [reserved language boundaries](../design/reserved-language-boundaries.md) for
the design constraints. Add a milestone here only when a concrete program
requires one of these boundaries to move.
