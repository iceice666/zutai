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

Auto-imported names: types `Type Text Bool Int Float List Optional Maybe`;
list verbs `map filter fold`; `id`; `print`.

## Modules

| Module | Contents | Status |
| --- | --- | --- |
| [Config](config.md) | `overlay overlayDeep Patch DeepPatch` | accepted; full record-literal overlays lower to AOT record updates |
| `fn` | `const compose flip` | planned (source) |
| `list` | `foldr foldl' length reverse append zip flatten take drop find any all sum product partition sortBy groupBy indexOf uncons head? tail?` | planned (source + spine intrinsics) |
| `optional` | `map andThen filter withDefault isSome toList` | planned (source) |
| `result` | `Result Validation ok err valid invalid map map2 mapErr andThen withDefault errors` | planned (source); see [Prelude](prelude.md) error model |
| `num` | `min max abs clamp pow rem gcd toFloat round truncate` | planned (source + conversion intrinsics) |
| `text` | `length split join trim toUpper toLower contains replace show parseInt parseFloat` | planned (intrinsic — no char ops in source) |
| `cmp` | `Ordering (#lt/#eq/#gt)`, comparator builders | planned; generic `compare` waits on v1 constraints |
| `reflect` | `fields schema` | accepted; THIR-only intrinsic, backend-gated |

`sortBy`/`groupBy`/`cmp` take explicit comparators: v0 has no `Ord`/`Eq` typeclass, so
ad-hoc ordering is passed as a function until v1 constraints land.

Standard-library names are ordinary bindings, not language keywords, unless a page explicitly says otherwise.
