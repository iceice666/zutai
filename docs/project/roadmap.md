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

1. **DONE. Retained-tree diff for steady-state children reconciliation.**
   Added a pure `diff_children` (`crates/browser/kernel/src/diff.rs`, no
   `web_sys` dependency, unit-tested with plain `cargo test`) that matches
   `new` children against the retained `old` `Document` as data, plus
   `diff_patch_document`/`diff_patch_children`/`diff_patch_element` in
   `dom.rs` that apply the resulting `ChildOp`s to the live DOM. `dispatch`
   now calls `diff_patch_document(&document, &app.rendered, &next_document)`
   instead of reading identity back off the DOM. Hydration's DOM-walking in
   `start`/`patch_document`/`attach_children` is untouched, by design — it
   has no in-memory old tree. `data-zutai-key` still gets written to the DOM
   (hydration needs it); steady-state diffing just stops reading it back.
   As a side effect this already replaced the O(n^2) per-child DOM scan with
   an O(n) hashmap lookup in `diff_children`, ahead of milestone 2 below.
   Attribute application (`apply_element_attributes`) is still the
   clear-and-reapply-everything strategy — milestone 3, not touched here.
2. **DONE. Minimal moves and unkeyed shift tolerance.** `diff_children` now
   computes the longest increasing subsequence over matched old indices
   (`longest_increasing_subsequence`, O(n log n)) and marks those positions
   `ChildOp::Keep` (no DOM move); every other match is `ChildOp::Move`.
   `diff_patch_children` in `dom.rs` applies this with a backward,
   anchor-based walk (each already-placed node becomes the `insertBefore`
   anchor for whatever precedes it), the standard minimal-move keyed-diff
   shape used by Vue/Inferno. Unkeyed matching no longer requires the exact
   same index: old and new unkeyed nodes are grouped into per-kind (text, or
   element-by-tag) FIFO queues, so a mid-list insert or removal re-syncs on
   the next same-kind sibling instead of cascading into replacing everything
   after it — see the `unkeyed_mid_list_insert_...`/`..._removal_...` tests
   in `diff.rs`.
3. **DONE. Attribute-level diffing.** Added `diff_element_attributes`/
   `diff_static_attributes` (`diff.rs`), which build a name -> `AttributeEffect`
   map per element/body (`Text(String)` or `Styles(Vec<Declaration>)`,
   compared structurally so an unchanged stylesheet never gets re-rendered
   just to check equality) and emit only the names that were actually added,
   changed, or removed. `dom.rs`'s `diff_apply_element_attributes`/
   `diff_apply_static_attributes` apply that `AttributeDiff` in place of the
   old `clear_attributes` + reapply-everything loop. `value`/`checked` stay
   excluded from the diff entirely and are still compared against *live* DOM
   state unconditionally on every patch (not old-vs-new declared value),
   because typing or checking a box can diverge the live property from
   whatever was last declared — diffing those the same way as other
   attributes would silently break the "revert an invalid edit" pattern.
   Hydration's `patch_element`/`apply_element_attributes` (no old tree to
   diff against) is untouched.
4. **DONE. Head diffing.** Added `diff_head` (`diff.rs`), reusing the same
   `ListOp`/`ListDiff`/longest-increasing-subsequence machinery `diff_children`
   uses (renamed from `ChildOp`/`ChildDiff` now that a second, unrelated list
   shares them), but matched by full `HeadNode` structural equality instead
   of keys or tags — head nodes carry no independent state, so two equal
   nodes are fully interchangeable regardless of position, and a match never
   needs a content update, only a possible reposition. `dom.rs`'s
   `diff_patch_head` applies it with the same backward, anchor-based walk as
   children. This was the milestone with the user-visible payoff: a render
   that leaves `Document.head` unchanged (the common case) now touches
   `<head>` not at all — no more removing and reinserting the page's
   `<style>` tag, and the reparse/flicker that came with it, on every event.
   Hydration's `patch_head`/`create_head_node` (no old tree to diff against)
   is untouched.
