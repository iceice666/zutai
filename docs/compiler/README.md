# Zutai Compiler Internals

These documents describe the post-frontend compiler pipeline and runtime
contracts. They are contributor references, not the normative language
specification.

```text
Source → HIR → THIR → TLC → Dataflow Core → ANF → SSA → LLVM IR
```

- [TLC](tlc.md) — fully elaborated typed core with explicit polymorphism
- [Dataflow Core](dataflow-core.md) — graph IR with explicit sharing and recursion
- [ANF](anf.md) — scheduled `let`/`letrec` representation
- [Runtime and ABI](runtime-abi.md) — native runtime representation and linking contract

The crate ownership map remains on the main [documentation index](../README.md#compiler-layer-ownership).
