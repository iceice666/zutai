# Zutai Roadmap

Zutai has no numbered language-version roadmap. The currently accepted syntax
is one stable surface specified under the [language specification](../spec/00-index.md).
Implementation history lives under [`docs/history/`](../history/README.md); this file contains
only concrete open work.

## Status (2026-07-13)

### Browser kernel: retained-tree DOM reconciliation

`patch_document` (`crates/browser/kernel/src/dom.rs`) reconciles the *live
DOM* against the newly rendered `Document` on every event; it never reads
`App::rendered`, the previous `Document` value already held in memory.
Identity for keyed children is recovered by reading `data-zutai-key` back off
DOM nodes with a linear scan per keyed child (O(n^2) for a keyed list of
length n), and `patch_element`/`patch_document` unconditionally
`clear_attributes` then reapply every attribute and every managed `<head>`
node on every patch, whether or not anything changed. Hydration (`start`,
patching against build-time prerendered HTML) is the one place a DOM read is
unavoidable, since the first "old tree" only exists as SSR markup — that is
why `render.rs` stamps `data-zutai-key` into the prerendered HTML in the
first place (see `prerenders_semantic_html_and_omits_handlers`), and that
stamping must stay. Everything after hydration has a real old `Document` in
`App::rendered` and should diff against it as plain Rust values instead of
reading the DOM.

Planned milestones, in order:

1. **Retained-tree diff for steady-state children reconciliation.** Add a
   pure `diff_children(old: &[Html], new: &[Html]) -> Vec<ChildOp>` (no
   `web_sys` dependency, testable with plain `cargo test` using the same
   `Html`/`Element` builders `render.rs` and `events.rs` already use) plus a
   thin wasm-only apply step, and wire it into `dispatch`'s patch call.
   Hydration's DOM-walking in `start`/`attach_children` is an explicit
   carve-out and stays as-is. `data-zutai-key` keeps being written to the DOM
   (hydration still needs it); steady-state diffing just stops reading it
   back.
2. **O(n) keyed matching and minimal moves.** Replace per-child linear scans
   with a `HashMap<key, old-index>` built once per sibling list, and compute
   a minimal move set (longest-increasing-subsequence over matched old
   indices) so unmoved keyed children emit zero `insertBefore` calls. Extend
   unkeyed matching to tolerate small positional shifts instead of only the
   exact same index, so a single mid-list insert/remove doesn't cascade into
   replacing every later unkeyed sibling.
3. **Attribute-level diffing.** Compare old vs. new `Element.attributes` and
   `Document.body_attributes` as data and emit only the add/remove/set ops
   that actually changed, replacing the current clear-all-then-reapply in
   `patch_element` and `patch_document`. Keep the existing `value`/`checked`
   special-casing for input cursor/IME safety.
4. **Head diffing.** Diff `Document.head` old vs. new instead of removing and
   recreating every `[data-zutai-managed]` node on every patch in
   `patch_head`. This is the milestone with a user-visible payoff: today
   every event reparses and reinserts the page's `<style>` tag even when the
   rendered CSS is byte-identical.
5. **`wasm-bindgen-test` harness.** Add headless-browser test infrastructure
   (none exists in this crate today — no CI wasm test target) to cover what
   the pure diff tests structurally cannot: hydration itself, focus/selection
   restore (`SelectionSnapshot`), and end-to-end event-dispatch-to-DOM
   scenarios for at least one keyed-list case.

Not scheduled beyond this: whole-tree re-render/re-walk on every event (no
per-subtree memoization upstream of the patcher) is a separate, larger
architectural question tied to the render/dataflow layer rather than the DOM
kernel, and should be scoped on its own once 1-4 land and profiling shows it
matters.

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
