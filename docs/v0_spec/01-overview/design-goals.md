## 1. Design goals

Zutai is a two-mode language system for data, configuration, validation, and pure data transformation.

Zutai has two file modes:

| Mode           | Extension | Purpose                               |
| -------------- | --------: | ------------------------------------- |
| Immediate mode |    `.zti` | inert data literal format             |
| General mode   |     `.zt` | pure lazy typed computation over data |

The central design split is:

> `.zti` is data. `.zt` is computation over data.

Immediate mode is designed to be deterministic, SIMD-friendly, non-evaluating, and suitable for both full parsing and daemon-side lazy materialization.

General mode is designed to be a pure, lazy, expression-oriented scripting language with a full type system, first-class type-level computation, and metaprogramming facilities.

### Data-oriented design

The primary abstractions in Zutai are structured data: records, lists, atoms, and unions. Computation is expressed as pure transformations over collections rather than mutation of individual objects. The type system — closed records, row-polymorphic views, union types with variants — is designed to represent data layouts directly, not to model object hierarchies.

Idiomatic Zutai code processes batches of data through pipelines:

```zt
items
  |> filter (\x => x > 0)
  |> map (\x => x * 2)
  |> fold (\acc x => acc + x) 0
```

### Functional design

All values are immutable. There is no mutation or assignment in the core language.

Functions are first-class and curried. The `|>` pipeline operator is the idiomatic way to chain transformations. Pattern matching over union types is the primary branching mechanism — `if` exists for boolean conditions, but multi-variant dispatch uses `match`.

Type-level computation uses the same pure expression language as runtime code. Type constructors are ordinary functions that return `Type`.

---

