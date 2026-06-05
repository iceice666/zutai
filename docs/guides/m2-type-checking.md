# M2 Type Checking

M2 is the semantic milestone that verifies HIR expressions and declarations are
well-typed. It runs after M1 name resolution, so expression names have already
been resolved to `SymbolId`s.

The current implementation lives in `zutai-semantic` and is registered as the
default semantic pass.

## Where M2 runs

The full semantic entry point is `zutai_semantic::analyze`:

```text
source text
  -> zutai_syntax::parse
  -> CST SyntaxNode
  -> zutai_hir::lower_file      (M1 name resolution)
  -> HIR
  -> elab::elab_file            (HIR type annotations -> semantic types)
  -> pass::default_passes
  -> TypeCheck                  (M2)
```

The important files are:

- `crates/general/semantic/src/lib.rs` for the `analyze` pipeline.
- `crates/general/semantic/src/pass.rs` for the pass registry.
- `crates/general/semantic/src/passes/type_check.rs` for M2.
- `crates/general/semantic/src/elab.rs` for annotation elaboration.
- `crates/general/semantic/src/ty.rs` for semantic type representation.
- `crates/general/hir/src/symbol.rs` for `Symbol::ty` write-back.

## Why M2 depends on M1

M2 does not resolve source names directly. It reads HIR where successful name
references already look like:

```text
HirExprKind::Var(symbol_id)
```

That means M2 can look up the referenced symbol and its type without repeating
lexical scope logic. If M1 failed, the HIR uses `ERROR_SYM` or error expressions;
M2 should recover quietly where possible so one unknown name does not create a
large cascade of unrelated type errors.

## Core model

M2 uses semantic type IDs:

- `Ty` is the semantic type enum.
- `TyId` is a handle into the type interner.
- `TyInterner` stores and deduplicates semantic types.
- `HirTyRef` records a semantic type ID back into HIR symbols.

Primitive types have stable pre-interned IDs:

```text
Unknown, Int, Float, Text, Bool, None
```

Composite types include optionals, lists, closed records, unions, tuples,
functions, type applications, atoms, and type parameters.

## Check mode and infer mode

M2 uses bidirectional type checking. The two main operations are:

```text
check(expr, expected_ty)
infer(expr) -> TyId
```

Use check mode when context already tells the compiler what type an expression
must have. Use infer mode when the compiler needs to synthesize a type from the
expression itself.

For an annotated binding:

```zt
port : Int = 8080
port
```

M2 elaborates `Int`, checks `8080` against `Int`, then writes `Int` back to the
`port` symbol.

For an inferred binding:

```zt
port := 8080
port
```

M2 infers `Int` from the literal and writes the inferred type back to the
`port` symbol.

That symbol type write-back is important for later declarations and later
compiler stages.

## Records

Record types are closed in v0. When a record expression is checked against a
known record type, M2 verifies that:

- Every required field is present.
- No unknown extra field is present.
- Each field value has the expected type.

This is valid:

```zt
Server :: type { host : Text; port : Int; tls? : Bool; }

server : Server = { host = "localhost"; port = 8080; }
server
```

This emits a semantic diagnostic because `tls` is not declared:

```zt
Server :: type { host : Text; port : Int; }

server : Server = { host = "localhost"; port = 8080; tls = true; }
server
```

Closed-record behavior is covered by the invalid fixture
`crates/general/fixtures/invalid/closed_records.zt`.

## Unions and atoms

In type position, atom literals become singleton types. A union like this:

```zt
Env :: type [#dev; #test; #prod;]
```

accepts only those atoms as values:

```zt
env : Env = #dev
env
```

This emits `E0030 TypeMismatch` because `#staging` is not a member:

```zt
Env :: type [#dev; #test; #prod;]

env : Env = #staging
env
```

The same rule applies when passing an atom to a function expecting `Env`.

Union membership behavior is covered by
`crates/general/fixtures/invalid/union_membership.zt`.

## Function calls

Function types are represented as nested `Ty::Function { param, ret }` values.
For a call, M2 infers the callee type, checks the argument against the parameter
type, and returns the function result type.

Example:

```zt
Env :: type [#dev; #test; #prod;]

greet :: Env -> Text
      :: #dev { "dev" }
      :: #test { "test" }
      :: #prod { "prod" }

greet #dev
```

Calling `greet #staging` should emit a type mismatch because `#staging` is not a
valid `Env`.

## Diagnostics

M2 currently emits:

- `E0021 UnknownField` for record fields that do not exist on the expected type.
- `E0030 TypeMismatch` when an expression does not match the expected type.

M1 diagnostics are merged into the final semantic result before M2 runs, but M1
still owns `E0020 UnknownIdentifier`.

## How to work on M2

Use the spec and tests together:

- Spec source of truth: `docs/v0_spec/05-type-system/`.
- Polymorphism model: `docs/v0_spec/06-polymorphism/polymorphism.md`.
- Error model: `docs/v0_spec/08-reference/error-model.md`.
- Acceptance tests: `crates/general/semantic/tests/acceptance.rs`.
- Elaboration and write-back tests: `crates/general/semantic/tests/elab.rs`.
- Semantic implementation plan: `docs/plans/semantic-analysis.md`.

When changing M2 behavior, prefer focused tests that isolate the rule being
changed. Good test cases usually assert one of these outcomes:

- A valid program has no semantic diagnostics.
- A specific invalid program emits `E0021 UnknownField`.
- A specific invalid program emits `E0030 TypeMismatch`.
- An annotated or inferred binding writes the expected type back to its symbol.

