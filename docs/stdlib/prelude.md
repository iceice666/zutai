# Standard Library: Prelude

## Status

Accepted for post-v0 library design. Implementation is staged; see *Build order* below.

The prelude is the small set of names that are in scope in every `.zt` module without an
explicit `import`. It is the only auto-imported tier; everything else is an explicit
[module](00-index.md).

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
bridge `listFoldlStrict`; and dynamic load `loadZti`/`loadZt` — plus the builtin
type constructors (`List`, `Optional`, `Maybe`, `Patch`, `DeepPatch`). The
**source** layer is two ambient prelude files, both `include_str!`d in the HIR
lowerer with their declarations injected as a fallback
(`crates/general/hir/src/lower/prelude/`):

- the *stream* prelude `STREAM_MODULE_SRC` (`stream.zt`), so `Stream`,
  `StreamEff`, `Step`, and the non-conflicting combinators `empty`/`cons`/
  `singleton`/`unfold`/`take`/`drop`/`toList`/`fromList`/`takeList` are in scope
  without an import (V3-G2);
- the *function/list* prelude `PRELUDE_MODULE_SRC` (`prelude.zt`), so
  `id`/`const`/`compose`/`flip` and the `List` verbs `fold`/`foldl'`/`map`/
  `filter`/`length`/`append`/`uncons`/`head?`/`tail?` are in scope without an
  import (stdlib slices B/C).

Both are importable via `import stdlib.stream` / `import stdlib.prelude`.
Stream `map`/`filter`/`fold`/`uncons` are still exported by `stdlib.stream`, but
are not ambient because the unqualified names now denote the `List` verbs. Use a
qualified stream import (`s.map`, `s.fold`, `s.uncons`) when both surfaces appear
in one program.

## Contents

The prelude is deliberately focused — only what every pipeline needs:

```text
Types        Type Text Bool Int Float List Optional Maybe   (intrinsic)
Stream       Stream StreamEff Step; empty cons singleton unfold
             take drop toList fromList takeList             (ambient source prelude)
Effect       print                                           (intrinsic)
Function     id const compose flip                           (ambient source prelude)
List verbs   fold foldl' map filter length append uncons head? tail?
                                                             (ambient source prelude)
```

Rationale:

- The **types** are needed to write any signature; they are already intrinsic type
  constructors.
- The **list verbs** are the advertised idiom (`docs/spec/v0/05-type-system/lists.md`):
  `items |> filter pred |> map f |> fold step init`. `fold` is the strict left fold.
- `id` is the no-op combinator pipelines and higher-order functions reach for.
- `print` is the one host effect (`io.print`); it stays a pipeline-friendly tap.

Excluded from the prelude on purpose:

- `foldr`, `zip`, `flatten`, `reverse`, `take`, `drop`, search/sort/grouping
  helpers, and numeric aggregates remain explicit `list` work.
- All of `text`, `num`, `optional`, `result`, `cmp`, and `reflect` are explicit
  imports.

Naming note: the list-specialized `map`/`fold` may later become the
witness-dispatched `Functor`/`Foldable` methods once v1 constraints land
(`docs/spec/v1/03-constraints.md`). Stream methods with the same names remain
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
- `include_str!` each prelude file (`stream.zt`, `prelude.zt`) and parse it.
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

The prelude **must not** re-export backend-gated intrinsics (`fields`, `schema`,
or residual/partial `overlay`/`overlayDeep` forms): they stay behind explicit
`reflect`/`config` imports until their full backend lowering is available.

## Error-handling boundary

The error story splits by **control vs data**:

```text
effect (fail / warn)        = control: propagate, abort, capability boundaries
Result / Validation (data)  = collect, store, pattern-match, serialize
```

`fail`/`warn` (`docs/spec/v1/05-effects.md`) is the *default, blessed* error idiom and is
**not** mirrored by a prelude `Result`. But `Result` (and an accumulating `Validation`)
earn an explicit `result` module, because effects do not cover:

| Need | `fail` / `warn` | `Result` / `Validation` |
| --- | --- | --- |
| Short-circuit / abort / capability | blessed | verbose |
| Accumulate **all** errors (config normalize) | `fail` short-circuits | `Validation (List E)` |
| Serialize errors across the `.zti` boundary | control, not data | ordinary union |
| Compile to a backend binary today | handled effects lower natively (native effect parity, 2026-06-26); unhandled and non-`io.print` resource effects still gate at TLC→DC | pure, lowers now |

So errors-as-data are an opt-in module, not prelude, and not the default. Native
effect lowering has landed for handled effects, but accumulate-all error
collection remains a distinct idiom that `Validation (List E)` covers cleanly
(`fail` short-circuits by design); `Validation` is expected to stay.

## Build order

1. List-destructuring patterns + strict-`fold` intrinsic + `map`/`filter`/`fold`
   in source. *(still open — milestone C)*
2. `prelude.zt` resolution. *(landed for the function prelude: `id`/`const`/
   `compose`/`flip` are ambient source decls via HIR-lowerer injection, importable
   as `stdlib.prelude`; the list-verb half waits on step 1)*
3. `list`, `optional`.
4. `stream` landed as ambient prelude **and** importable embedded `stdlib.stream`
   (V3-G2/G6); `Stream A` is codata over recursive types, not a deferred module. ✅
5. `result` (with `Validation`).
6. Intrinsic-heavy `text`, `num`.
7. Fold `config`/`reflect` under the import scheme.
