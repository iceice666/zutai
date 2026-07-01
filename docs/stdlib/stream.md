# Standard Library: Stream

Status: core API shipped as source prelude functions (V3-G2, 2026-06-25) **and**
as an importable module (V3-G6, 2026-06-25). `Stream A` is demand-driven
**codata** — `Unit -> { #nil; #cons : { head : A; tail : Stream A; }; }` (V3-G1) —
not `List A`. The non-conflicting combinators `empty`, `cons`, `singleton`,
`unfold`, `take` (as `Stream -> Stream`), `drop`, `toList`, `fromList`, and
`takeList` are still available without import. Stream `map`/`filter`/`fold`/
`uncons` are exported by `import stdlib.stream` and should be used qualified
because the unqualified names now denote the ambient `List` verbs. `unfold`
takes a step function returning a
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
`crates/general/stdlib/src/modules/stream.zt` (registered by `zutai-stdlib` and
compatibly re-exported as `zutai_hir::STREAM_MODULE_SRC`), which feeds both
surfaces (V3-G6):

- **Ambient** (no import). The HIR lowerer reads the embedded source from
  `zutai-stdlib` and injects non-conflicting declarations as a fallback. Stream
  `empty`/`cons`/`singleton`/
  `unfold`/`take`/`drop`/`toList`/`fromList`/`takeList` resolve directly; a user
  or constraint-method binding of the same spelling still wins.
- **Importable** (explicit). `import stdlib.stream` resolves to **embedded
  in-binary source** (no install path, no subtree-confinement exception) and
  binds the module's exported record, so every exported value is available
  qualified — `s.map`, `s.fold`, `s.uncons`, … A path-relative
  `import "stream.zt"` still works when a local file of that name sits in the
  importing file's directory subtree. The export record carries the combinator
  functions **and** the type values `Data`, `DataField`, `Stream`, `Step`, and
  `StreamEff` as named, selectable/destructurable fields, so `s.Stream`,
  `s.Step`, and `s.StreamEff` are available, and a parametric imported
  constructor can be *applied* in an annotation (`x :: s.Stream Int`).
  Selective/open import reuses the destructuring binding form:
  `{ map; fold; } ::= import stdlib.stream;` brings those stream members in
  unqualified when wanted.

```zt
s ::= import stdlib.stream;
double :: Int -> Int = x => x * 2;
add :: Int -> Int -> Int = a b => a + b;
s.fold add 0 (s.map double (s.cons 1 (s.cons 2 (s.singleton 3))))   -- 12
```

`Stream A` is demand-driven **codata** — a step function `Unit -> StreamCell A`
over a `#nil`/`#cons` cell — not a memoizing lazy list: consuming a stream twice
steps twice, and infinite streams are representable (an `unfold` with a
non-terminating seed, bounded by `take`/`uncons`). The `stream { yield expr;
... }` syntax desugars by continuation-passing onto this codata cell; `yield`
may appear under conditionals and tail recursion (`yield from`), so pure finite
*and* infinite generators type-check and evaluate on both the interpreter and
the native backend.

## Initial API surface

```zt
Data              -- dynamic load envelope for first-order `.zti` / `.zt` data
DataField         -- `{ name : Text; value : Data; }`
Stream A          -- = Unit -> { #nil; #cons : { head : A; tail : Stream A; }; }  (codata)
StreamEff A e     -- effectful stream; forcing a cell may perform ops in row e. StreamEff A {} = Stream A
Step S A          -- = { #done; #yield : { item : A; next : S; }; } (unfold step result)
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
The full combinator set above is implemented in `.zt` over the codata cell and
the list-interop bridge primitives (`listEmpty`/`listCons`/`listIsNil`/
`listHead`/`listTail`); compiler intrinsics are allowed only to preserve sharing,
avoid repeated thunk allocation, or optimize stream stepping without changing the
source binding semantics. Effectful streams use the `StreamEff A e` alias and the
effect machinery, not a separate effectful codata type (see
`docs/v3_spec/01-generators.md`).

## Host boundary policy

Pure constructors such as `unfold` and `fromList` belong here now. Host-backed
producers belong in explicit capability modules after Phase 27 and return or
consume `Stream` only through capability-typed APIs. Filesystem, environment,
clock, randomness, and future network-backed generation remain non-ambient host
boundaries, not available functions in this module. The runnable
`examples/host_stream_read.zt` artifact demonstrates the rule for
`stream { yield perform fs.read ... }`: forcing one lazy cell reads through the
interpreter/native host boundary, a source handler around the stream does not
capture that cell-level host operation, and an unforced missing tail is never
read.
