# Zutai Implementation Status

This page records the current implementation baseline and validation notes. It
is descriptive rather than normative: stable language behavior lives in the
[language specification](../spec/00-index.md), while unfinished work belongs in
the [roadmap](roadmap.md).

## Compilation pipeline

```text
Source → HIR → THIR → TLC
                        ↓  TLC→DC: tree-to-graph, sharing, recursion explicit
                   Dataflow Core
                        ↓  DC→ANF: topo-sort SCCs, name every node, let/letrec
                       ANF
                        ↓  ANF→SSA: basic blocks, phi-nodes
                       SSA
                        ↓  SSA→LLVM
                    LLVM IR
```

- **THIR** is the error-tolerant, source-preserving typed IR and the output of
  `check`.
- **TLC** is produced only after successful type checking. It has explicit
  polymorphism (`TyLam`/`TyApp`) and resolved inference variables.
- **Dataflow Core → ANF → SSA → LLVM** is the production AOT pipeline. Laziness
  and sharing are structural in Dataflow Core, not runtime thunks.
- **`zutai-eval`** is the reference semantics oracle. The default `run`/`repl`
  path is TLC-first for executable value programs; THIR remains the regression
  oracle and runtime `Type`/reflection boundary.

Design details: [`docs/compiler/tlc.md`](../compiler/tlc.md),
[`docs/compiler/dataflow-core.md`](../compiler/dataflow-core.md), [`docs/compiler/anf.md`](../compiler/anf.md), and
[`docs/compiler/runtime-abi.md`](../compiler/runtime-abi.md).

## Current baseline

The 2026-07-17 canonical-formatting baseline adds idempotent `.zt` and `.zti`
formatters exposed through `zutai format` and LSP document formatting. General
mode preserves comments, compatibility spellings, token order, and line
boundaries while normalizing delimiter indentation and line endings; immediate
mode preserves parsed field and item order. Unit, CLI, protocol, tracked-source,
and specification-fence gates cover parse preservation and second-pass byte
equality.

The 2026-07-17 stable-diagnostic baseline gives parser, HIR, import, THIR,
derive, and backend-gate diagnostics stable semantic identities. CLI and LSP
renderers preserve each code, severity, primary source range, and related
cross-file locations; parser-authored unambiguous fixes remain available as LSP
quick fixes. Backend-refusal and cross-file diagnostic matrices now assert the
machine-readable contract instead of relying on message text alone.

The 2026-07-17 import-aware editor baseline completes package aliases and public
module paths from the prepared package graph, completes exported members from
the analyzed import target, and searches root, imported, and otherwise-unopened
public dependency modules with deterministic source locations. Filesystem
recording eagerly captures every public module without network access; package
fixtures cover nested module namespaces, transitive symbols, visibility,
shadowing, malformed-package fallback, and unsaved overlays.

The 2026-07-17 package-wide editor baseline extends references and rename across
the root package and transitive dependencies using the same recorded package
graph and unsaved overlays as checking. Value and type member references follow
exact re-export origins without crossing shadowed or unrelated members; rename
updates intermediate re-export fields, refuses builtins/generated bindings and
immutable locked-Git snapshots, and is regression-tested by applying the edits
and rechecking the renamed three-package project.

The 2026-07-17 self-hosted website integration baseline makes the official site
a local package with a package-owned demo domain and one portable web bundle
shared by native and Wasm browser coverage. Native tests compare filesystem and
in-memory package-graph analysis, prerendering, and interactions; the WebDriver
scenario hydrates that same fixture and proves events, keyed reordering,
controlled inputs, focus effects, and package-backed updates. Development serve
keeps the last successful revision during failed rebuilds and reloads after
recovery; a live browser smoke test covers the watcher and reload path.

The 2026-07-17 standard-library ergonomics baseline adds application-shaped
examples for records, tagged unions, streams, nested `FromData` derivation, and
an explicit `Load` capability, with reference-interpreter/native output parity.
A typed browser example and shared native/Wasm fixture cover `stdlib.html`,
`stdlib.css`, and `stdlib.browser`; CLI checks, LSP diagnostic fixtures, safe CSS
rendering, and `zutai-web build` lock each surface to its documented support
level without changing language syntax.

