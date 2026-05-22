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

---

