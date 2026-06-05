# M1 Name Resolution

M1 is the first semantic milestone for general mode (`.zt`). Its job is to turn
surface name references into stable HIR symbol references.

In the current implementation, M1 runs during CST-to-HIR lowering in
`zutai_hir::lower_file`. It emits `E0020 UnknownIdentifier` diagnostics and
lowers resolved expression names to `HirExprKind::Var(SymbolId)`.

## Where M1 runs

The full semantic entry point is `zutai_semantic::analyze`:

```text
source text
  -> zutai_syntax::parse
  -> CST SyntaxNode
  -> zutai_hir::lower_file      (M1 name resolution happens here)
  -> HIR with SymbolIds
  -> annotation elaboration
  -> M2 type checking
```

The important files are:

- `crates/general/hir/src/lower/mod.rs` for the lowering entry point.
- `crates/general/hir/src/lower/decl.rs` for top-level collection and body lowering.
- `crates/general/hir/src/lower/ctx.rs` for symbol definition and lookup helpers.
- `crates/general/hir/src/scope.rs` for lexical scope lookup.
- `crates/general/hir/src/symbol.rs` for `SymbolId`, `SymbolKind`, and `ERROR_SYM`.

## Core model

M1 uses two related data structures:

- `ScopeStack` maps visible names to `SymbolId`s.
- `SymbolTable` stores the symbol data: name, kind, definition range, and later type.

When lowering sees a definition, it allocates a `Symbol` and registers the name
in the current scope. When lowering sees a name in expression position, it asks
the current `ScopeStack` to resolve that name.

If lookup succeeds, the expression becomes:

```text
HirExprKind::Var(symbol_id)
```

If lookup fails, lowering emits `E0020 UnknownIdentifier` and uses `ERROR_SYM` so
later passes can recover without panicking.

## Top-level names are recursive

The v0 spec says top-level declarations in a `.zt` file share one recursive
scope. That means a declaration body may refer to another top-level declaration
that appears later in the file.

This is valid:

```zt
first := second
second := 42
first
```

To support this, `lower_file_decls` uses two phases:

1. Pre-populate the file scope with built-in type names and every top-level
   declaration name.
2. Lower each declaration body and the final expression with those names already
   visible.

This also makes simple mutual top-level references resolve during M1:

```zt
left := right
right := left
left
```

M1 only resolves names. Whether recursive values make sense at type-checking or
evaluation time is handled by later milestones.

## Local bindings are sequential

Block-local `:=` bindings are different. A local binding is scoped only to the
remainder of the block, so the right-hand side is lowered before the new name is
defined.

This is valid:

```zt
{
  x := 1;
  y := x;
  y
}
```

This emits `E0020 UnknownIdentifier` for `y`:

```zt
{
  x := y;
  y := 1;
  x
}
```

That behavior is intentional: local bindings are not recursive.

## Other scopes

M1 opens child scopes for constructs that introduce temporary names:

- Lambda parameters.
- Function type parameters.
- Function clause patterns.
- Match-case patterns.
- Block-local bindings.

Each child scope points at its parent. Lookup starts from the current scope and
walks outward until it finds a name or reaches the root.

Zutai has one namespace in v0. Values, functions, type definitions, and type
parameters all resolve through the same scope mechanism, with different
`SymbolKind` values recording what was defined.

## Built-in type names

Lowering pre-populates the file scope with current built-in type names such as:

```text
Type, Int, Float, Text, Bool, List
```

This prevents annotations like `x : Int = 1` from producing unknown-identifier
diagnostics during M1.

## Diagnostics

M1 currently emits:

- `E0020 UnknownIdentifier` when a name cannot be resolved.

Duplicate top-level names are handled earlier by syntax validation, not reissued
by HIR lowering. If a name fails to resolve, M1 records the diagnostic and uses
`ERROR_SYM` as a recovery sentinel.

## How to work on M1

Use the spec and tests together:

- Spec source of truth: `docs/v0_spec/04-general-mode/file-structure.md`.
- Implementation tests: `crates/general/hir/tests/m1_name_resolution.rs`.
- Semantic pipeline notes: `docs/plans/semantic-analysis.md`.

When changing M1 behavior, add or update focused HIR lowering tests. Good test
cases usually assert one of these outcomes:

- A name resolves without lowering diagnostics.
- A specific source emits `E0020 UnknownIdentifier`.
- Top-level forward references still work.
- Local forward references still fail.

