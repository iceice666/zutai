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
  auto-seeded by the intrinsic layer until source stdlib modules replace the
  surfaces that can be expressed safely. Domain helpers (`text`, `num`, …) are
  explicit imports, keeping the source auto-scope small and collision-free. See
  [Prelude](prelude.md).
- **Source-canonical, intrinsic-optimized.** Wherever the language can express a function,
  its `.zt` definition is the specification; any compiler-internal implementation is a
  verified optimization of that exact binding, not a parallel semantics. Functions the
  language *cannot* yet express (list spine access, strict `fold`, text char-ops,
  reflection, effect host ops) are true intrinsics.

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
`compose` `flip`; `fold`/`foldl'` `map` `filter` `length` `append` `uncons`
`head?` `tail?` over `List`). Stream `map`/`filter`/`fold`/`uncons` remain
available through `import stdlib.stream`; a user binding of the same spelling
always shadows the ambient fallback.

## Modules

| Module | Contents | Status |
| --- | --- | --- |
| [Config](config.md) | `overlay overlayDeep Patch DeepPatch` | accepted intrinsic compatibility surface; full record-literal overlays lower to AOT record updates, but no embedded `stdlib.config` source module ships yet |
| `list` | `foldl' fold map filter length append uncons head? tail?` | accepted as ambient source prelude and importable through `stdlib.prelude`; `fold`/`foldl'` share a strict left-fold bridge intrinsic (`listFoldlStrict`) and native runtime call; list nil/cons patterns lower through THIR/TLC/Dataflow/ANF/SSA/LLVM |
| `stream` | `Data` `DataField` `Stream` `StreamEff` `Step`, `stream { yield ...; }`, `empty singleton cons unfold uncons map filter take drop fold fromList toList takeList` | accepted; `Stream A` is demand-driven **codata** (not `List A`), importable via embedded `stdlib.stream`; non-conflicting names stay ambient, while stream `map`/`filter`/`fold`/`uncons` are qualified/imported to leave ambient names for `List`; pure finite and infinite generators run on interpreter and native backend; effectful generators reference-interpreter + native `io.print` parity (see [stream](stream.md)) |
| `optional` | `map andThen filter withDefault isSome toList` | accepted as explicit importable source module via `import stdlib.optional`; exports `map` `andThen` `filter` `withDefault` `isSome` `toList` over `Optional`; interpreter/TLC/native parity covered |
| `result` | `Result Validation ok err valid invalid map map2 mapErr andThen withDefault errors` | accepted as explicit importable source module via `import stdlib.result`; exports `Result` `Validation` `ok` `err` `valid` `invalid` `map` `map2` `mapErr` `andThen` `withDefault` `errors`; `Result` short-circuits and `Validation` accumulates `List E` errors; interpreter/TLC/native parity covered |
| `num` | `min max abs clamp pow rem gcd toFloat round truncate` | accepted as explicit importable source module via `import stdlib.num`; exports `min` `max` `abs` `clamp` `pow` `rem` `gcd` `toFloat` `round` `truncate`; Int helpers are source wrappers over checked scalar bridge intrinsics where needed; `toFloat`/`round`/`truncate` are conversion intrinsics; interpreter/TLC/native parity covered |
| `text` | `length split join trim toUpper toLower contains replace show parseInt parseFloat` | accepted as explicit importable source module via `import stdlib.text`; backed by scalar bridge intrinsics/runtime helpers for Unicode scalar length, splitting/joining, trimming, case conversion, containment/replacement, text quoting, and numeric parsing; interpreter/TLC/native parity covered |
| `cmp` | `Ordering (#lt/#eq/#gt) lt eq gt isLt isEq isGt reverse then by compareInt compareFloat compareText` | accepted as explicit importable source module via `import stdlib.cmp`; comparator composition and concrete Int/Float/Text comparators are source definitions; the exported `then` field is backed by an internal `thenCmp` binding because `then` is a keyword |
| `reflect` | `fields variants schema witness` | accepted intrinsic compatibility surface; THIR/TLC/evaluator support; `compile`/`dataflow` fold serializable reflection to backend constants and reject residual reflection (a raw `witness` dictionary or `Type`-valued result) before lowering, but no embedded `stdlib.reflect` source module ships yet |

`sortBy`/`groupBy` take explicit comparator functions. Generic witness-dispatched
`compare` remains deferred; concrete `stdlib.cmp` comparators are available now.

`Stream` is the standard-library home for iterator-like pure APIs. Its
combinators are **ambient prelude** (no import needed) and also importable via
embedded `stdlib.stream`; the language-level producer `stream { yield ...; }` is
syntax, not a standard-library function. Host-backed streams such as file lines,
environment scans, clock events, and randomness require explicit capabilities
(v2 host capabilities); they must not become ambient APIs.

Standard-library names are ordinary bindings, not language keywords, unless a page explicitly says otherwise.
