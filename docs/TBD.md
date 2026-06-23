# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

## Near-term hardening

### Per-layer typing for the forall-lambda lowering

`lower_lambda` (`crates/general/tlc/src/lower/expr.rs`, the `forall_layers`
block) wraps every TyLam and dictionary `Lam` layer of a polymorphic lambda
expression with the lambda's full `outer_ty` instead of peeling one
quantifier/arrow per layer. This is the same shape as the value-parameter
currying bug fixed 2026-06-22 (commit `c81f207`), where reusing one type
across curried layers made an inner layer's declared type disagree with its
bind and aborted the Dataflow Core structural validator with an
internal-compiler-error panic.

Not yet a confirmed live bug: the value-parameter layers just above it are
peeled correctly, and the only reachable trigger — a polymorphic class
witness such as `Functor @(Result E) { map = \f r. r; }` — compiles without
an ICE today, because an unused witness is not lowered and witness-method
dispatch does not yet reach the backend through a value binding. TyLam/dict
layers also validate differently from the value-`Lam` `Fun`-param check that
the original bug tripped. Treat as defensive hardening: peel one
`ForAll`/`Fun` per layer so each layer carries `param -> rest`, mirroring the
correct per-layer wrapping in `crates/general/tlc/src/lower/decl.rs`, and add
a backend compile test once a polymorphic witness can be invoked end to end.

### Differential-corpus coverage for value rendering

The compiled-vs-interpreter differential gate compares stdout, so any
divergence in how the two render a value is invisible unless the corpus
exercises that shape. Record field ordering was such a silent divergence
(the backend sorts fields by name for slot layout; the interpreter kept
source order) and went unnoticed until 2026-06-22 because every exercised
record happened to be alphabetical (fixed in commit `e0e8235`, interpreter
`Display` now sorts). Expand the differential corpus to cover
non-alphabetical records, variants, nested tuples, text escaping, and
negative integers so the next rendering divergence fails a test instead of
shipping silently.

## v1 native-backend lowering

v1 is **semantically complete**: every feature parses, type-checks through
THIR, elaborates to TLC, and runs in the `zutai-eval` reference interpreter
that serves as the differential oracle (`docs/ARCHIVED.md` "Current
baseline"). What is missing for "complete" by the v0 bar — full LLVM/native
lowering plus runtime — is the **backend half of the pipeline** for most v1
features. None of this is currently scheduled; the project has deliberately
settled v1 at check-plus-interpreter and deferred native parity. Support
levels below cite the manual's "Implemented extensions beyond v0" table
(`docs/language-manual.md`).

Ranked by remaining work:

1. **Constraints / witnesses / derive (largest).** Direct, bounded,
   conditional, imported, operator, method-level, and higher-kinded witnesses
   plus structural Show/Ord derive are supported through THIR/TLC dictionary
   passing and the interpreter, but the table states **no full native-backend
   parity**. Completing this means lowering dictionary passing — witness
   records, superconstraint/conditional resolution, and higher-kinded
   instantiation — through Dataflow Core -> ANF -> SSA -> LLVM and the runtime,
   the path that record/tuple/union values already take. This is an entire v1
   area at zero native support.

2. **Row polymorphism (large).** Parser/HIR/THIR/TLC carry row variables and
   the interpreter runs row-typed code as ordinary records/unions, but only
   **concrete value-level `select`** is confirmed to lower through
   DC/ANF/SSA/LLVM. Open records/unions, named row tails (`...Rest`), and
   union extension (`...Shape`) are not confirmed past TLC. Completing this
   means confirming row erasure onto the existing record/union backend (or
   adding explicit lowering) plus a backend compile-test corpus for row-typed
   programs.

3. **Residual-effect runtime (medium; partly a non-goal).** Effects a `handle`
   fully discharges are CPS-elaborated to ordinary functions/matches before
   TLC->DC and lower natively today; ambient `io.print` lowers to the runtime
   `HostPrint` path. Effects that escape to the entry boundary other than
   `io.print`, open effect rows, and effectful entry shapes the runtime ABI
   cannot display are **rejected** before Dataflow Core
   (`docs/v1_spec/05-effects.md` "Laziness and Ordering"). A general
   residual-effect ABI is the gap; whether it is in scope is itself a design
   decision — the strict AOT backend may keep rejecting unhandled effects.

4. **Explicit universe-level syntax (small; mostly v2).** THIR/TLC carry
   internal universe levels, but surface level syntax is unimplemented;
   level-polymorphic constructors default to the lowest consistent universe
   and erase before backend lowering (`docs/ARCHIVED.md` "Current baseline").
   Full stratification is the deferred v2 universe-levels design.

**By design, not gaps** — do not file these as missing native work:

- **Reflection** (`fields`/`variants`/`schema`/`witness`) is compile-time:
  `compile`/`dataflow` fold serializable reflection such as `schema` to backend
  constants and reject residual reflection before lowering. Fold-or-reject is
  the intended model, so reflection is already as native as it should be.
- **Rejecting unhandled non-`io.print` effects** at the backend is the
  committed strict-AOT behavior, not an unfinished feature.
- **Annotation-required inference** where row/constraint inference is not
  principal is specified behavior (`docs/v1_spec/01-row-polymorphism.md`
  "Extended Inference"), not a backend gap.

## Deferred to v2/v3

### Garbage collection — deferred behind tail-call optimization

Decision (A), 2026-06-22: the native backend commits to **strict semantics plus
tail-call optimization** (Phase 31), with garbage collection **deferred**. The
compiled heap stays leak-by-default inside the capped thread-local arena
(Phase 30); the cap bounds the blast radius, and finite strict programs
terminate within it.

Empirical basis (measured with `ZUTAI_HEAP_STATS`):

- Before TCO the native stack was the binding constraint — recursion overflowed
  around 10^5–10^6 frames, far below the heap cap (~10^7–10^8 objects). GC could
  not help those programs; TCO could, and now does.
- After TCO, deep tail recursion runs in O(1) stack and the heap becomes the
  binding constraint, so GC is now a meaningful space optimization for
  bounded-live / unbounded-allocation programs (accumulator loops).
- ~2/3 of accumulator allocation is calling-convention overhead (one arg-tuple
  and one closure per curried call), not user data — so **uncurrying** would cut
  more allocation than GC for typical code.

Reordered trajectory (was: leak → mark-sweep → generational copying):

1. **Uncurrying / known-call optimization** — collapse saturated curried calls
   to direct multi-arg calls, removing the per-call closure+tuple churn.
2. **Collector**, only once a real GC workload exists (unbounded streams reach
   the backend, or accumulator garbage dominates). Root-finding, not the
   algorithm, is the open gap: untagged `i64` values (D-0002) make conservative
   scanning ambiguous, so a precise moving collector needs a shadow stack or
   stack maps — an ABI change beyond D-0008/D-0009. A conservative non-moving
   mark-sweep is the zero-ABI bridge; Cheney copying is the endgame and stays
   write-barrier-free only while the heap is write-once.
3. A lazy backend (thunk update = mutation = old→young pointers) would force a
   write barrier; that path is **not** taken — strict-plus-TCO is committed.

## Deferred beyond planned v3 work

GADT-style local type equalities and the coercion/cast core node (an explicit
non-goal, `tlc-core.md` §10), impredicative instantiation, unforgeable
capability tokens, and nominal recursive types remain unassigned to the active
v2/v3 roadmap.
