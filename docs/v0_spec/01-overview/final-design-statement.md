## Final design statement

Zutai has two coordinated modes.

`.zti` is pure immediate data. It is deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.

`.zt` is pure lazy typed computation over data. It provides top-level value, import, function, and type declarations, one namespace, functions, records, tuples, lists, optionals, tagged union types, pattern matching, and parametric generics.

Atoms are prefixed with `#` in both `.zti` and `.zt`:

```zti
#prod
```

Record syntax and semicolon-terminated sequence syntax are intentionally shared between modes. `.zti` arrays correspond to `.zt` list values when imported. Evaluation, imports, functions, and types exist only in `.zt`. In type-context positions, record and union literals are parsed as type literals, so they do not repeat the `type` keyword.

The compact v0 specification is:

> `.zti` is inert data. `.zt` is pure lazy typed computation. Declarations include `name ::= expr` (inferred value), `name :: Type = expr` (typed value), `name :: import "path"` (static import binding), and `name :: Sig` followed by `= patterns => body;` / `name :: type TypeExpr` (function or type definition). `import` is a declaration, not an expression. Functions with a `::` signature use one or more `=` clauses; no-sig single-clause definitions use `name pattern = body`. Top-level declarations are separated by line boundaries at delimiter depth zero. Type values are first-class compile-time values, may be imported or exported by `.zt` modules, and are not serializable outputs. Type-valued bindings are capitalized. Zutai has one namespace. Value records use `=` for field assignment: `{ field = value; }`. Record types use `:` for field annotation: `type { field : TypeExpr; }`. Record types are closed. Tagged union types use `type { #name; #name; }` (pure enum) or `type { #name: { field: Type; }; #name; }` (tagged with payload); all members are semicolon-terminated. Tags carry `#` in both type definitions and value/pattern syntax: `#name` (singleton), `#name { field = val; }` (record payload), or `#name (val)` (tuple payload). Every tagged union value exposes a `.tag` accessor returning the atom tag. Tagged union values are general-mode values and are not part of `.zti` serialization in v0. Optional values use `T?`, which aliases `Optional T` (`#none` or `#some (x)`). Optional fields use `field? : T`; field access returns `Maybe T` (`#absent` or `#present (x)`) to preserve physical field presence. Optional chaining uses `?.` on `Optional` and `Maybe` receivers without flattening. The defaulting operator is `??` and unwraps one `Optional` or `Maybe` layer. Pipelines use `|>` and `<|`. Anonymous functions use `\params. expr`. Pattern matching uses `match` with exhaustiveness checking, guard clauses (`if`), and nested patterns. Match bodies use `{ | pattern => expr; }` arms. Function bodies use top-level `= pattern => expr;` clauses after the signature. Parametric polymorphism is declared with `<A, B>` type parameter lists immediately after `::`. Type parameters are unconstrained in v0 and implicitly instantiated at call sites. The final expression of a `.zt` file is its output.

Features deferred to v1: row polymorphism (open records, named row tails, open unions), selective projection (`select`), the constraint system (witnesses, `derive`, higher-kinded constraints), reflection APIs, and metaprogramming (`fields`, `schema`). See [v1 spec](../../v1_spec/00-index.md).
