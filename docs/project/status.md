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

The in-progress 2026-07-13 typed-staging slice adds compile-time-only `Code A`,
hygienic direct/bound `quote(expr)` / `splice(expr)`, generic quoted-record
derive recipes, and a provisional ambient `FromData`/`decode` structural
decoder. Supported decoders accumulate path-aware record/list errors and lower
to ordinary TLC terms; missing physical optional fields decode as absent, and
no `Code` node or decoder runtime primitive reaches Dataflow Core. Full recipe
evaluation, typed structural reflection descriptors, and richer expansion
diagnostics remain open in the roadmap. Reference/TLC evaluation supports nested
records and unions; LLVM/native execution is currently verified only for
primitive and flat-record decoders, with nested-record parity still open.

_Last updated: 2026-07-13 (browser DOM event expansion: `stdlib.html`
`EventHandler` gains `#change`/`#submit`/`#blur`/`#focus`/`#keyDown`/`#keyUp`
variants alongside the existing `#click`/`#input`, with matching
`onChange`/`onSubmit`/`onBlur`/`onFocus`/`onKeyDown`/`onKeyUp` (and `*With`
options-taking) constructors; the browser kernel decodes the new tags,
listens for the corresponding native DOM events (`change`/`submit`/`blur`/
`focus`/`keydown`/`keyup`), and extracts `Text` payloads from `<select>`
elements and `KeyboardEvent.key` for the key handlers; see
`crates/browser/kernel/tests/events.rs`);
prior baseline updates: 2026-07-13 (stdlib package independence: the
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
HIR-derived navigation, rename, symbols, completion, and parser quick fixes; see "Language Server
Protocol editor baseline" below.
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
