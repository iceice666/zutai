# Macro Kernel Examples

A pressure-test suite for Zutai general mode's typed macro/metaprogramming
kernel: typed staging (`quote`/`splice`/`Code A`), derive recipes and generic
builders (`deriveShow`/`deriveOrdLex`/`deriveFromData`), reflection
(`fields`/`variants`/`schema`), witness dispatch, and the ambient structural
decoder (`FromData`/`decode`/`fromData`).

Run any example through the CLI (from the workspace root):

```sh
ZUTAI_STDLIB_ROOT=stdlib cargo run -q -p zutai-cli -- run examples/macro_kernel/decode/primitives.zt
ZUTAI_STDLIB_ROOT=stdlib cargo run -q -p zutai-cli -- check examples/macro_kernel/safety/conflicting_witness_rejected.zt
```

Each file's header comment states its intent and expected outcome. Files under
`bugs/` reproduce the soundness defects from the original pressure test; all
four are now fixed and serve as regression guards — see
`PRESSURE_TEST_REPORT.md` at the repo root for the full analysis.

## Layout

| Directory | What it exercises |
| --- | --- |
| `staging/` | Hygienic `quote`/`splice` roundtrip, nested quotation, compile-time `Code` leak and non-`Code` splice refusals |
| `derive/` | `deriveOrdLex` (delegating compare + `#eq` fallthrough), `deriveFromData` record decode, non-derivable and irreducible-recipe refusals |
| `reflection/` | `fields`, `schema`, `variants`, `witness C @T` dispatch, missing-witness refusal, recursive-type `schema` |
| `decode/` | Primitive decode, typed mismatch issues, missing required field, multi-error accumulation, absent optional, list-element error path, nested records + lists, deep nested error path |
| `safety/` | Conflicting-witness and method-arity refusals, annotation-driven decoder shape selection |
| `bugs/` | Minimal reproductions of soundness defects S1, S1b, S2, S3 (now fixed; regression guards) |

## Expected outcomes

Non-`bugs/` examples: `check` and `run` agree, and either succeed (exit 0) or
refuse cleanly with a `type error:` / parse diagnostic (exit 1) as their header
notes. Two exceptions are run-only reflection demos — `reflection/fields_record.zt`
and `reflection/variants_union.zt` — whose results embed `Type` values: `run`
shows them (exit 0), while `check`/`compile` refuse them as a program entry result
via the S4 entry-type gate (exit 1). Use `schema` for a serializable analog.

`bugs/` examples (defects now fixed; each row is the current, correct behavior):

| File | `check` | `run` | Resolution |
| --- | --- | --- | --- |
| `S1_witness_dispatch_unbound_crash.zt` | 1 | 1 | refuses cleanly: `no witness in scope for `Show @Text`` (was: run-time unbound-binding crash) |
| `S1b_inferred_structural_union_crash.zt` | 1 | 1 | refuses cleanly: `no witness in scope for `Show @#err`` (was: run-time crash via inferred structural-union operand) |
| `S2_derive_recursive_type_stack_overflow.zt` | 0 | 0 | recursive derive terminates and evaluates (was: check-time stack overflow, exit 134) |
| `S3_deriveShow_drops_values.zt` | 0 | 0 | delegates to component witness: renders `{x = <INT>, y = <INT>}` (was: `{x, y}`, names only) |
