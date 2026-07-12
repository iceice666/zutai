# Standard Library: Prelude

## Status

Accepted and implemented through the current source-prelude / stdlib-H baseline:
ambient `stream.zt` and `prelude.zt` are live, and the explicit
`stdlib.optional`, `stdlib.result`, `stdlib.num`, `stdlib.text`, and
`stdlib.cmp` modules are shipped. The explicit `stdlib.config`,
`stdlib.reflect`, `stdlib.list`, `stdlib.data`, and `stdlib.validate` modules
are also filesystem modules and importable; none of those larger surfaces becomes ambient.

The prelude is the set of names in scope in every `.zt` module without an
explicit `import`. Its source layer is deliberately small; its intrinsic layer
also carries compiler-backed compatibility names until those surfaces can move
behind explicit source modules. Everything outside those two layers is an
explicit [module](00-index.md).

## Two layers behind one scope

What looks like "the prelude" is two cooperating layers:

```text
intrinsic prelude  = Rust-backed names the language cannot express in source
source prelude     = ordinary `.zt` definitions, auto-imported (`prelude.zt`)
```

Resolution layers as **user > prelude > intrinsics**. A user binding shadows a prelude
name; a prelude binding shadows an intrinsic of the same spelling. The source prelude is
written on top of the intrinsics and the core syntax — it imports nothing.

Today two layers are live. The **intrinsic** layer is `BUILTIN_VALUE_NAMES`
seeded in `crates/general/hir/src/lower/mod.rs`: `print`; reflection `fields`,
`variants`, `schema`; config `overlay`, `overlayDeep`; the list-interop bridge
`listEmpty`/`listCons`/`listIsNil`/`listHead`/`listTail`; the strict list-fold
bridge `listFoldlStrict`; dynamic load `loadZti`/`loadZt`; and the internal
numeric scalar bridge `__numAbs`/`__numRem`/`__numPow`/`__numToFloat`/
`__numRound`/`__numTruncate` used by explicit `stdlib.num` — plus the builtin
type constructors (`List`, `Optional`, `Maybe`, `Patch`, `DeepPatch`). The
**source** layer is two ambient prelude files declared by the filesystem stdlib
manifest under `crates/general/stdlib/src/modules/`; semantic analysis passes
their loaded source to the HIR lowerer for fallback injection:

- the *stream* prelude `stream.zt`, so `Data`,
  `DataField`, `Stream`, `StreamEff`, `Step`, and the non-conflicting
  combinators `empty`/`cons`/`singleton`/`unfold`/`take`/`drop`/`toList`/
  `fromList`/`takeList` are in scope without an import;
- the *function/list* prelude `prelude.zt`, so
  `id`/`const`/`compose`/`flip`/`not` and the `List` verbs `fold`/`foldl'`/`map`/
  `filter`/`length`/`append`/`uncons`/`head?`/`tail?` are in scope without an
  import (stdlib slices B/C).

Both are importable via `import stdlib.stream` / `import stdlib.prelude`.
Stream `map`/`filter`/`fold`/`uncons` are still exported by `stdlib.stream`, but
are not ambient because the unqualified names now denote the `List` verbs. Use a
qualified stream import (`s.map`, `s.fold`, `s.uncons`) when both surfaces appear
in one program.

## Contents

The source prelude is deliberately focused; the full ambient scope also includes
compiler-backed compatibility names from the intrinsic layer:

```text
Types        Type Text Bool Int Float List Optional Maybe
             Patch DeepPatch                                (intrinsic)
Stream       Data DataField Stream StreamEff Step; empty cons singleton unfold
             take drop toList fromList takeList             (ambient source prelude)
Host/effects print; loadZti loadZt                           (intrinsic)
Reflect      fields variants schema                          (intrinsic)
Config       overlay overlayDeep                             (intrinsic)
Function     id const compose flip not                       (ambient source prelude)
List verbs   fold foldl' map filter length append uncons head? tail?
                                                             (ambient source prelude)
```

Rationale:

- The **types** are needed to write any signature; they are already intrinsic type
  constructors.