5. **DONE. `wasm-bindgen-test` harness.** Added
   `crates/browser/kernel/tests/browser_hydration.rs`
   (`#![cfg(target_arch = "wasm32")]`, `wasm-bindgen-test = "=0.3.71"` — the
   exact release pinned against this workspace's `wasm-bindgen 0.2.121`/
   `js-sys 0.3.98`), plus `tests/fixture/mod.rs`: a small model/update/view
   Zutai program (toggle, keyed list add/remove, draft input) with its own
   `WebBundleV3`, embedding only the stdlib modules it needs
   (`stream`/`prelude`/`html`/`css`) via `include_str!` so building the
   bundle needs no filesystem access — required since wasm32 has none at
   test-run time, which also rules out `zutai_semantic::analyze`/
   `analyze_path` (both resolve the stdlib from disk unconditionally, even
   for the caller-supplied-`StdlibSources` — the only filesystem-free
   constructor — `StdlibSources::from_memory` — used here and in the real
   `start()`). `tests/browser_program.rs` (native) runs the same fixture
   through `analyze_sources_with_stdlib_and_packages` → decode → init →
   transition → render with no browser needed, which is what actually
   caught and fixed two real Zutai-syntax mistakes in the fixture program
   (a multi-arg call needing explicit parens, and bare tag literals not
   unifying to the declared `Msg` type without a named typed wrapper) before
   ever touching wasm.

   `browser_hydration.rs` drives the real `start()` entry point end to end:
   hydration, keyed-list add (asserting the pre-existing "seed" node's
   *identity* survives via `is_same_node`, not just its content), an
   unrelated toggle re-render (asserting list AND head node identity both
   survive — milestones 1/2/4 together), keyed-list removal (surviving
   sibling keeps its node), and focus/selection restore across a patch.

   Runs via `cargo test --target wasm32-unknown-unknown -p zutai-browser
   --test browser_hydration` (a `[target.wasm32-unknown-unknown] runner =
   "wasm-bindgen-test-runner"` entry was added to `.cargo/config.toml`;
   `wasm-bindgen-cli` already bundles that binary). Needs a headless browser
   + WebDriver on `PATH` — added Chromium + chromedriver to `flake.nix`'s
   devShell, Linux-only (nixpkgs does not package Chromium for Darwin).
   `events.rs`/`website.rs` were given `#![cfg(not(target_arch = "wasm32"))]`
   defensively, since they use the disk-reading `analyze`/`analyze_path` and
   were never meant to run under the wasm32 target.

   Verified in this environment: `cargo check`/`cargo clippy --target
   wasm32-unknown-unknown -p zutai-browser --tests` are clean, and
   `tests/browser_program.rs`'s native run passes, proving the shared
   fixture program and stdlib subset are correct. The automated
   `browser_hydration.rs` scenario itself has not been run through
   `wasm-bindgen-test-runner` in this sandbox (no headless browser here), but
   every behavior it asserts — hydration, keyed-list add/remove, the
   unrelated-re-render node-identity checks, and focus/selection restore —
   was confirmed by hand against the same fixture program (`zutai-web build`
   + a plain static server) in real Chromium, so the reconciler behavior
   itself is confirmed working end to end. Running the automated test
   locally (`cargo test --target wasm32-unknown-unknown -p zutai-browser
   --test browser_hydration` after reloading the dev shell) still gives the
   more precise, repeatable signal and is worth doing once.

   Manual verification surfaced (and this then fixed, once diagnosed) an
   unrelated `zutai-web serve` bug: its file watcher could see its own
   build activity as a source change and rebuild forever, each cycle
   bumping the injected reload script's revision and forcing a full page
   reload — which is what "the page keeps refreshing and loses my typed
   state" turned out to be, unrelated to the reconciler. Two independent
   causes, found by instrumenting `spawn_watcher` and observing real
   `notify` events rather than guessing:

   - `build_site`'s actual writes land in a *staging* directory
     (`out_dir.parent()/<STAGING_DIR_PREFIX><pid>-<hash>`, a sibling of
     `out_dir`, not a descendant) before the final rename into place — so
     an `-o` under the entry's own directory (a natural, common choice)
     meant almost every build-triggered filesystem event was outside the
     one path (`out_dir`) an earlier attempt at this fix checked.
   - Independently of `out_dir`'s location at all: `build_site` reads the
     entry file (and every imported source file) on *every* run to analyze
     it, and the dev-server's `notify` watcher reports pure `Access`
     (open/read/close) events for that read — which a naive "did anything
     under source_root change" check treats as a real edit, so simply
     *building* would perpetually re-trigger itself regardless of where
     output lives.

   Fixed in `crates/web/src/lib.rs`: `spawn_watcher`'s
   `is_relevant_source_change` now (a) never treats `EventKind::Access`
   events as a change, and (b) ignores any event whose paths are entirely
   inside `out_dir` or a `STAGING_DIR_PREFIX`-prefixed staging directory
   (a shared constant, so `build_site` and the watcher can't drift apart on
   the naming convention). Verified empirically, not just by unit test:
   reproduced the original nested `-o dist` layout, confirmed the server
   stays at build-count 1 while idle (previously climbed continuously), and
   confirmed a genuine source edit still triggers exactly one rebuild
   (revision 1 -> 2, stable afterward) — the watcher still works, it just
   no longer answers to its own echo.

Not scheduled beyond this: whole-tree re-render/re-walk on every event (no
per-subtree memoization upstream of the patcher) is a separate, larger
architectural question tied to the render/dataflow layer rather than the DOM
kernel, and should be scoped on its own once 1-4 land and profiling shows it
matters.

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
