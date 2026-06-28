# Zutai Open Work

Open work is grouped by deferral horizon. Completed milestones and their
implementation detail live in `docs/ARCHIVED.md`; language design lives in
`docs/spec/v0/` (stable), `docs/spec/v1/`, `docs/v2_spec/`, and `docs/v3_spec/`.
New implementation phases should be added here when scoped.

## Status (2026-06-28)

The post-V3 readiness audit **landed 2026-06-28** (see `docs/ARCHIVED.md`
"Post-V3 readiness audit"). v1 semantics are complete, and native support is
release-ready inside the documented AOT envelope; v2 is largely native (four of
five features lower natively, universe levels erase before the backend); v3
Track 1 and its scoped follow-ups are complete. The
audit reconciled user-facing docs and support levels with the landed baseline,
and confirmed an executable coverage anchor for every
materially supported feature. The GC residual is retired (conservative
default-on mark-sweep is the committed endpoint). Track 2 remains
demand-gated and must not be implemented unless a concrete program forces one
of the reserved core-design boundaries.

The next concrete follow-up is **source-prelude / stdlib usability** work:
B (small function prelude) and C (minimal `List` verbs) **landed 2026-06-28**
(see `docs/ARCHIVED.md` "Small function prelude (stdlib slice B)" and "Minimal
List verbs (stdlib slice C)"). This is stdlib work, not Track 2, and does not
reopen any core language boundary.

## Source prelude / stdlib active work

Goal: make the planned source prelude real without changing V0–V3 core
semantics. Source definitions are the specification; compiler intrinsics are
allowed only as verified optimizations of the same binding or for operations
that source cannot yet express safely.

### B — Small function prelude ✅

_Landed 2026-06-28. `prelude.zt` ships `id`/`const`/`compose`/`flip` as ambient
source declarations (HIR-lowerer fallback, importable as `stdlib.prelude`);
user bindings shadow the prelude; interpreter/TLC/native agree on
representative higher-order uses. See `docs/ARCHIVED.md` "Small function prelude
(stdlib slice B)" for the summary._

### C — Minimal `List` verbs ✅

_Landed 2026-06-28. `prelude.zt` now ships ambient/importable `List` verbs
`fold`/`foldl'`/`map`/`filter`/`length`/`append`/`uncons`/`head?`/`tail?`, backed
by list nil/cons patterns through THIR/TLC/eval/native and a strict
`listFoldlStrict` bridge. Stream `map`/`filter`/`fold`/`uncons` remain available
through `import stdlib.stream`. See `docs/ARCHIVED.md` "Minimal List verbs
(stdlib slice C)" for the summary._

Non-goals for this slice: `optional`, `result`, `num`, `text`, `cmp`, full
stdlib completion, non-tail generator `yield from`, cross-module witness native
ABI, and all Track 2 boundaries.

## Tooling / test-harness backlog

- **Native-link test race under `cargo test --workspace`.** CLI native-compile
  tests (`compile_bin_stdout` and friends in `crates/cli/tests/cli.rs`) shell out
  to `cargo build` to materialize `target/debug/libzutai_rt.a`
  (`crates/cli/src/commands/toolchain.rs`), then invoke `clang` to link. Under
  `cargo test --workspace`, concurrent test threads each spawn a CLI process →
  inner `cargo build`, contending on the cargo package-cache lock; a `clang`
  invocation can reach the link step before `libzutai_rt.a` is linkable, failing
  with `clang: error: no such file or directory: '.../libzutai_rt.a'`. Repro:
  passes in isolation (`cargo test -p zutai-cli --test cli -- <name>`) and with
  `--test-threads=1`; flakes under parallel `cargo test --workspace`. Fix
  options: pre-build `libzutai_rt.a` before the test suite, have the CLI reuse a
  pre-built artifact instead of re-invoking `cargo build`, or serialize the
  native-link tests. Surfaced 2026-06-28 during the function-prelude verify step
  (the failure is independent of the function-prelude change, which never
  touches `zutai-rt`/codegen linking).

## v1 residual — by design, not gaps

Do not file these as missing native work:

- **Higher-kinded instantiation** (polymorphic dictionary passing) stays
  check-only by design — eval and compile both refuse HKT execution
  (`unify.rs` "a refused check is the safe direction"); a type-checking
  `mapTwice (\x. x) [1; 2;]` is rejected at the type-check stage on both paths.
- **Reflection** (`fields`/`variants`/`schema`/`witness`) is compile-time:
  `compile`/`dataflow` fold serializable reflection to backend constants and
  reject residual reflection (a raw `witness` dictionary or a `Type`-valued
  result) before lowering (`aot_reflection_program`). Fold-or-reject is the
  intended model.
- **Rejecting unhandled non-`io.print` effects** at the backend is the committed
  strict-AOT behavior.
- **Annotation-required inference** where row/constraint inference is not
  principal is specified behavior (`docs/spec/v1/01-row-polymorphism.md`
  "Extended Inference").
- **Cross-module witness exports** stay native-gated: imported witness
  dictionaries execute through the interpreter today, while native builds reject
  modules that export typeclass witnesses before Dataflow Core rather than
  silently dropping dispatch state. Promote this only if a concrete native
  module-witness use case requires it.

## Deferred beyond v2 (v3+)

Now sequenced in the **V3 roadmap** (`docs/v3_spec/02-roadmap.md`). Summary:

- **Track 1 — generators and streams. ✅ Complete (V3-G1…G5, 2026-06-25).**
  `Stream A` is **codata** (demand-driven step+seed), not a memoizing lazy list,
  so it stays inside strict+TCO and the write-barrier-free GC. The full spine
  landed: G1 (codata representation) → G2 (stdlib API) → G3 (richer `yield`) → G4
  (effectful generators, reference-interpreter) → G5 (GC keeps unbounded pipelines
  bounded). The G2 residuals (`empty`/`unfold`, `List` interop, importable `.zt`
  packaging) have all landed, as has G4 finalization (the `finally` handler
  clause, 2026-06-26), the ergonomic effectful-stream type (call-site effect-row
  inference + the `StreamEff` alias, 2026-06-27), cooperative cancellation
  (aborting the granting handler, 2026-06-27), and resource lifetime as the
  granting-handler dynamic-extent contract (2026-06-27).
- **Track 2 — reserved design boundaries (demand-gated, not a backlog)**
  (`docs/v2_spec/00-index.md` "Deferred beyond v2"): GADT-style local type
  equalities and the coercion/cast core node (an explicit non-goal,
  `tlc-core.md` §10), impredicative instantiation (v2 higher-rank polymorphism
  stays predicative), unforgeable capability tokens (v2 capabilities carry
  advisory authority), and nominal recursive types (v2 recursive types are
  equirecursive). Build one only when a concrete need drives it.

## Doc reconciliation (2026-06-24 audit)

Surfaced by the V1→V2 readiness audit; both resolved 2026-06-24:

- **Forward-dated ARCHIVED stamps.** Nine `2026-06-25` stamps in
  `docs/ARCHIVED.md` (the "Current baseline" Last-updated note plus eight
  completed-milestone entries) were one day ahead of the authoritative date;
  corrected to 2026-06-24.
- **Higher-rank support level.** `docs/v2_spec/05-higher-rank-polymorphism.md`
  "Support Level" understated support as reference-interpreter only; native
  rank-2 lambda-arg parity is in fact tested
  (`compiled_rank2_lambda_arg_matches_oracle`). Corrected.
