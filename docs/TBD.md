# Zutai TBD

This document tracks open milestones and unresolved TBD items. Implemented
status, validation notes, archived decisions, and completed history live in
[`ARCHIVED.md`](ARCHIVED.md). When a milestone finishes, move a short status
summary there and leave any unresolved follow-up here.

## Open milestones / TBD

### Phase 19 (TBD): Effect lowering past TLC

Goal: decide and implement how algebraic effects cross the AOT pipeline boundary.
The current implementation is complete through parser/HIR/THIR/TLC typing and the
TLC reference evaluator; Dataflow Core, ANF, SSA, LLVM, and the runtime ABI remain
pure and reject residual effect syntax.

Current support:

- [x] Parse/lower `perform`, `handle ... with { ... }`, `resume`, and function
  effect rows.
- [x] Type-check effect rows, standard aliases (`fail`, `warn`, `log`, `ask`),
  dotted operation names such as `fs.read`, handler forwarding/removal, resume
  result types, and the v1 one-shot `resume` rule.
- [x] Lower effect rows and `Perform`/`Handle`/`Resume` into TLC.
- [x] Run handled effects in the TLC reference evaluator using delimited
  continuations.
- [x] Route `print` through `io.print`; let source handlers intercept it; handle
  residual `io.print` at the host `run` boundary.
- [x] Reject residual effects in `compile`/`dataflow` with explicit diagnostics.

TBD:

- [ ] Pick the backend representation: explicit effect IR in Dataflow Core,
  lowering to ANF sequencing, CPS/free-monad elaboration before DC, or another
  design that preserves the lazy pure core.
- [ ] Define host capability boundaries beyond `io.print` without adding ambient
  filesystem/network/time/randomness primitives.
- [ ] Lower handled effects and residual host effects past TLC without silently
  erasing non-empty function effect rows.
- [ ] Add end-to-end compile tests for effect programs only after the lowering is
  deterministic and the runtime ABI can execute compiled binaries.

Verification gate: `check` and `run` behavior must continue matching
`docs/v1_spec/05-effects.md`; `compile`/`dataflow` must either execute the same
semantics end-to-end or keep rejecting residual effects with precise diagnostics.

### Phase 18: Runtime and ABI

Goal: make emitted LLVM IR link and run. The pipeline is feature-complete through
IR text emission, but previous backend gates asserted IR shape only. The runtime
ABI design and decisions D-0001…D-0010 live in [`docs/runtime-abi.md`](runtime-abi.md).

Remaining backend work tracked by this phase: dense union tags are still name
hashed; `@main` still prints every non-posit result with `print_i64` regardless
of type; and `compile` still emits LLVM text without assembling/linking a native
binary.

- [x] **Runtime crate skeleton** — `crates/general/runtime/` (`zutai-rt`,
  `staticlib` + `rlib`): bump arena, object headers, and the `@zutai.*` ABI for
  record/tuple/list/variant/text constructors and accessors, `coalesce`, raw
  `print_*`, and type-directed `show` from static descriptors.
- [x] **Uniform closure ABI (D-0003)** — replace the `{ __fn, caps }` record hack
  with a closure object `{ header, code, caps[] }` and a single curried
  application convention; top-level functions become empty-capture closures.
- [x] **Slot-indexed records (D-0004)** — carry resolved ordinal slots through
  `Select`/`RecordUpdate`; remove `str_hash` keying from codegen.
- [ ] **Dense variant indices (D-0009)** — assign union members dense tags in
  declaration order; thread union type into variant construction and matching.
- [ ] **Type descriptors + `@main` (D-0007/D-0009)** — emit static descriptors
  from `DfTy`; route `@main` through `zutai.show`; reject function/`Type` results
  with precise diagnostics.
- [ ] **Toolchain driver (D-0010)** — support `compile --emit=llvm|obj|bin`,
  invoke `llc`/`clang`, link `libzutai_rt`, use the host target triple, and emit
  actionable toolchain diagnostics.
- [ ] **Deferred GC** — ship v0 with the leak arena while reserving header layout
  bits for later precise and generational collectors.

Verification gate: compile runnable v0 fixture/spec programs to native binaries,
run them, and assert stdout matches the `zutai-eval` oracle; skip with notice
when `clang`/`llc` is unavailable. Keep cheap LLVM IR-text shape tests.

