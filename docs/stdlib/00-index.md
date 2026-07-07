# Zutai Standard Library

This directory documents current and planned standard-library APIs outside the
core language syntax.

## Layering

The library is split into two tiers:

```text
prelude   = always-in-scope source helpers + compiler-backed compatibility names
modules   = everything else, brought in with an explicit `import`
```

Name resolution layers as **user > prelude > intrinsics**: a user binding shadows
a prelude name, and a prelude name shadows a Rust intrinsic of the same spelling.

Two principles govern what lives where:

- **Focused source prelude.** Ordinary `.zt` prelude names stay limited to
  broadly useful pipeline helpers. Compiler-backed compatibility names
  (`print`, reflection, config overlay, dynamic load, and low-level bridges) are
  auto-seeded by the intrinsic layer for backwards compatibility, while the
  importable source modules expose the same supported surfaces explicitly.
  Domain helpers (`text`, `num`, `data`, `validate`, …) are explicit imports,
  keeping the source auto-scope small and collision-free. See [Prelude](prelude.md).
- **Source-canonical, intrinsic-optimized.** Wherever the language can express a function,
  its `.zt` definition is the specification; any compiler-internal implementation is a
  verified optimization of that exact binding, not a parallel semantics. Functions the
  language *cannot* yet express (list spine access, strict `fold`, text char-ops,
  reflection, effect host ops) are true intrinsics.

## Implementation home

Embedded source modules live under `crates/general/stdlib/src/modules/` and are
registered by the dependency-free `zutai-stdlib` crate. HIR consumes that
registry for ambient source-prelude injection, while `zutai-semantic` consumes
the same registry for `import stdlib.<name>` resolution. `zutai-hir` still
re-exports the old `*_MODULE_SRC` constants for Rust-side compatibility, but
`zutai-stdlib` is the source of truth.

## Adding standard-library functionality

1. Prefer a new or existing explicit `stdlib.<module>` source file; do not add
   ambient names unless the prelude policy is deliberately changed.
2. Keep public semantics in `.zt` whenever the language can express them. If an
   operation needs compiler help, expose it through a source wrapper over a
   private bridge such as `__moduleOp`.
3. Classify support precisely in docs and tests: pure source, bridge-backed
   native, compile-time fold/reject, or host/effectful.
4. Update the `zutai-stdlib` registry, the module doc page, import/eval/native
   coverage, and the implementation status notes together.

## Prelude

See [Prelude](prelude.md) for the auto-imported set, the `prelude.zt` resolution
mechanism, the list-destructuring decision, and the error-handling boundary.

Auto-imported names: the intrinsic prelude (`print`; reflection `fields`
`variants` `schema`; config `overlay` `overlayDeep`; list-bridge `listEmpty`
`listCons` `listIsNil` `listHead` `listTail`; strict list fold
`listFoldlStrict`; dynamic load `loadZti` `loadZt`), the ambient **stream**
prelude (`Data` `DataField` `Stream` `StreamEff` `Step` and the
non-conflicting combinators `empty` `cons` `singleton` `unfold` `take` `drop`
`toList` `fromList` `takeList`), and the ambient **function/list** prelude (`id` `const`
`compose` `flip` `not`; `fold`/`foldl'` `map` `filter` `length` `append` `uncons`
`head?` `tail?` over `List`). Stream `map`/`filter`/`fold`/`uncons` remain
available through `import stdlib.stream`; a user binding of the same spelling
always shadows the ambient fallback.

## Modules