The 2026-07-16 reproducible native-artifact baseline adds deterministic
`compile --metadata <path>` JSON containing the logical package roots,
package-graph and stdlib identities, compiler compatibility, target triple, PIC
relocation model, artifact kind, and explicit runtime ABI version. Native
binary/library compilation resolves a prebuilt target runtime from
`ZUTAI_RUNTIME_ARCHIVE` or the executable-relative installation and no longer
invokes Cargo or depends on the caller's working directory; the install recipe
ships the matching target archive. Cross-checkout CLI coverage proves identical
metadata for the same package graph without absolute checkout paths.

The 2026-07-16 native value-shape parity baseline extends full
parse-to-LLVM execution coverage to decoded `.zti` records, finite stream
results, source-handled effects at the value boundary, and local package
imports. A shared-library host matrix compares both exported JSON paths —
`zutai_to_json(zutai_entry(), zutai_entry_descriptor())` and
`zutai_entry_json()` — with the reference interpreter and exact expected JSON
shapes for every fixture.

The 2026-07-16 backend refusal matrix locks seven intentional support boundaries
into executable fixtures. CLI coverage now exercises higher-kinded witness
execution, residual reflection, unhandled effects, ungranted capabilities,
non-principal row inference, non-matchable imported witness exports, and
non-tail `yield from` through the applicable `check`, reference-interpreter
`run`, and strict-AOT `compile` paths.

The 2026-07-16 locked Git package baseline adds manifest format 2 path/Git
sources, deterministic root-scoped lockfiles, content-addressed package nodes,
and immutable tree-hashed snapshots. Native `zutai-cli package sync`, `fetch`,
and `update` are the only acquisition entry points. Analysis, LSP, compilation,
and browser execution remain network-free and accept only a validated prepared
graph; portable browser bundles replay the same logical package/source locations
as filesystem analysis. Git acquisition isolates ambient configuration, supports
offline cache reuse and monorepo subdirectories, and refuses stale manifests,
missing snapshots, unsafe paths, and tampered sources.

The 2026-07-16 deterministic package-analysis cache reuses completed imported
module analyses across CLI checks/compilation, LSP sessions, and web rebuilds.
Callers own the process-local cache; entries are keyed by stable module identity
and analysis options, then validated against source content, package manifests
and graph structure, the complete stdlib identity, compiler compatibility, and
transitive module/data dependencies. Cache hits preserve recorded filesystem,
package, and explicit-stdlib sources for portable replay; changing one module or
data import invalidates that module and its dependents while independent modules
remain reusable.

The 2026-07-14 typed macro kernel completes the typed-staging/decoder slice:
compile-time-only `Code A`, hygienic direct/bound `quote(expr)` / `splice(expr)`,
generic quoted-record derive recipes, and an ambient `FromData`/`decode`
structural decoder routed through the generic `deriveFromData` reflection
builder. Pure recipe evaluation reduces `match`/recursion through a bounded
compile-time reducer that never evaluates builtins; fuel exhaustion, irreducible
`Code` recipes, effect-carrying recipe bodies, and open-row structural derives
are all refused with source-located diagnostics carrying request and definition
locations. `stdlib.reflect` exposes typed rank-2 field/variant descriptors and a
compile-time record builder. Supported decoders accumulate path-aware
record/list errors and lower to ordinary TLC terms; missing physical optional
fields decode as absent, and no `Code` node or decoder runtime primitive reaches
Dataflow Core. Reference/TLC evaluation supports nested records and unions, and
LLVM/native execution is verified for primitive, flat-record, and nested-record
decoders, the last via a native oracle test that decodes a nested record with a
list-of-records against the interpreter.

