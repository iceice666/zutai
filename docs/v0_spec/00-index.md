# Zutai Language Specification v0

This specification is split into multiple files, grouped by chapter.

## Chapter 1 — Overview

- [Design goals](01-overview/design-goals.md)
- [File modes](01-overview/file-modes.md)
- [Final design statement](01-overview/final-design-statement.md)

## Chapter 2 — Lexical Conventions

- [Lexical conventions](02-lexical/conventions.md)
- [Operator precedence](02-lexical/operator-precedence.md)
- [Core grammar sketch](02-lexical/grammar-sketch.md)

## Chapter 3 — Immediate Mode

- [Immediate mode `.zti`](03-immediate-mode/immediate-mode.md)

## Chapter 4 — General Mode

- [File structure](04-general-mode/file-structure.md)
- [Values](04-general-mode/values.md)
- [Imports](04-general-mode/imports.md)
- [Functions](04-general-mode/functions.md)
- [Conditionals](04-general-mode/conditionals.md)
- [Laziness and purity](04-general-mode/laziness-and-purity.md)

## Chapter 5 — Type System

- [Overview](05-type-system/overview.md)
- [Record types](05-type-system/records.md)
- [Lists](05-type-system/lists.md)
- [Optional values](05-type-system/optional-values.md)
- [Optional fields](05-type-system/optional-fields.md)
- [Field access and optional chaining](05-type-system/field-access.md)
- [Defaulting operator](05-type-system/defaulting-operator.md)
- [Union types](05-type-system/unions.md)
- [Tagged unions](05-type-system/tagged-unions.md)
- [Equality](05-type-system/equality.md)

## Chapter 6 — Polymorphism

- [Polymorphism](06-polymorphism/polymorphism.md)
- [Pattern matching](06-polymorphism/pattern-matching.md)

## Chapter 7 — Modules

- [Modules](07-modules/modules.md)
- [Serialization boundary](07-modules/serialization-boundary.md)

## Chapter 8 — Reference

- [Error model](08-reference/error-model.md)
- [Complete example](08-reference/complete-example.md)

---

Features deferred to v1: row polymorphism, type-level computation, constraint system, metaprogramming.
See [v1 spec index](../v1_spec/00-index.md).
