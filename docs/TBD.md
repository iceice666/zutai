# Zutai Open Work

Open work is now grouped by deferral horizon. Completed milestones live in
`docs/ARCHIVED.md`; new implementation phases should be added here when scoped.

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