_Last updated: 2026-07-17 (import-aware completion and workspace symbols:
package aliases, nested public modules, imported members, and unopened dependency
symbols use the prepared recorded graph with exact overlay-aware locations);
prior baseline updates: 2026-07-17 (package-wide editor references and safe rename:
cross-package value/type references follow exact re-export origins through
unsaved overlays, rename preserves a valid project and refuses locked-Git or
non-source bindings);
prior baseline updates: 2026-07-17 (self-hosted website integration: the official site is
a local package, native and Wasm browser tests consume the same portable package
bundle, WebDriver covers hydration/interactions, and live serve reloads after a
successful rebuild while preserving the last good revision across errors);
prior baseline updates: 2026-07-17 (standard-library ergonomics: application-shaped
record/union/stream/`FromData`/host-capability examples now have CLI,
reference-interpreter, and native parity coverage; HTML/CSS/browser docs,
diagnostics, fixture rendering, and web-build support levels are executable);
prior baseline updates: 2026-07-16 (reproducible native artifacts: deterministic build
metadata records logical package roots, package/stdlib identities, compiler
compatibility, target/PIC mode, artifact kind, and runtime ABI version;
binary/library links consume a prebuilt explicit or installed runtime archive
without invoking Cargo, and cross-checkout builds produce identical metadata);
prior baseline updates: 2026-07-16 (native value-shape parity: decoded `.zti` records,
finite stream results, handled-effect boundaries, and local package imports now
run through parse -> HIR -> THIR -> TLC -> Dataflow Core -> ANF -> SSA -> LLVM;
both shared-library JSON exports match the interpreter and exact expected shapes);
prior baseline updates: 2026-07-16 (backend refusal matrix: seven executable fixtures now
lock the documented frontend, reference-interpreter, and strict-AOT boundaries
for higher-kinded witnesses, residual reflection, effects/capabilities,
non-principal inference, imported witness exports, and non-tail delegation);
prior baseline updates: 2026-07-16 (locked Git package acquisition: manifest
format 2, deterministic root lockfiles, content-addressed nodes/snapshots,
isolated native `package sync`/`fetch`/`update`, network-free prepared-graph
consumers, portable CLI/browser source identities, offline reuse, and
stale/missing/tampered-source refusal are implemented and covered by hermetic
Git fixtures);
prior baseline updates: 2026-07-16 (deterministic package-analysis cache:
imported-module analyses are reused by CLI, LSP, and web rebuild lifetimes
through an explicit caller-owned cache. Source, manifest/graph, complete stdlib,
compiler, analysis option, and recursive dependency fingerprints gate every hit;
cache replay preserves portable filesystem/package/stdlib recording. Focused
fixtures cover unchanged-graph hits, exact dependent invalidation for `.zt` and
`.zti` changes, manifest invalidation, and cached recording completeness);
prior baseline updates: 2026-07-16 (cross-file import diagnostics: package setup
and resolution, module cycles, imported witness conflicts, derive failures, and
native-only import refusals now retain request, definition, manifest, and
import-chain locations where available. CLI miette output reads the owning
source for every primary and related span; LSP routing publishes diagnostics to
the owning URI with related information for the rest of the chain. Focused
semantic/CLI/LSP fixtures cover unknown aliases, duplicate dependency aliases,
package and module cycles, non-matchable witness exports, malformed derive
recipes, and native-gated imports without losing the original use site);
prior baseline updates: 2026-07-16 (package-aware LSP: editor analysis records
and replays the CLI package graph, maps stable package identities to filesystem
URIs, consumes dependency overlays, and navigates imported value and type-valued
members; malformed package setup survives overlay fallback and CLI parity is
covered);
prior baseline updates: 2026-07-14 (typed macro kernel: completed the six-bullet
macro-kernel milestone — pattern-driven pure recipe evaluation with
source-located fuel exhaustion; typed rank-2 reflection descriptors and a
compile-time record builder in `stdlib.reflect`; `FromData` routed through the
generic `deriveFromData` builder; nested derived-record decoders fixed for
LLVM/native by lifting `Letrec` bindings to globals, proven by a native oracle
test that decodes a nested record with a list-of-records against the
interpreter; request/definition locations on macro diagnostics; and the
malformed-staging/effect/fuel/recursion/open-row/residual refusal envelope,
including a new `DeriveOpenRowTarget` diagnostic and irreducible-`Code`-recipe
refusal. The recipe reducer stays pure-structural and never evaluates builtins,
so an arithmetic-driven recipe refuses rather than mis-deriving a witness);
prior baseline updates: 2026-07-13 (`zutai-web serve`: fixed a dev-server watcher bug found
while manually verifying the browser kernel reconciler (below) — the file
watcher could see its own build activity as a source change and rebuild
forever, each cycle forcing a full page reload. Two independent causes,
found by instrumenting `spawn_watcher` and reading real `notify` events
rather than guessing: `build_site`'s writes actually land in a staging
directory that is a *sibling* of `out_dir`, not a descendant, so an `-o`
under the entry's own directory put almost every write outside the one path
a filter on `out_dir` alone would catch; and, independently of `out_dir`'s
location, `build_site` reads the entry file on every run, which the watcher
reports as a pure `Access` event that a naive "did anything change" check
treats as a real edit. `crates/web/src/lib.rs`'s `is_relevant_source_change`
now excludes `EventKind::Access` outright and recognizes the staging
directory by its `STAGING_DIR_PREFIX` (a constant shared with `build_site`
so the two can't drift apart). Verified empirically, not just by unit test:
reproduced the original nested `-o dist` layout, confirmed the server holds
at build-count 1 while idle, and confirmed a genuine source edit still
triggers exactly one rebuild), 2026-07-13 (browser kernel reconciliation, milestone 5: added a
wasm-bindgen-test browser harness (`crates/browser/kernel/tests/
browser_hydration.rs`, `wasm-bindgen-test = "=0.3.71"` pinned to match this
workspace's `wasm-bindgen 0.2.121`) exercising `start()` end to end —
hydration, keyed-list add/remove with DOM node *identity* assertions
(`is_same_node`, not just content), an unrelated re-render leaving list and
head nodes untouched, and focus/selection restore — against a small fixture
Zutai program (`tests/fixture/mod.rs`) whose `WebBundleV3` embeds only the
stdlib it needs via `include_str!` (no filesystem access, required for
wasm32); a native counterpart (`tests/browser_program.rs`) runs the same
program through analyze/decode/init/render without a browser and is what
actually caught two Zutai-syntax mistakes in the fixture before it ever
reached wasm. `.cargo/config.toml` now points the wasm32 target's runner at
`wasm-bindgen-test-runner`, and `flake.nix`'s devShell gained Chromium +
chromedriver (Linux-only). The automated scenario has not been run through
`wasm-bindgen-test-runner` in this environment (no headless browser here),
but every behavior it asserts was confirmed by hand in real Chromium against
the same fixture program), 2026-07-13 (browser kernel reconciliation, milestone 4: `diff_head` diffs
`Document.head` by full `HeadNode` structural equality (head nodes carry no
independent state, so equal nodes are fully interchangeable regardless of
position), reusing the `diff_children` move-minimization machinery renamed
`ListOp`/`ListDiff` now that it backs two unrelated lists; a render that
leaves the head unchanged now touches `<head>` not at all, removing the
per-event `<style>` teardown/reparse; `dom.rs`'s `diff_patch_head` applies it
via the same backward, anchor-based walk as children), 2026-07-13 (browser kernel reconciliation, milestone 3: `diff_element_attributes`/
`diff_static_attributes` compute an add/change/remove `AttributeDiff` from a
structural name -> `AttributeEffect` map (style declarations compared
without rendering), replacing the old clear-and-reapply-every-attribute
patch for both element attributes and `Document.body_attributes`;
`value`/`checked` stay excluded from the diff and are still compared against
live DOM state unconditionally, since typed/checked input can diverge from
the last declared value), 2026-07-13 (browser kernel reconciliation, milestone 2: `diff_children`
now selects a minimal DOM move set via a longest-increasing-subsequence pass
over matched old indices, and unkeyed sibling matching moved from
exact-position to per-kind FIFO queues (text, or element-by-tag), so a
mid-list unkeyed insert/removal re-syncs on the next matching sibling
instead of cascading into replacing everything after it; applied in `dom.rs`
via a backward, anchor-based patch walk), 2026-07-13 (browser kernel reconciliation, milestone 1: steady-state
DOM patching now diffs the retained previous `Document` (`App::rendered`)
against the newly rendered one as plain data, via a new pure `zutai-browser`
`diff` module (`diff_children`, unit-tested without a wasm target), instead of
reading child identity back off the live DOM; hydration is unchanged and still
DOM-walks the prerendered document, since it has no in-memory old tree; this
also incidentally gave keyed matching O(n) hashmap lookup instead of an O(n^2)
per-child DOM scan), 2026-07-13 (browser DOM event expansion: `stdlib.html`
`EventHandler` gains `#change`/`#submit`/`#blur`/`#focus`/`#keyDown`/`#keyUp`
variants alongside the existing `#click`/`#input`, with matching
`onChange`/`onSubmit`/`onBlur`/`onFocus`/`onKeyDown`/`onKeyUp` (and `*With`
options-taking) constructors; the browser kernel decodes the new tags,
listens for the corresponding native DOM events (`change`/`submit`/`blur`/
`focus`/`keydown`/`keyup`), and extracts `Text` payloads from `<select>`
elements and `KeyboardEvent.key` for the key handlers; see
`crates/browser/kernel/tests/events.rs`), 2026-07-13 (stdlib package independence: the
filesystem stdlib tree moved out of the Rust workspace to a top-level
`stdlib/` directory, the loader (`StdlibSources`, manifest parsing, root
resolution) was folded into `zutai-semantic`, and the now-empty
`zutai-stdlib` crate was removed), 2026-07-13 (local package system and split
filesystem stdlib), 2026-07-13 (cross-file `.zti` validation diagnostics),
2026-07-12 (filesystem-only stdlib and portable stdlib bundles),
2026-07-12 (dedicated `zutai-web` CLI),
2026-07-12 (unversioned stable syntax specification),
2026-07-12 (browser kernel and self-hosted website baseline),
2026-07-12 (LSP editor baseline),
2026-06-23 (language specs, Unicode XID, evaluator/backend hardening),
2026-06-24 (Phase A: `.zt`/`.zti` native module-import lowering), 2026-06-26
(general-mode `;`-terminator / container-glyph grammar; docs migrated; `import`
unified as an expression; **native effect parity**), and 2026-06-27 (resource
lifetime for effectful generators; dynamic `load.zti` / `load.zt` host effects;
GC residual retired with conservative default-on GC as the committed endpoint),
and 2026-06-28 (post-V3 readiness audit: user-facing doc reconciliation,
support-level reconciliation, coverage/diagnostics/backend audit; ambient vs
imported stream-combinator convergence follow-up fixed; small function prelude
`id`/`const`/`compose`/`flip` landed as ambient source decls + `stdlib.prelude`;
minimal `List` verbs landed as ambient/importable source decls backed by list
patterns and a strict fold bridge; explicit `stdlib.optional` helpers landed for
`map`/`andThen`/`filter`/`withDefault`/`isSome`/`toList`; explicit
`stdlib.result` landed for `Result`/`Validation` errors-as-data helpers;
explicit `stdlib.num` landed for `min`/`max`/`abs`/`clamp`/`pow`/`rem`/`gcd`/
`toFloat`/`round`/`truncate` with checked scalar bridge intrinsics and native
runtime helpers; explicit `stdlib.text` landed for `length`/`split`/`join`/
`trim`/`toUpper`/`toLower`/`contains`/`replace`/`show`/`parseInt`/`parseFloat`
with text bridge intrinsics and native runtime helpers; explicit `stdlib.cmp`
landed for `Ordering`, comparator combinators, and concrete Int/Float/Text
comparators; native-link test race fixed with a process-released runtime-build
lock; release slice R0 added a single CLI acceptance pack that gates check/run/
native-compile parity across the shipped V3 + stdlib-H envelope; see "Post-V3
readiness audit", "Ambient/imported stream-combinator convergence", "Small
function prelude (stdlib slice B)", "Minimal List verbs (stdlib slice C)",
"Optional helpers (stdlib slice D)", "Result and Validation helpers (stdlib
slice E)", "Numeric helpers (stdlib slice F)", "Text helpers (stdlib slice G)",
"Comparator helpers (stdlib slice H)", "Native-link test race fix", and
"Release acceptance pack (release slice R0)"), and 2026-06-30 (native SSA
pattern tests now short-circuit variant/list payload destructuring before
binding; real examples check/run/native-compile parity is covered; see "Native
pattern-test short-circuiting" below; DX polish added row diagnostic field
context, nested-conditional parser help, `just native-examples`, and
real-program style docs; see "DX polish slice" below; Unicode source/native
hardening made `.zt` Unicode whitespace/comments diagnostic-safe and made LLVM
name emission ASCII-safe for Unicode source identifiers; native effectful
generator host-boundary parity now covers standard host operations in lazy cells
while preserving custom/`io.print` handler parity; see
"Unicode source/native hardening" and "Native effectful-generator
host-boundary parity" below; native Text equality now compares runtime text
contents instead of heap object identity; descriptor-backed native structural
equality now covers records, lists, tuples, atoms, variants, and floats; native
Float arithmetic/order now uses `f64` runtime helpers instead of integer ops on
bit patterns; native Text ordering now compares text contents instead of pointer
words; native logical `&&`/`||` now lower to control-flow matches instead of
eager bitwise ops; defaulting `??` now lowers to control-flow matches so present
values skip fallback evaluation; see "Native Text equality parity", "Native Text
ordering parity", "Native structural equality parity", "Native Float scalar
parity", "Native logical short-circuit parity", and "Native coalesce fallback
parity" below; native atom-pattern tests now compare atom text contents so
runtime-created host values such as `#none` match static atom patterns)._
The same 2026-06-30 native parity pass also stores optional record fields as
native `Maybe` envelopes, lowers `?.` to control-flow matches, and teaches
runtime record rendering/equality to skip `#absent` fields and unwrap
`#present` payloads; see "Native optional-field presence parity" below.
The same 2026-06-30 baseline also included the explicit stdlib expansion:
`stdlib.config`, `stdlib.reflect`, `stdlib.list`, `stdlib.data`, and
`stdlib.validate` were embedded importable modules, with config/reflect compiler
gates recognizing qualified and destructured aliases; see "Explicit stdlib
expansion" below. A follow-up stdlib crate extraction moved embedded `.zt`
sources and module metadata into `zutai-stdlib`, while preserving the old
`zutai_hir::*_MODULE_SRC` Rust re-exports and all user-facing import behavior;
see "Stdlib crate extraction" below. The 2026-07-12 filesystem-only stdlib
baseline superseded that storage mechanism with a filesystem registry; the
2026-07-13 package baseline then made root `zutai.zti` canonical,
native tools load `.zt` sources from the selected stdlib root, HIR receives
ambient prelude source explicitly, and web bundle v2 transports the exact
resolved stdlib set to Wasm. There is no embedded fallback.
The 2026-07-13 package baseline adds inert `zutai.zti` manifests, explicit
public module maps, local path dependency aliases, transitive per-package
resolution, package-graph diagnostics, and portable package metadata/sources in
web bundle format v3. Existing quoted imports and `stdlib.*` imports are
unchanged. The filesystem stdlib now uses a root `zutai.zti` compatibility
index and is physically split into `base`, `data`, `system`, and `web` package
units, each with its own manifest.
The 2026-07-01 baseline adds native shared-library artifacts:
`compile --emit=lib` links a platform shared library that exports raw, descriptor,
and JSON entry points, with the JSON path backed by the runtime's descriptor
walker and `serde_json`; see "Native library artifacts and JSON bridge" below.
The 2026-07-07 baseline adds the scoped text filesystem IO slice: opaque
`Reader`/`Writer` support types, explicit `stdlib.fs`, granted host operations
for line readers and text writers, runtime handle tables, and render gates for
opaque handles; see "Scoped filesystem IO foundation" below.
The same 2026-07-07 usability pass adds named effect-row alias spreads and
stdlib filesystem effect aliases, so large effect rows can be factored without
changing their checked operation set; see "Effect-row alias spreads" below.
The same 2026-07-07 stdlib network pass adds `Net` as an explicit host
capability type and `stdlib.net` as the source module for existing TCP
`net.listen`/`net.accept`/`net.read`/`net.write`/`net.close` host effects; see
"Explicit network helpers" below.
The 2026-07-08 stdlib network scoped-lifetime pass adds `net.withConnection`
as a minimal `finally`-backed helper for one accepted TCP connection; see
"Scoped network connection helper" below.
The 2026-07-08 ergonomics pass adds ambient/importable prelude `not` and the
general-mode `%` integer remainder operator; see "Boolean/remainder ergonomics"
below.
The same 2026-07-08 parser-sugar pass adds value-level field sections
`_.field` and `_?.field`, desugaring to ordinary lambdas; see
"Field-section shorthand" below.
The same 2026-07-08 conditional-sugar pass makes `cond { guard => expr; _ =>
fallback; }` the canonical source form for expression conditionals, desugaring
to the existing core `if`/`else` AST; see "Cond expression sugar" below.
The same 2026-07-08 ergonomics pass adds interleaved do-block bindings, opt-in
list rollup helpers, and `stdlib.optional.isNone`; see "Do-block and stdlib
ergonomics" below.
The same 2026-07-08 import-keyword decision keeps `import` as the only static
import spelling and leaves `use` available as an ordinary identifier; see
"Import keyword decision" below.
The same 2026-07-08 stdlib helper slice adds opt-in list/stream search/extrema
and Result/Validation convenience helpers as pure source exports; see
"Stdlib helper slice" below.
The 2026-07-12 editor tooling pass adds `zutai-cli lsp`: a stdio LSP service
with incremental diagnostics, THIR-derived hover/signature types, and
HIR-derived navigation, rename, symbols, completion, and parser quick fixes.
The 2026-07-16 package-aware pass routes filesystem and overlay analysis through
the same recorded local-package graph as CLI analysis, including imported value
and type-valued member navigation across package boundaries; see "Language
Server Protocol editor baseline" and "Package-aware LSP analysis" below.
The 2026-07-16 import-provenance pass gives package setup/resolution, module
cycles, imported witnesses, derive failures, and native backend refusals
structured request and cross-file related locations. CLI diagnostics render the
owning source buffers, while the LSP routes primaries to their source URIs and
uses `relatedInformation` for manifest declarations, import chains, and witness
definitions.

The 2026-07-13 diagnostic-provenance pass keeps the immediate runtime AST
source-free while adding an opt-in located `.zti` parse tree. Static type
mismatches discovered when a `.zt` typed boundary consumes imported `.zti`
data now retain the offending data span: `zutai-cli check` renders the `.zti`
source, and the LSP publishes the diagnostic to the `.zti` URI with related
information pointing back to the `.zt` boundary. Open-document overlays and
open-root reanalysis make unsaved `.zti` fixes clear those diagnostics;
pure runtime validators remain ordinary execution and are not run by `check`.
The same 2026-07-12 browser pass adds an interpreter-backed WebAssembly kernel,
typed browser/HTML/CSS stdlib modules, deterministic prerendered bundles,
the dedicated `zutai-web build` / `serve` app (also exposed through the
compatibility `zutai-cli web` subcommand), and a pure-Zutai official website; see
"Browser kernel and self-hosted website" below.

- General-mode (`.zt`) surface grammar now uses `;` as the universal
  terminator/separator: every value-like top-level declaration ends in `;`, and a
  trailing `;` makes an expression a `()` statement. The container glyph picks the
  shape — `{ … }` is a parallel record (`name = value;`) or list (bare `value;`),
  and `[ … ]` is a serial do-block (local bindings + tail). The scope picks the
  binding operator — top-level `::=` / `:: T =`, local (inside `[ … ]`) `:=` / `: T =`.
  Local do-block bindings may appear after earlier statement expressions and
  scope over only the following statements. Empty record `{}`, empty list `{;}`,
  empty do-block `[]`. Immediate mode `.zti` is unchanged
  (arrays stay `[ … ]`). v0 spec docs, the language manual, and stdlib notes were
  migrated to this grammar; the `v0_spec` doc-fence acceptance test was updated
  to the new accepting set (decl-only `.zt` snippets now form complete programs
  that evaluate to `()`).
- Immediate mode parses `.zti` data through selectable parser backends
  (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and
  elaborates complete programs to TLC.
- THIR covers the stable language semantics:
  row-polymorphic records/unions, `select`, constraints/witnesses, `derive`,
  method-level type params, higher-kinded constraints, algebraic-effect typing,
  named effect-row alias spreads, higher-rank annotation checking, predicative
  inference, guarded recursive type aliases, stream-backed generator sugar, and
  standard host capability/effect-row checking.
- TLC covers row variables, effect rows, explicit dictionary passing, witnessed
  operator lowering, source effect markers, higher-rank `ForAll`/`TyLam`/`TyApp`
  elaboration, CPS elaboration for handled effects, equirecursive alias identity,
  runtime `io.print` lowering through ordinary TLC function values, and residual
  host-effect grant gating before Dataflow Core.
  Constraint-attached derive recipes are stored through Syntax/HIR/THIR and
  drive specialized TLC Show/Ord dictionary synthesis; `witness C @T` reflects
  resolvable dictionaries using the same concrete/conditional lookup as implicit
  method dispatch. Runtime type reflection includes `fields`, `variants`, and
  `schema` views.
- THIR and TLC carry internal universe levels for surface `Type`. Explicit
  `$ℓ` / `<$l>` surface syntax has landed as a front-end-only layer; level-
  polymorphic type constructors default to the lowest consistent universe and
  erase before runtime/backend lowering.
- Dataflow Core, ANF, SSA, and LLVM IR text emission exist and are test-covered.
  Record/tuple access is slot-indexed; union construction now uses dense
  per-union tags; ambient `io.print` lowers to a runtime `HostPrint` path;
  granted v2 host operations lower to explicit `HostOp` nodes through
  Dataflow/ANF/SSA/LLVM/runtime; dynamic `load.zti` / `load.zt` dispatches parse
  `.zti` data or evaluates a `.zt` file at runtime and return the source-level
  `Data` envelope; recursive and generic recursive aliases lower to finite
  cyclic `DfTyId` graphs; codegen emits static descriptors for `zutai.show`;
  `@main` renders through the type-directed runtime display path and rejects
  function / `Type` results. **`.zti` data imports and `.zt` pure value/function
  imports compile natively** via one-arena Dataflow Core merge (Phase A): imported
  modules are lowered into the same graph under a `$dep{idx}$` namespace prefix;
  the root references the dep's module-value global (`$dep{idx}$$value`).
  Imported concrete witnesses and structurally matchable conditional witnesses
  compile natively through extern witness tables; higher-kinded or otherwise
  non-matchable witness exports still reject before DC by the witness gate.
- `compile --emit=llvm|obj|bin` selects LLVM text, object, or native binary
  output. Object/binary modes invoke `llc`/`clang`, link `libzutai_rt`, emit
  actionable diagnostics when the host toolchain is absent, and produce
  PIE-capable Linux binaries without `-no-pie`.
- `zutai-eval` has both the THIR oracle and TLC evaluator. Differential coverage
  includes constraints, optionals, `.zti` imports, `.zt` imports, dynamic
  `.zti`/`.zt` loads, imported functions, transitive imports, imported witness
  dictionaries, record update, config overlay, effects, reflection/type-value
  boundaries, polymorphic curried helpers, ambient/imported list verbs, list
  nil/cons pattern evaluation, repeated nested destructures, and name-sorted
  record display.
- `print` remains a prelude compatibility binding, but its type is now
  `Text -> Text ! { io.print : Text -> Text }`. TLC lowers the builtin value to
  a runtime-dispatching function; source handlers can intercept `io.print`, and
  the host `run`, `compile`, and `dataflow` paths dispatch ambient `io.print` at
  runtime instead of replaying compile-time captured output.
- `compile` and `dataflow` no longer fold effectful entry programs through the
  evaluator before Dataflow Core. Supported handled effects, including raw-cell
  effectful generators for custom operations and ambient `io.print`, lower
  through the backend `Computation`/host-driver paths. Resource-backed generator
  cells carrying standard host operations such as `fs.read` now lower to the host
  boundary under the same grant policy as top-level host operations, matching the
  interpreter instead of extending a source handler across a lazy field.
  Unhandled or ungranted residual host effects and unsupported effect rows stay
  gated by `residual_effect_reason` / `zutai_dataflow::try_lower_tlc`.
- `compile` and `dataflow` still fold renderable compile-time reflection
  programs through the THIR type-value evaluator before Dataflow Core.
  Reflection combined with effectful code remains rejected so AOT reflection does
  not consume host effects at compile time.
- Supported full config-overlay calls lower before Dataflow Core: patch-first
  `overlay`/`overlayDeep` applications with record-literal patch values become
  ordinary record updates, and required nested records merge recursively.
- Unsupported residual overlay forms, optional nested-record deep overlays,
  reflection combined with effectful code, unsupported host operations/effect
  rows, function entries, and `Type` entries still reject before DC. Dynamic
  `load.zt` also rejects non-first-order final values at the host boundary.

## Validation notes

- Optional value syntax remains `T? = Optional T` with `#none` / `#some (v)`.
  Optional field access preserves physical presence as `Maybe T` with `#absent`
  / `#present (v)`, so `field? : T?` yields `Maybe (Optional T)`. `?.` works on
  both `Optional` and `Maybe`; `??` unwraps exactly one layer.
- Stable spec docs use parser-accepted typed bindings (`name :: Type = value`) and
  semicolon-terminated record/tagged patterns. Fixtures pin stale syntax
  rejections.
- `Int??` lexes as `Int` + `??`, not a double optional. Write `(Int?)?` for a
  nested optional.
- CLI native binary coverage includes primitive, record, tuple, union, text,
  atom, and posit entry values; the Linux PIE matrix is verified with
  `llc -relocation-model=pic` and `clang -pie`.
