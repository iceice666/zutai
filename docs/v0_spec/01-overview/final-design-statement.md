## Final design statement

Zutai has two coordinated modes.

`.zti` is pure immediate data. It is deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.

`.zt` is pure lazy typed computation over data. It provides three declaration forms, one namespace, functions, imports, records, tuples, lists, optionals, union types, pattern matching, and parametric generics.

Atoms are prefixed with `#` in both `.zti` and `.zt`:

```zti
#prod
```

Record syntax and semicolon-terminated sequence syntax are intentionally shared between modes. `.zti` arrays correspond to `.zt` list values when imported. Evaluation, imports, functions, and types exist only in `.zt`. In type-context positions, record and union literals are parsed as type literals, so they do not repeat the `type` keyword.

The compact v0 specification is:

> `.zti` is inert data. `.zt` is pure lazy typed computation. Declarations use three forms: `name := expr` (inferred value), `name :: Type = expr` (typed value), and `name :: Sig { | patterns => body; }` / `name :: type TypeExpr` (function or type definition). Functions with a `::` signature use a `{ }` block containing `|` clauses; no-sig single-clause definitions use `name pattern = body`. Top-level declarations are separated by line boundaries at delimiter depth zero. Type values are first-class compile-time values, may be imported or exported by `.zt` modules, and are not serializable outputs. Type-valued bindings are capitalized. Zutai has one namespace. Value records use `=` for field assignment: `{ field = value; }`. Record types use `:` for field annotation: `type { field : TypeExpr; }`. Record types are closed. Tuple types with named fields use forms such as `(#atom, field : Type)`; tuple construction and pattern matching bind named fields with `=` (`(#atom, field = value)`). Union types use `type [ TypeExpr; TypeExpr; ]`, and structured alternatives are represented as ordinary tuple types inside the union. Tuple values are general-mode values and are not part of `.zti` serialization in v0. Optional values use `T?`, which aliases `Optional T` (`#none` or `(#some, value : T)`). Optional fields use `field? : T` and field access returns an optional value. Optional chaining uses `?.`. The defaulting operator is `??`. Pipelines use `|>` and `<|`. Anonymous functions use `\params. expr`. Pattern matching uses `match` with exhaustiveness checking, guard clauses (`if`), and nested patterns. Both `match` bodies and function bodies use the same `{ | pattern => expr; }` block form — `FuncClause` and `MatchCase` are the same grammar production. Parametric polymorphism is declared with `<A, B>` type parameter lists immediately after `::`. Type parameters are unconstrained in v0 and implicitly instantiated at call sites. The final expression of a `.zt` file is its output.

Features deferred to v1: row polymorphism (open records, named row tails, open unions), selective projection (`select`), the constraint system (witnesses, `derive`, higher-kinded constraints), reflection APIs, and metaprogramming (`fields`, `schema`). See [v1 spec](../../v1_spec/00-index.md).
