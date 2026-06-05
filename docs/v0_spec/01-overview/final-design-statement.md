## 31. Final design statement

Zutai has two coordinated modes.

`.zti` is pure immediate data. It is deterministic, non-evaluating, and optimized for fast parsing and lazy materialization.

`.zt` is pure lazy typed computation over data. It provides three declaration forms, one namespace, functions, imports, records, tuples, lists, optionals, union types, tagged tuple union arms, pattern matching, and parametric generics.

Atoms are prefixed with `#` in both `.zti` and `.zt`:

```zti
#prod
```

Record and list syntax are intentionally shared between modes. Evaluation, imports, functions, and types exist only in `.zt`. In type-context positions, `{ ... }` and `[ ... ]` are parsed as type literals, so they do not repeat the `type` keyword.

The compact v0 specification is:

> `.zti` is inert data. `.zt` is pure lazy typed computation. Declarations use three forms: `name := expr` (inferred value), `name : Type = expr` (annotated value), and `name :: Sig` / `name :: patterns { body }` (function or type definition). Type-valued bindings are capitalized. Zutai has one namespace. Value records use `=` for field assignment: `{ field = value; }`. Record types use `:` for field annotation: `type { field : TypeExpr; }`. Record types are closed. Tuples use comma-separated positional or named elements. Union types use `type [ TypeExpr; ... ]`; a tuple whose first positional element is an atom singleton, such as `(#atom, field : Type)`, is interpreted as a tagged tuple union arm. Tuple values and patterns bind named fields with `=` (`(#atom, field = value)`). Optional values use `T?`. Optional fields use `field? : T`. Optional chaining uses `?.`. The defaulting operator is `??`. Pipelines use `|>` and `<|`. Anonymous functions use `\params => expr` (short) or `\params { block }` (block form). Pattern matching uses `match` with exhaustiveness checking, guard clauses (`if`), and nested patterns. Parametric polymorphism is declared with `[A, B]` lists immediately after `::`. Type parameters are unconstrained in v0 and implicitly instantiated at call sites. The final expression of a `.zt` file is its output.

Features deferred to v1: row polymorphism (open records, named row tails, open unions), first-class `Type` values and type-level computation, the constraint system (witnesses, `derive`, higher-kinded types), and metaprogramming (`fields`, `schema`). See [v1 spec](../../v1_spec/00-index.md).
