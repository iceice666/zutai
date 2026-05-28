## 31. Final design statement

Zutai has two coordinated modes.

`.zti` is pure immediate data. It is deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.

`.zt` is pure lazy typed computation over data. It provides three declaration forms, one namespace, functions, imports, records, lists, pattern matching, first-class `Type`, type-level computation, and metaprogramming.

Atoms are prefixed with `#` in both `.zti` and `.zt`:

```zti
#prod
```

Record and list syntax are intentionally shared between modes. Evaluation, imports, functions, and types exist only in `.zt`. In type-context positions, `{ ... }` and `[ ... ]` are parsed as type literals, so they do not repeat the `type` keyword.

The compact specification is:

> `.zti` is inert data. `.zt` is pure lazy typed computation. Declarations use three forms: `name := expr` (inferred value), `name : Type = expr` (annotated value), and `name :: Sig` / `name :: patterns { body }` (function or type definition). Types are first-class compile-time values of `Type`. Type-valued bindings are capitalized. Zutai has one namespace. Value records use `=` for field assignment: `{ field = value; }`. Record types use `:` for field annotation: `type { field : TypeExpr; }`. Record types are closed by default and may be opened with row tails `...;` and `...Rest;`. Union types use `type [ TypeExpr; ... ]`. Tagged variants use `(#atom, field : Type, ...)` syntax for their type; construction and pattern matching bind each field with `=` (`(#atom, field = value)`), consistent with value records. Optional values use `T?`. Optional fields use `field? : T`. Optional chaining uses `?.`. The defaulting operator is `??`. Pipelines use `|>` and `<|`. Selective projection uses `select x { field; ... }`. Anonymous functions use `\params => expr` (short) or `\params { block }` (block form). Pattern matching uses `match` with exhaustiveness checking, guard clauses (`if`), and nested patterns. The final expression of a `.zt` file is its output. Type-level computation is powerful but bounded by deterministic compile-time evaluation limits.
