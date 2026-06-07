# Zutai Documentation

## Sections

- [v0 language specification](v0_spec/00-index.md)
- [v1 deferred feature specification](v1_spec/00-index.md)
- [Standard library](stdlib/00-index.md)
- [Design decisions](decisions/0001-record-update-and-config-layering.md)

## General-mode compiler layers

The general-mode implementation uses these semantic layers:

```text
Parser AST -> HIR -> THIR -> Semantic facade -> Core/Eval IR
```

- HIR is resolved, source-preserving, and not fully typed.
- THIR is the typed high-level IR produced by type checking and elaboration.
- The semantic facade wires parsing, HIR lowering, and THIR lowering into one staged analysis API.
- Passes live in the IR crate they transform; the semantic facade controls ordering and aggregation.
- Core/Eval IR is reserved for the later lazy evaluator representation.