| Module | Contents | Status |
| --- | --- | --- |
| [Config](config.md) | `Patch DeepPatch overlay overlayDeep` | accepted as explicit embedded `stdlib.config`; supported full record-literal calls through module-qualified or destructured aliases lower exactly like the ambient builtin overlay forms; residual/partial overlay remains backend-gated |
| [List](list.md) | `empty singleton cons fold foldl' foldr map filter length append uncons head? tail? reverse flatten zip zipWith enumerate take drop takeWhile dropWhile span any all find partition range sum product sortBy groupBy dedupBy` | accepted as explicit embedded `stdlib.list`; current ambient list basics stay in `stdlib.prelude`, while the larger toolbox remains opt-in; pure source over list patterns/bridges with interpreter and native parity |
| `stream` | `Data` `DataField` `Stream` `StreamEff` `Step`, `stream { yield ...; }`, `empty singleton cons unfold uncons map filter take drop fold fromList toList takeList` | accepted; `Stream A` is demand-driven **codata** (not `List A`), importable via embedded `stdlib.stream`; non-conflicting names stay ambient, while stream `map`/`filter`/`fold`/`uncons` are qualified/imported to leave ambient names for `List`; pure finite and infinite generators run on interpreter and native backend; raw-cell effectful generators have native parity for supported custom effects, ambient `io.print`, and standard host operations granted at the host boundary (see [stream](stream.md)) |
| `optional` | `map andThen filter withDefault isSome toList` | accepted as explicit importable source module via `import stdlib.optional`; exports `map` `andThen` `filter` `withDefault` `isSome` `toList` over `Optional`; interpreter/TLC/native parity covered |
| `result` | `Result Validation ok err valid invalid map map2 mapErr andThen withDefault errors` | accepted as explicit importable source module via `import stdlib.result`; exports `Result` `Validation` `ok` `err` `valid` `invalid` `map` `map2` `mapErr` `andThen` `withDefault` `errors`; `Result` short-circuits and `Validation` accumulates `List E` errors; interpreter/TLC/native parity covered |
| `num` | `min max abs clamp pow rem gcd toFloat round truncate` | accepted as explicit importable source module via `import stdlib.num`; exports `min` `max` `abs` `clamp` `pow` `rem` `gcd` `toFloat` `round` `truncate`; Int helpers are source wrappers over checked scalar bridge intrinsics where needed; `toFloat`/`round`/`truncate` are conversion intrinsics; interpreter/TLC/native parity covered |
| `text` | `length split join trim toUpper toLower contains replace show parseInt parseFloat` | accepted as explicit importable source module via `import stdlib.text`; backed by scalar bridge intrinsics/runtime helpers for Unicode scalar length, splitting/joining, trimming, case conversion, containment/replacement, text quoting, and numeric parsing; interpreter/TLC/native parity covered |
| [FS](fs.md) | `ReadLine/WriteText/ScopedRead/ScopedWrite/ScopedReadWrite/WholeRead/WholeWrite/WholeFile` aliases plus `openRead readLine closeRead openWrite writeText flush closeWrite withReader withWriter readAll writeAll` | accepted as explicit embedded `stdlib.fs`; source wrappers over explicit `fs.*` host effects, with opaque `Reader`/`Writer` handles, effect aliases for common rows, and bracket helpers for scoped close; whole-file `readAll`/`writeAll` remain compatibility wrappers over `fs.read`/`fs.write`; no filesystem API is ambient |
| [Net](net.md) | `Listen/Accept/Read/Write/Close/Connection/Server` aliases plus `listen accept read write close` | accepted as explicit embedded `stdlib.net`; source wrappers over existing `net.*` TCP host effects and effect aliases for common server/connection rows; listener and connection handles remain `Int`, `write` preserves the current-connection runtime behavior; no network API is ambient |
| `cmp` | `Ordering (#lt/#eq/#gt) lt eq gt isLt isEq isGt reverse then by compareInt compareFloat compareText` | accepted as explicit importable source module via `import stdlib.cmp`; comparator composition and concrete Int/Float/Text comparators are source definitions; the exported `then` field is backed by an internal `thenCmp` binding because `then` is a keyword |
| [Data](data.md) | `Data DataField DecodeError Result bool int float text atom list record tagged fieldOf kind asBool asInt asFloat asText asAtom asList asRecord asTagged field field? at tag payload mapList` | accepted as explicit embedded `stdlib.data`; first-order data constructors and structured decoder errors; decoder results use the `stdlib.result.Result` shape through the module's exported `Result` alias |
| [Validate](validate.md) | `ValidationError Validation Result valid invalid errors map map2 map3 required satisfy nonEmptyText intRange oneOfText oneOfInt toResult fromResult` | accepted as explicit embedded `stdlib.validate`; accumulating validation helpers over structured errors; validation/result aliases forward to `stdlib.result` shapes |
| [Reflect](reflect.md) | `SchemaKind SchemaField SchemaVariant Schema fields variants schema` | accepted as explicit embedded `stdlib.reflect`; `fields`/`schema` keep THIR-oracle routing, `fields`/`variants`/`schema` keep compile/dataflow fold-or-reject behavior; `witness C @T` remains syntax and is not exported |

`sortBy`/`groupBy` take explicit comparator functions. Generic witness-dispatched
`compare` remains deferred; concrete `stdlib.cmp` comparators are available now.

`Stream` is the standard-library home for iterator-like pure APIs. Its
combinators are **ambient prelude** (no import needed) and also importable via
embedded `stdlib.stream`; the language-level producer `stream { yield ...; }` is
syntax, not a standard-library function. Host-backed streams such as file lines,
environment scans, clock events, randomness, and network sockets require
explicit capabilities (v2 host capabilities); they must not become ambient APIs.
The low-level text filesystem helpers live in explicit `stdlib.fs`, and the
current TCP helpers live in explicit `stdlib.net`.

Standard-library names are ordinary bindings, not language keywords, unless a page explicitly says otherwise.
