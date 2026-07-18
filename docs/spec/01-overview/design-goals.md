## Design goals

Zutai is a two-mode language system for data, configuration, validation, and pure data transformation.

Zutai has two file modes:

| Mode           | Extension | Purpose                               |
| -------------- | --------: | ------------------------------------- |
| Immediate mode |    `.zti` | inert data literal format             |
| General mode   |     `.zt` | pure lazy typed computation over data |

The central design split is:

> `.zti` is data. `.zt` is computation over data.

Immediate mode is designed to be deterministic, SIMD-friendly, non-evaluating, and suitable for both full parsing and daemon-side lazy materialization.

General mode is designed to be a pure, lazy, expression-oriented scripting language with a full type system and first-class type-level computation.

Compile-time reflection and metaprogramming are explicit, typed facilities; they
must not introduce ambient runtime behavior.

### Data-oriented design

The primary abstractions in Zutai are structured data: records, tuples, lists,
atoms, and unions. Immediate mode contains blocks and arrays; when imported into
general mode these correspond to record and list values. General mode also has
tuple values for structured alternatives and intermediate computation.
Computation is expressed as pure transformations over collections rather than
mutation of individual objects. Closed records describe exact layouts;
row-polymorphic views describe the fields a computation needs. Tagged unions
model finite alternatives rather than object hierarchies.

Idiomatic Zutai code processes batches of data through pipelines:

```zt
items
  |> filter (\x. x > 0)
  |> map (\x. x * 2)
  |> fold (\acc x. acc + x) 0
```

### Functional design

All values are immutable. There is no mutation or assignment in the core language.

Functions are first-class and curried. The `|>` pipeline operator is the idiomatic way to chain transformations. Pattern matching over union types is the primary branching mechanism. Boolean conditionals use `cond`, which desugars to the core `if`/`else` form; multi-case structural dispatch uses `match`.

Type-level computation uses the same pure expression language as runtime code. Type constructors are ordinary functions that return `Type`, and type-level evaluation is deterministic and bounded by implementation limits.

### Scope discipline

Language and library work serves the data workflow above. New abstractions
should be ordinary `.zt` libraries when the stable language can express them;
new syntax or a new trusted-core node requires a concrete program that cannot
be served by the existing library or explicit host boundary.

Compiler backends, packages, editor tooling, and browser integration support
and validate the language. Their independent feature completeness is not a
language-design goal, and tooling or deployment convenience alone does not
justify expanding the language core.

---