- The **list verbs** are the advertised idiom (`docs/spec/05-type-system/lists.md`):
  `items |> filter pred |> map f |> fold step init`. `fold` is the strict left fold.
- `id` is the no-op combinator pipelines and higher-order functions reach for.
- `print` is the one ambient host effect (`io.print`); it stays a
  pipeline-friendly tap. Reflection/config/dynamic-load names are compatibility
  surfaces with their own fold-or-reject or host-boundary gates, not source
  prelude helpers. `Data` and `DataField` live in the stream prelude because
  dynamic `loadZti`/`loadZt` returns that envelope.

Excluded from the prelude on purpose:

- `foldr`, `zip`, `flatten`, `reverse`, `take`, `drop`, search/sort/grouping
  helpers, and numeric aggregates remain explicit `stdlib.list` work.
- `text`, `num`, `optional`, `result`, `cmp`, `data`, and `validate` are
  explicit imports.
- `reflect` and `config` have explicit modules, but the ambient intrinsic
  compatibility names remain compiler-backed gates rather than source-prelude
  helpers.

Naming note: the list-specialized `map`/`fold` may later become the
witness-dispatched `Functor`/`Foldable` methods through the constraint system
(`docs/spec/06-polymorphism/constraints.md`). Stream methods with the same names remain
available via `import stdlib.stream`.

## Source-canonical, intrinsic-optimized

For any function the language can express, the **`.zt` definition is the specification**;
a compiler-internal implementation is a *verified optimization of that exact binding id*,
never a second semantics. This avoids the drift trap where a hand-written intrinsic and the
source form disagree.

`map`/`filter`/`fold` are source-prelude declarations over `List`. `map` and
`filter` use list-destructuring patterns and recursion; `fold` and `foldl'`
share the strict `listFoldlStrict` bridge so ambient pipelines are strict
left folds rather than lazy accumulator chains. Compiler internals remain
justified for two non-cosmetic reasons:

- **Strict `fold` (`foldl'`).** A lazy core leaks space on a naive left fold.
  Until a `seq`/`force` strictness primitive exists, the strict `fold` cannot be
  written directly in source, so the bridge intrinsic is mandatory.
- **Spine access.** Runtime lists are flat arrays (`Value::List(Rc<[Thunk]>)` in
  `crates/general/eval/src/value.rs`). Pattern lowering handles `{h; ...t}`
  consistently through THIR/TLC/eval and the native pipeline.

Equivalence is enforced by differential tests: the source prelude is evaluated
through TLC and the THIR oracle, and native list pipelines are checked against
the interpreter.

## List-destructuring patterns

`HirPatKind`/`ThirPatKind` carry explicit nil and cons-list patterns:

```zt
match xs {
  | {;}       => empty-case;
  | {h; ...t} => cons-case using h and t;
}
```

Exhaustiveness is **nil + (cons or wildcard)**, never an enumeration of list
lengths. The implementation spans grammar/`ast.rs`, `HirPatKind`/`ThirPatKind`,
exhaustiveness (`crates/general/thir/src/lower/exhaust.rs`, with `ListNil` /
`ListCons` constructors), the evaluators, and DC/ANF/SSA/codegen.

## `prelude.zt` resolution

The source prelude is loaded by the HIR lowerer (`zutai-hir`), not a separate
semantic pass — the same model as the stream prelude:

```text
- Load each prelude from the validated stdlib root (or web bundle) and parse it.
- Inject its declarations into every module's root scope as a FALLBACK, after
  intrinsic seeding and user top-level names: a user/constraint binding of the
  same spelling wins, and a colliding name raises no duplicate-binding error.
- Lower a prelude's bodies only when the program references one of its names
  (all-or-nothing per prelude); a program that touches no prelude name keeps it
  out of THIR/TLC/codegen entirely.
- It imports nothing; it depends only on intrinsics + core syntax.
- A parse/lower failure is an internal compiler diagnostic, not a user error.
```

