# Standard Library: Stream

Status: core API shipped as ambient prelude functions (V3-G2, 2026-06-25) **and**
as an importable module (V3-G6, 2026-06-25). `Stream A` is demand-driven
**codata** — `Unit -> { #nil; #cons : { head : A; tail : Stream A; }; }` (V3-G1) —
not `List A`. The combinators `empty`, `cons`, `singleton`, `unfold`, `map`,
`filter`, `take` (as `Stream -> Stream`), `drop`, `fold`, and `uncons` are
available without import (the prelude is a fallback: a user or constraint-method
name of the same spelling wins). `unfold` takes a step function returning a
`Step S A` union (`#done`/`#yield { item; next }`) rather than the builtin
`Optional` — `Optional`'s `#some` payload is a positional tuple that does not
compose with a record payload at the surface. `empty :: <A> Stream A` is a
polymorphic nullary value; it now instantiates correctly per use (a `<A>`
reference outside callee position freshens its type variable — see
`docs/ARCHIVED.md` "BindingRef instantiation site"). The `List`-interop subset —
`toList`, `fromList`, and `takeList` (`= toList ∘ take`) — **shipped 2026-06-26**
(V3-G2 residual). `take` stays `Stream -> Stream`; `takeList` is the named
`take -> List` form. The builtin `List` has no source-level head/tail ops, so the
three combinators ride internal scalar bridge primitives the compiler provides
over the builtin `List` (`listEmpty`/`listCons`/`listIsNil`/`listHead`/`listTail`);
the `if`/`match` branching lives in the `.zt` source. See `docs/ARCHIVED.md`
"V3-G2".

## Two surfaces, one source

The combinators live in one canonical file,
`crates/general/hir/src/lower/prelude/stream.zt` (exposed to Rust as
`zutai_hir::STREAM_MODULE_SRC`), which feeds both surfaces (V3-G6):

- **Ambient** (no import). The HIR lowerer `include_str!`s the file and injects its
  declarations as a fallback, so `map`/`filter`/`fold`/… resolve directly. This is
  the original V3-G2 behavior, unchanged.
- **Importable** (explicit). `s ::= import "stream.zt";` binds the module's exported
  record, so the combinators are used qualified — `s.map`, `s.fold`, … The file's
  final expression is that record. Resolution is **path-relative** (the file must
  sit in the importing file's directory subtree); there is no stdlib-root install
  path yet (`docs/TBD.md` "V3-G6 follow-ups"). The export carries the eight
  combinator functions; the `Stream` type is not a named field (it crosses
  structurally inside the combinator signatures), so there is no `s.Stream` yet.

```zt
s ::= import "stream.zt";
double :: Int -> Int = x => x * 2;
add :: Int -> Int -> Int = a b => a + b;
s.fold add 0 (s.map double (s.cons 1 (s.cons 2 (s.singleton 3))))   -- 12
```

`Stream A` is a pure lazy sequence for iterator-style pipelines when producing
or consuming every element as a `List A` would be unnecessary. Phase 29's
`stream { yield expr; ... }` syntax produces finite stream-backed values through
the current list representation.

## Initial API surface

```zt
Stream A
Step S A  -- = { #done; #yield : { item : A; next : S; }; } (unfold step result)
empty     :: <A> Stream A
singleton :: <A> A -> Stream A
cons      :: <A> A -> Stream A -> Stream A
unfold    :: <S, A> (S -> Step S A) -> S -> Stream A
uncons    :: <A> Stream A -> { #none; #some : { head : A; tail : Stream A; }; }
map       :: <A, B> (A -> B) -> Stream A -> Stream B
filter    :: <A> (A -> Bool) -> Stream A -> Stream A
take      :: <A> Int -> Stream A -> Stream A
drop      :: <A> Int -> Stream A -> Stream A
fold      :: <A, B> (B -> A -> B) -> B -> Stream A -> B
fromList  :: <A> List A -> Stream A
toList    :: <A> Stream A -> List A
takeList  :: <A> Int -> Stream A -> List A   -- = toList (take k s)
```

## Edge behavior

- `take n xs` returns `{;}` for `n <= 0` and otherwise demands at most `n`
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
