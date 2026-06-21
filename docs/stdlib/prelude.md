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

Today only the intrinsic layer exists: `BUILTIN_VALUE_NAMES` (`print`, `fields`, `schema`,
`overlay`, `overlayDeep`) seeded in `crates/general/hir/src/lower/mod.rs`, and the builtin
type constructors (`List`, `Optional`, `Maybe`, `Patch`, `DeepPatch`) recognized in the same
lowerer. The source prelude (`prelude.zt`) is the new piece.

## Contents

The prelude is deliberately focused — only what every pipeline needs:

```text
Types       Type Text Bool Int Float List Optional Maybe
List verbs  map filter fold
Function    id
Effect      print
```

Rationale:

- The **types** are needed to write any signature; they are already intrinsic type
  constructors.
- The **list verbs** are the advertised idiom (`docs/v0_spec/05-type-system/lists.md`):
  `items |> filter pred |> map f |> fold step init`. `fold` is the strict left fold.
- `id` is the no-op combinator pipelines and higher-order functions reach for.
- `print` is the one host effect (`io.print`); it stays a pipeline-friendly tap.

Excluded from the prelude on purpose:

- `const`, `compose`, `flip` live in `fn`. `compose` duplicates `|>`; `flip` usually
  papers over a bad argument order rather than fixing it.
- `foldr`, `foldl'`, `zip`, `flatten`, `length`, and friends live in `list`.
- All of `text`, `num`, `optional`, `result`, `cmp`, `reflect` are explicit imports.

Naming note: the list-specialized `map`/`fold` may later become the witness-dispatched
`Functor`/`Foldable` methods once v1 constraints land
(`docs/v1_spec/03-constraints.md`). That is a smooth migration, not a conflict.

## Source-canonical, intrinsic-optimized

For any function the language can express, the **`.zt` definition is the specification**;
a compiler-internal implementation is a *verified optimization of that exact binding id*,
never a second semantics. This avoids the drift trap where a hand-written intrinsic and the
source form disagree.

`map`/`filter`/`fold` are written in the source prelude using list-destructuring patterns
and recursion (recursion and SCC scheduling already exist: `docs/ARCHIVED.md`, Phases 2–3).
Compiler internals remain justified for two non-cosmetic reasons:

- **Strict `fold` (`foldl'`).** A lazy core leaks space on a naive left fold. Until a
  `seq`/`force` strictness primitive exists, the strict `fold` *cannot* be written in
  source, so its intrinsic is mandatory.
- **Spine access.** The runtime list value is a flat array (`Value::List(Rc<[Thunk]>)` in
  `crates/general/eval/src/value.rs`), making naive `[h; ...t]` tail O(n). Internals can
  lower to spine destructuring / backend loops.

Equivalence between the source form and any optimized lowering is enforced by differential
tests (source-defined path vs optimized path).

## List-destructuring patterns

`HirPatKind` (`crates/general/hir/src/ir.rs`) currently has no list pattern, which is why
list iteration is unwritable in source today. The prelude work adds:

```zt
match xs {
  | []        => empty-case;
  | [h; ...t] => cons-case using h and t;
}
```

Lists are non-finite, so exhaustiveness is **nil + (cons or wildcard)**, never an
enumeration of lengths. The change spans grammar/`ast.rs`, `HirPatKind`/`ThirPatKind`,
exhaustiveness (`crates/general/thir/src/lower/exhaust.rs`, with `Nil`/`Cons` constructors),
the evaluators, and DC/ANF/SSA/codegen.

## `prelude.zt` resolution

The source prelude is loaded by the semantic facade (`zutai-semantic`):

```text
- Parse + lower `prelude.zt` once, cache it.
- Prefix its declarations into every module's scope AFTER intrinsic seeding and BEFORE
  user name resolution.
- It imports nothing; it depends only on intrinsics + core syntax + list patterns.
- A parse/lower failure is an internal compiler diagnostic, not a user error.
```

Both evaluators must resolve prelude names as **real declarations**, uniformly: the THIR
oracle (`crates/general/eval/src/eval/top_env.rs`) and the TLC path
(`crates/general/eval/src/tlc_entry.rs`) currently seed only `BuiltinFn`s and need the
prelude decls threaded through.

The prelude **must not** re-export backend-gated intrinsics (`fields`, `schema`,
or residual/partial `overlay`/`overlayDeep` forms): they stay behind explicit
`reflect`/`config` imports until their full backend lowering is available.

## Error-handling boundary

The error story splits by **control vs data**:

```text
effect (fail / warn)        = control: propagate, abort, capability boundaries
Result / Validation (data)  = collect, store, pattern-match, serialize
```

`fail`/`warn` (`docs/v1_spec/05-effects.md`) is the *default, blessed* error idiom and is
**not** mirrored by a prelude `Result`. But `Result` (and an accumulating `Validation`)
earn an explicit `result` module, because effects do not cover:

| Need | `fail` / `warn` | `Result` / `Validation` |
| --- | --- | --- |
| Short-circuit / abort / capability | blessed | verbose |
| Accumulate **all** errors (config normalize) | `fail` short-circuits | `Validation (List E)` |
| Serialize errors across the `.zti` boundary | control, not data | ordinary union |
| Compile to a backend binary today | effects stop at TLC (Phase 19 TBD) | pure, lowers now |

So errors-as-data are an opt-in module, not prelude, and not the default. Revisit only if
Phase 19 effect lowering plus a reify-at-boundary handler convention ever cover accumulate-all
(unlikely to do so cleanly — `Validation` is expected to stay).

## Build order

1. List-destructuring patterns + strict-`fold` intrinsic + `map`/`filter`/`fold` in source.
2. `prelude.zt` resolution + seeding in `zutai-semantic` and both evaluators.
3. `fn`, `list`, `optional`.
4. `result` (with `Validation`).
5. Intrinsic-heavy `text`, `num`.
6. Fold `config`/`reflect` under the import scheme.
