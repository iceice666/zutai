## 31. Final design statement

Zutai has two coordinated modes.

`.zti` is pure immediate data. It is deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.

`.zt` is pure lazy typed computation over data. It provides immutable `let` bindings, one namespace, functions, imports, records, lists, pattern matching, first-class `Type`, type-level computation, and metaprogramming.

Atoms are prefixed with `#` in both `.zti` and `.zt`:

```zti
#prod
```

Record and list syntax are intentionally shared between modes, while evaluation, imports, functions, and types exist only in `.zt`. In type-context positions, such as type annotations, function-type operands, `forall` bodies, and nested forms inside `type { ... }` or `type [ ... ]`, `{ ... }` and `[ ... ]` are parsed as type literals, so they do not repeat the `type` keyword.

The compact specification is:

> `.zti` is inert data. `.zt` is pure lazy typed computation. All declarations are immutable `let` bindings. Types are first-class compile-time values of `Type`. Type-valued bindings are capitalized. Zutai has one namespace. Record types are closed by default with `type { field = TypeExpr; }` in expression context and `{ field = TypeExpr; }` in type context, and may be opened with row tails such as `...;` and `...Rest;`. Union types use `type [ TypeExpr; ... ]` in expression context and `[ TypeExpr; ... ]` in type context. Optional values use `T?`. Optional fields use `field? = T`. Optional chaining uses `?.`. Pipelines use `|>` and `<|` as syntax for ordinary function application. Selective projection uses `select x { field; ... }`. The final expression of a `.zt` file is its output. Type-level computation is powerful but bounded by deterministic compile-time evaluation limits.
