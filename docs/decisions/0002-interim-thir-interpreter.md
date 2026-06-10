# Decision 0002: Interim THIR Reference Interpreter

## Status

Accepted.

## Decision

Add `crates/general/eval/` (`zutai-eval`): an interim tree-walking interpreter that evaluates
type-checked THIR and provides a working REPL, plus `run <path>` and `repl` subcommands in the
CLI.

The interpreter is a *semantics oracle*: it REFUSES to evaluate any program that is not fully
type-checked (`is_thir_complete()` must be true), so it can never silently produce a wrong
result.

## Rationale

The v0 implementation roadmap declares the old tree-walking interpreter "superseded" by the
AOT pipeline (`THIR → TLC → Dataflow Core → ANF → SSA → LLVM IR`) and defers a REPL to "JIT
compilation … until the LLVM backend is stable."  Those statements are about the *production
compilation strategy* — they do not preclude a reference tool.

The back half of the pipeline (TLC, Dataflow Core, ANF, SSA, LLVM) does not exist yet.  That
leaves no way to execute a `.zt` program for the foreseeable future.  An interim interpreter
over THIR:

1. **Unblocks development**: `.zt` programs can be run immediately, enabling semantic
   experimentation and debugging before any LLVM work.
2. **Provides a golden oracle**: The interpreter's output becomes the ground truth for future
   differential testing of the LLVM backend.  Any LLVM output that disagrees with the
   interpreter is a codegen bug.
3. **Complements, not replaces, the AOT path**: The production compile target remains LLVM.
   The interpreter is a parallel reference tool, not an alternative compilation strategy.

"Superseded" remains true for the *compilation* strategy: laziness is represented structurally
in Dataflow Core, not via runtime thunks.  This interpreter *locally* re-introduces thunks —
an explicitly acknowledged cost for a reference tool.

## Target IR: THIR (now), TLC (later)

THIR is targeted immediately because:

- It already exists and gate-checks programs completely.
- At runtime, types are erased.  v0 has no type-directed dispatch (parametric polymorphism is
  erased; typeclass/witness resolution is post-v0).  TLC's elaboration (explicit `TyLam`/`TyApp`)
  buys the interpreter nothing in v0.
- The runtime core (`value`, `thunk`, `env`, pattern matcher) is IR-agnostic — only `eval.rs`
  touches THIR.

**Migration trigger**: switch the eval walker to TLC when typeclass/witness resolution
(dictionary-passing) lands.  That is the first feature whose runtime behavior the interpreter
cannot read from THIR.  At that point, add a parallel TLC walker reusing the same runtime core,
keep both, and differential-test them.

## Scope: what the THIR interpreter evaluates today

Supported: literals, arithmetic/comparison/logic/`??`, let-blocks, `if`/`else`, records + field
access (including optional absent → `Nothing`), lists, tuples, named top-level functions with
recursion and parameter-level pattern matching (function clauses with literal, bind, tuple, and
record patterns, including guards), partial application (currying).

Not yet supported (THIR does not yet type-check these): anonymous lambda expressions,
standalone `match` expressions, imports, optional access (`?.`).  The pre-flight gate blocks
these programs — they cannot silently produce wrong results.  The evaluator arms for
lambda/match are written (gated as unreachable) and will activate when THIR or TLC type-checks
them.

## Known costs

- **Deliberate per-run `Rc` cycle**: the letrec top-level environment creates a `Closure →
  Env → Closure` cycle.  All `Rc`s are dropped at the end of `eval_file`.  Acceptable for a
  batch/interactive tool.
- **`BindingId` instability across analyses**: REPL sessions rebuild the env from scratch each
  turn.  Thunks are never cached across REPL turns.
- **No infinite-structure printing**: `force_deep` does not guard against infinite lazy lists.
  These cannot be produced by the current THIR gate (lambda/match are gated out) and are
  documented.
