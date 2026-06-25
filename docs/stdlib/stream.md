# Standard Library: Stream

Status: core API shipped as ambient prelude functions (V3-G2, 2026-06-25).
`Stream A` is demand-driven **codata** — `Unit -> { #nil; #cons : { head : A;
tail : Stream A; }; }` (V3-G1) — not `List A`. The combinators `cons`,
`singleton`, `map`, `filter`, `take` (as `Stream -> Stream`), `drop`, `fold`, and
`uncons` are available without import (the prelude is a fallback: a user or
constraint-method name of the same spelling wins). Deferred: `empty`/`unfold`
(type-inference edge cases) and the `List`-interop subset (`take -> List`,
`toList`, `fromList`), which needs source-level list construction; see
`docs/ARCHIVED.md` "V3-G2".

`Stream A` is a pure lazy sequence for iterator-style pipelines when producing
or consuming every element as a `List A` would be unnecessary. Phase 29's
`stream { yield expr; ... }` syntax produces finite stream-backed values through
the current list representation.

## Initial API surface

```zt
Stream A
empty     :: <A> Stream A
singleton :: <A> A -> Stream A
cons      :: <A> A -> Stream A -> Stream A
unfold    :: <S, A> (S -> Optional { item : A; next : S; }) -> S -> Stream A
uncons    :: <A> Stream A -> Optional { head : A; tail : Stream A; }
map       :: <A, B> (A -> B) -> Stream A -> Stream B
filter    :: <A> (A -> Bool) -> Stream A -> Stream A
take      :: <A> Int -> Stream A -> List A
drop      :: <A> Int -> Stream A -> Stream A
fold      :: <A, B> (B -> A -> B) -> B -> Stream A -> B
fromList  :: <A> List A -> Stream A
toList    :: <A> Stream A -> List A
```

## Edge behavior

- `take n xs` returns `[]` for `n <= 0` and otherwise demands at most `n`
  stream cells.
- `drop n xs` returns `xs` for `n <= 0` and otherwise demands at most `n`
  stream cells before returning the remaining stream.
- `toList` is only for finite streams; on an infinite stream it does not
  terminate.
- `fold` may not terminate on infinite streams unless the stream ends; use
  `take` before `fold` for finite prefixes.
- `filter` over an infinite stream may continue demanding input until it finds
  the next matching element.

## Source and intrinsic policy

Define stream functions in `.zt` when recursive types and list/optional
primitives can express them. Today only the `Stream` type constructor and finite
`stream { yield ...; }` producer shell are implemented; compiler intrinsics are
allowed only to preserve sharing, avoid repeated thunk allocation, or optimize
stream stepping without changing the source binding semantics.

## Host boundary policy

Pure constructors such as `unfold` and `fromList` belong here now. Host-backed
producers belong in explicit capability modules after Phase 27 and return or
consume `Stream` only through capability-typed APIs. Filesystem, environment,
clock, randomness, and future network-backed generation remain non-ambient host
boundaries, not available functions in this module.
