# Zutai Language Specification

This is the single specification for the current Zutai language. Zutai no
longer assigns language features to numbered version buckets. If a
surface form is accepted by the parser and listed here, its syntax is stable.
Implementation limits are stated as support levels; they are not language
versions.

Stable syntax means existing accepted source will not be repurposed or removed
without an explicit compatibility decision. It does not mean every construct
can execute through every backend. Each feature document distinguishes syntax,
checking, reference-interpreter, and LLVM/native support where those differ.

## Chapter 1 — Overview

- [Design goals](01-overview/design-goals.md)
- [File modes](01-overview/file-modes.md)
- [Final design statement](01-overview/final-design-statement.md)

## Chapter 2 — Lexical Conventions

- [Lexical conventions](02-lexical/conventions.md)
- [Operator precedence](02-lexical/operator-precedence.md)
- [Core grammar reference](02-lexical/grammar-reference.md)

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
- [Equality](05-type-system/equality.md)
- [Recursive types](05-type-system/recursive-types.md)
- [Type-level computation](05-type-system/type-level-computation.md)
- [Universe levels](05-type-system/universe-levels.md)

## Chapter 6 — Polymorphism

- [Polymorphism](06-polymorphism/polymorphism.md)
- [Pattern matching](06-polymorphism/pattern-matching.md)
- [Row polymorphism and selective projection](06-polymorphism/row-polymorphism.md)
- [Constraints and witnesses](06-polymorphism/constraints.md)
- [Higher-rank polymorphism](06-polymorphism/higher-rank-polymorphism.md)

## Chapter 7 — Modules

- [Modules](07-modules/modules.md)
- [Serialization boundary](07-modules/serialization-boundary.md)

## Chapter 8 — Effects and Host Capabilities

- [Algebraic effects](08-effects/algebraic-effects.md)
- [Host capabilities](08-effects/host-capabilities.md)

## Chapter 9 — Compile-time Reflection and Derivation

- [Reflection and schema reification](09-metaprogramming/reflection.md)
- [Derive recipes](09-metaprogramming/derive-recipes.md)

## Chapter 10 — Streams and Generators

- [Generator and yield syntax](10-generators/generators.md)

## Chapter 11 — Reference

- [Error model](11-reference/error-model.md)
- [Complete example](11-reference/complete-example.md)

## Stability and support

The executable syntax source of truth is
`crates/general/syntax/src/parser/`. The compact accepted grammar is the
[grammar reference](02-lexical/grammar-reference.md). The
[language manual](../language-manual.md) is the user-facing guide, while
[ARCHIVED.md](../ARCHIVED.md) records implementation evidence and
[TBD.md](../TBD.md) contains only concrete open work.

Zutai includes both canonical spellings and parser-accepted
compatibility spellings. New syntax requires a specification update, parser and
diagnostic coverage, semantic support or a precise refusal point, and an entry
in the manual's support table.
