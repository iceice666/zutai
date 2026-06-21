# Zutai Open Work

## Native codegen

### PIE-safe executable output

Status: TBD

Current native binary emission links Linux artifacts with `-no-pie` because the
LLVM IR can materialize global addresses through integer constants such as
`ptrtoint (ptr @symbol to i64)`, which produces relocations rejected by PIE
linking.

Acceptance criteria:

- Generated object files can be linked as PIE on Linux without `-no-pie`.
- Descriptor, text, atom, closure, and runtime-call lowering avoid relocation
  forms rejected by PIE linkers.
- `compile --emit=bin` still runs successfully for primitive, record, tuple,
  union, text, atom, and posit entry values.
- Documentation states whether native output is PIE-capable or non-PIE-only.
