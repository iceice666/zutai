## Final design statement

Zutai has two coordinated modes.

`.zti` is pure immediate data. It is deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.

`.zt` is pure lazy typed computation over data. It provides top-level value, function, and type declarations, an `import` expression, one namespace, functions, records, tuples, lists, optionals, tagged union types, pattern matching, and parametric generics.

Atoms are prefixed with `#` in both `.zti` and `.zt`:

```zti
#prod
```

Record syntax and semicolon-terminated sequence syntax are intentionally shared between modes. `.zti` arrays correspond to `.zt` list values when imported. Evaluation, imports, functions, and types exist only in `.zt`. In type-context positions, record and union literals are parsed as type literals, so they do not repeat the `type` keyword.

The compact core statement is:

> `.zti` is inert data. `.zt` is pure, lazy, typed computation. Declarations
> use `::=` for inferred top-level values, `:: Type =` for typed values,
> `:: type` for type aliases, and `= patterns => body;` clauses for typed
> functions. `import` is a static literal expression. `;` terminates parallel
> declarations and container items; `[ ... ]` is a serial do-block. Records,
> lists, tuples, tagged unions, optionals, patterns, functions, row-polymorphic
> types, constraints, effects, reflection, recursive aliases, higher-rank
> annotations, and stream generators are all one language surface. The final
> expression of a `.zt` file is its output; a declaration-only file yields `()`.

See the [language specification index](../00-index.md) and
[grammar reference](../02-lexical/grammar-reference.md) for the complete stable
surface.
