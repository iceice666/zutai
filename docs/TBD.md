# Zutai Open Work

Open work falls in two independent groups: two **v1** milestones that each
remove a remaining AOT-backend rejection gate (a construct that already
type-checks and evaluates but does not yet compile natively), and a
**native-codegen** hardening item. The v1 typing and reference-evaluation
surface is otherwise complete — see [`ARCHIVED.md`](ARCHIVED.md) Phases 1–20.
The remaining v1 milestones are independent (different builtins/gates) and may
be implemented in any order; they are listed in recommended execution order.

Out of scope by v1 spec, deliberately not milestones: host capabilities beyond
`io.print` (filesystem/network/environment/clock/randomness, reserved as
non-ambient in `v1_spec/05-effects.md`), user-defined `derive` recipes
(`v1_spec/03-constraints.md` marks post-v1), and universe-level enforcement
(`v1_spec/02-type-level-computation.md` states it as a "should"; type-level
evaluation is fuel-bounded so `Type : Type` is not a runtime-soundness risk).

## Phase 21: Config-overlay AOT lowering

Goal: compile `overlay`/`overlayDeep` programs by lowering them to ordinary record
operations before Dataflow Core.

Removes the gate: `analysis.config_overlay_builtin_program()` exits in CLI
`compile`/`dataflow` (`crates/cli/src/commands.rs`).

Fixed design: lower `overlay`/`overlayDeep` to ordinary `Record`/field-presence
(`Maybe`) operations matching the reference-evaluator semantics
(`crates/general/eval/src/value.rs` `BuiltinFn`) and the merge rules in
[`stdlib/config.md`](stdlib/config.md) — shallow replaces present fields and keeps
absent ones; deep recursively merges present records and replaces scalars/lists/
union tuples; `#none` is a value, not deletion. `Patch`/`DeepPatch` stay type-level
utilities (compiler-backed type constructors, `stdlib/config.md`).

Acceptance criteria:

- `defaults |> overlay patch` over two record literals compiles via
  `compile --emit=llvm` and runs (via `--emit=bin`) to the merged record the
  reference evaluator yields.
- A nested-record `overlayDeep` program compiles and runs, recursively merging the
  nested record rather than replacing it.
- Unknown-field / type-mismatch patches remain type-check errors (no new runtime
  failure path), per `stdlib/config.md`.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean.

## Phase 22: Reflection AOT lowering

Goal: compile `fields`/`schema` programs by const-folding compile-time reflection
into ordinary backend values (both builtins).

Removes the gate: `analysis.reflection_builtin_program()` exits in CLI
`compile`/`dataflow` (`crates/cli/src/commands.rs`).

Fixed design: reflection is compile-time and its `Type` argument is statically
known, so `fields T` / `schema T` const-fold to literals before Dataflow Core.

- `schema T` → ordinary serializable `Record`/`List`/`Text`/`Bool` data exactly as
  the reference evaluator produces it (`v1_spec/04-metaprogramming.md` shapes); open
  rows rejected as today.
- `fields T` → a `List` of `{ name: Text; Type: <type value>; optional: Bool }`
  records. The embedded `Type` value lowers to a runtime pointer to the static type
  descriptor codegen already emits (`ARCHIVED.md` Phase 18); a runtime `Type` value
  *is* that descriptor pointer.
- Preserve the Phase 18 `@main`/`zutai.show` invariant: a final rendered value
  containing a `Type` field is rejected with the existing Type-result diagnostic
  (a `Type` value is not renderable); programs consuming only `.name`/`.optional`
  (`Text`/`Bool`) render normally.

Acceptance criteria:

- `schema Server` for a closed record and a closed union compiles via
  `--emit=llvm`/`--emit=bin` and the binary renders the same serializable structure
  the reference evaluator prints.
- A program reading `(fields Server)` metadata — e.g. the first field's `.name` and
  `.optional` — compiles and runs to the matching `Text`/`Bool` values.
- A `@main` that would render a raw `Type` value (e.g. bare `fields Server`) is
  rejected with the existing Type-result compile diagnostic, not a backend crash.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean.

## Native codegen

### PIE-safe executable output

Status: TBD

Current native binary emission links Linux artifacts with `-no-pie`
(`runtime_link_flags` in `crates/cli/src/commands.rs`) because the LLVM IR can
materialize global addresses through integer constants such as
`ptrtoint (ptr @symbol to i64)` (`crates/general/codegen/src/lib.rs`), which
produces relocations rejected by PIE linking.

Acceptance criteria:

- Generated object files can be linked as PIE on Linux without `-no-pie`.
- Descriptor, text, atom, closure, and runtime-call lowering avoid relocation
  forms rejected by PIE linkers.
- `compile --emit=bin` still runs successfully for primitive, record, tuple,
  union, text, atom, and posit entry values.
- Documentation states whether native output is PIE-capable or non-PIE-only.