Because prelude names are real HIR declarations (not `BuiltinFn`s), both
evaluators resolve them uniformly with no special seeding: the THIR oracle
(`crates/general/eval/src/eval/top_env.rs`) and the TLC path
(`crates/general/eval/src/tlc_entry.rs`) seed only the intrinsic `BuiltinFn`s;
the source prelude decls thread through as ordinary top-level decls. The same
source backs `import stdlib.stream` / `import stdlib.prelude` (the final record
is the module export; the ambient path reads only the declarations). Stream
`map`/`filter`/`fold`/`uncons` are intentionally qualified-only to keep the
ambient names bound to `List`.

The ambient source prelude **must not** add aliases or wrappers for backend-gated
reflection/config intrinsics. `stdlib.reflect` and `stdlib.config` now provide
explicit wrappers, and the compiler gates recognize their qualified/destructured
aliases. The unqualified compatibility names `fields`/`variants`/`schema` and
`overlay`/`overlayDeep` keep their existing intrinsic fold-or-reject / AOT-gate
behavior. `witness C @T` remains syntax, not a first-class function.

## Error-handling boundary

The error story splits by **control vs data**:

```text
effect (fail / warn)        = control: propagate, abort, capability boundaries
Result / Validation (data)  = collect, store, pattern-match, serialize
```

`fail`/`warn` (`docs/spec/08-effects/algebraic-effects.md`) is the *default, blessed* error idiom and is
**not** mirrored by a prelude `Result`. But `Result` (and an accumulating `Validation`)
earn an explicit `result` module, because effects do not cover:

| Need | `fail` / `warn` | `Result` / `Validation` |
| --- | --- | --- |
| Short-circuit / abort / capability | blessed | verbose |
| Accumulate **all** errors (config normalize) | `fail` short-circuits | `Validation (List E)` |
| Serialize errors across the `.zti` boundary | control, not data | ordinary union |
| Compile to a backend binary today | handled effects lower natively for supported shapes; raw-cell generator effects cover custom operations and ambient `io.print`, and standard host operations lower through the host boundary when granted | pure, lowers now |

So errors-as-data are an opt-in module, not prelude, and not the default. Native
effect lowering has landed for handled effects, but accumulate-all error
collection remains a distinct idiom that `Validation (List E)` covers cleanly
(`fail` short-circuits by design); `Validation` is expected to stay.

## Build order

1. List-destructuring patterns + strict-`fold` intrinsic + `map`/`filter`/`fold`
   in source. *(landed as slice C: ambient/importable `List` verbs in
   `prelude.zt`)*
2. `prelude.zt` resolution. *(landed for the function prelude:
   `id`/`const`/`compose`/`flip`/`not` and the slice C list verbs as ambient source
   declarations via HIR-lowerer injection, importable as `stdlib.prelude`)*
3. `optional`. *(landed as slice D: explicit import only via
   `stdlib.optional`; not ambient, so unqualified `map`/`filter` stay the List
   prelude names unless the module is explicitly destructured)*
4. `result` / `Validation`. *(landed as slice E: explicit import only via
   `stdlib.result`; not ambient, so errors-as-data stay opt-in and outside the
   prelude)*
5. `num`. *(landed as slice F: explicit import only via `stdlib.num`, backed by
   checked scalar bridge intrinsics for remainder, power, conversions, and
   checked abs)*
6. `text`. *(landed as slice G: explicit import only via `stdlib.text`, backed
   by scalar bridge intrinsics for string operations and numeric parsing)*
7. `cmp`. *(landed as slice H: explicit import only via `stdlib.cmp`; concrete
   `compareInt`/`compareFloat`/`compareText` and comparator combinators are
   source definitions; generic witness-dispatched `compare` remains deferred)*
8. Larger explicit stdlib modules. *(landed 2026-06-30: `stdlib.config` and
   `stdlib.reflect` wrap the existing compiler-gated intrinsics without becoming
   ambient; `stdlib.list`, `stdlib.data`, and `stdlib.validate` add the opt-in
   toolbox/data/validation surfaces over existing source features and bridge
   intrinsics.)*
