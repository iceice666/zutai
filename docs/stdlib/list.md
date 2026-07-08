# Standard Library: List

## Status

Accepted and implemented as an explicit embedded source module:
`l ::= import stdlib.list`. The smaller list basics remain ambient through
`stdlib.prelude`; the larger toolbox is opt-in and does not become ambient.

The module source lives at `crates/general/stdlib/src/modules/list.zt` and is
registered by `zutai-stdlib` as `LIST_MODULE_SRC`.

## API

```zt
empty singleton cons
fold foldl' foldr
map filter length append uncons head? tail?
reverse flatten zip zipWith enumerate
take drop takeWhile dropWhile span
any all find findMap countBy sumBy filterMap partition
range sum product
maximum minimum maximumBy minimumBy
sortBy groupBy dedupBy
```

`empty` is written as `Unit -> List A`, so use `l.empty ()` when an empty
polymorphic list value needs an explicit producer.

## Semantics

- `range start stop` returns the ascending half-open list `[start, stop)`. If
  `start >= stop`, it returns the empty list.
- `sortBy compare xs` is stable ascending. Comparators return
  `stdlib.cmp.Ordering`; concrete helpers such as `c.compareInt` live in
  `stdlib.cmp`.
- `groupBy same xs` groups consecutive runs only; it does not sort first.
- `dedupBy same xs` removes later values from consecutive runs only.
- `zip` and `zipWith` stop when either input list ends.
- `findMap f xs` returns the first `#some` produced by `f`, or `#none`.
- `countBy p xs` counts elements accepted by `p`.
- `sumBy f xs` maps each element to an `Int` and sums the results.
- `filterMap f xs` drops `#none` results and unwraps `#some` values.
- `maximum`, `minimum`, `maximumBy`, and `minimumBy` return `#none` for empty
  lists. The `By` forms return the selected `Int`, not the source element.

## Implementation Notes

The toolbox is pure `.zt` over list patterns and existing list bridge
intrinsics (`listEmpty`, `listCons`, and `listFoldlStrict`). No runtime ABI,
Dataflow, SSA, or codegen primitive is added for this module.
