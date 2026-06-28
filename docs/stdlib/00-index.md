# Zutai Standard Library

This directory documents standard-library APIs planned outside the core language syntax.

## Layering

The library is split into two tiers:

```text
prelude   = a small, always-in-scope set of names (auto-imported)
modules   = everything else, brought in with an explicit `import`
```

Name resolution layers as **user > prelude > intrinsics**: a user binding shadows
a prelude name, and a prelude name shadows a Rust intrinsic of the same spelling.

Two principles govern what lives where:

- **Focused prelude.** Only the names every pipeline needs are auto-imported. Domain
  helpers (`list`, `text`, `num`, …) are explicit imports, keeping the auto-scope small
  and collision-free. See [Prelude](prelude.md).
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
prelude (`Stream` `StreamEff` `Step` and the non-conflicting combinators
`empty` `cons` `singleton` `unfold` `take` `drop` `toList` `fromList`
`takeList`), and the ambient **function/list** prelude (`id` `const`
`compose` `flip`; `fold`/`foldl'` `map` `filter` `length` `append` `uncons`
`head?` `tail?` over `List`). Stream `map`/`filter`/`fold`/`uncons` remain
available through `import stdlib.stream`; a user binding of the same spelling
always shadows the ambient fallback.

## Modules

| Module | Contents | Status |
| --- | --- | --- |
| [Config](config.md) | `overlay overlayDeep Patch DeepPatch` | accepted; full record-literal overlays lower to AOT record updates |
| `list` | `foldl' fold map filter length append uncons head? tail?` | accepted as ambient source prelude and importable through `stdlib.prelude`; `fold`/`foldl'` share a strict left-fold bridge intrinsic (`listFoldlStrict`) and native runtime call; list nil/cons patterns lower through THIR/TLC/Dataflow/ANF/SSA/LLVM |
| `stream` | `Stream` `StreamEff` `Step`, `stream { yield ...; }`, `empty singleton cons unfold uncons map filter take drop fold fromList toList takeList` | accepted; `Stream A` is demand-driven **codata** (not `List A`), importable via embedded `stdlib.stream`; non-conflicting names stay ambient, while stream `map`/`filter`/`fold`/`uncons` are qualified/imported to leave ambient names for `List`; pure finite and infinite generators run on interpreter and native backend; effectful generators reference-interpreter + native `io.print` parity (see [stream](stream.md)) |
| `optional` | `map andThen filter withDefault isSome toList` | planned (source) |
| `result` | `Result Validation ok err valid invalid map map2 mapErr andThen withDefault errors` | planned (source); see [Prelude](prelude.md) error model |
| `num` | `min max abs clamp pow rem gcd toFloat round truncate` | planned (source + conversion intrinsics) |
| `text` | `length split join trim toUpper toLower contains replace show parseInt parseFloat` | planned (intrinsic — no char ops in source) |
| `cmp` | `Ordering (#lt/#eq/#gt)`, comparator builders | planned; generic `compare` is witness-dispatched once a `cmp` module lands (v1 constraints already support `Ord`) |
| `reflect` | `fields variants schema witness` | accepted; THIR/TLC/evaluator support; `compile`/`dataflow` fold serializable reflection to backend constants and reject residual reflection (a raw `witness` dictionary or `Type`-valued result) before lowering — the fold-or-reject model |

`sortBy`/`groupBy`/`cmp` take explicit comparators: the standard `Ord`/`Eq`
constraints (v1) support witness-dispatched comparison, but a dedicated `cmp`
module with comparator builders is still planned.

`Stream` is the standard-library home for iterator-like pure APIs. Its
combinators are **ambient prelude** (no import needed) and also importable via
embedded `stdlib.stream`; the language-level producer `stream { yield ...; }` is
syntax, not a standard-library function. Host-backed streams such as file lines,
environment scans, clock events, and randomness require explicit capabilities
(v2 host capabilities); they must not become ambient APIs.

Standard-library names are ordinary bindings, not language keywords, unless a page explicitly says otherwise.
