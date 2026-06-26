# Zutai Archive

This document tracks implemented status, validation notes, archived decisions,
and completed milestones. Open milestones and TBD items live in
[`TBD.md`](TBD.md). Language design still comes from `docs/spec/v0/` for v0 and
`docs/spec/v1/` for deferred v1 features.

## Compilation pipeline

```text
Source → HIR → THIR → TLC
                        ↓  TLC→DC: tree-to-graph, sharing, recursion explicit
                   Dataflow Core
                        ↓  DC→ANF: topo-sort SCCs, name every node, let/letrec
                       ANF
                        ↓  ANF→SSA: basic blocks, phi-nodes
                       SSA
                        ↓  SSA→LLVM
                    LLVM IR
```

- **THIR** is the error-tolerant, source-preserving typed IR and the output of
  `check`.
- **TLC** is produced only after successful type checking. It has explicit
  polymorphism (`TyLam`/`TyApp`) and resolved inference variables.
- **Dataflow Core → ANF → SSA → LLVM** is the production AOT pipeline. Laziness
  and sharing are structural in Dataflow Core, not runtime thunks.
- **`zutai-eval`** is the reference semantics oracle. The default `run`/`repl`
  path is TLC-first for executable value programs; THIR remains the regression
  oracle and runtime `Type`/reflection boundary.

Design details: [`docs/tlc-core.md`](tlc-core.md),
[`docs/dataflow-core.md`](dataflow-core.md), [`docs/anf.md`](anf.md), and
[`docs/runtime-abi.md`](runtime-abi.md).

## Current baseline

_Last updated: 2026-06-23 (language specs, Unicode XID, evaluator/backend hardening),
2026-06-24 (Phase A: `.zt`/`.zti` native module-import lowering), and
2026-06-26 (general-mode `;`-terminator / container-glyph grammar; docs migrated;
`import` unified as an expression — dedicated `name :: import` decl form removed)._

- General-mode (`.zt`) surface grammar now uses `;` as the universal
  terminator/separator: every value-like top-level declaration ends in `;`, and a
  trailing `;` makes an expression a `()` statement. The container glyph picks the
  shape — `{ … }` is a parallel record (`name = value;`) or list (bare `value;`),
  and `[ … ]` is a serial do-block (local bindings + tail). The scope picks the
  binding operator — top-level `::=` / `:: T =`, local (inside `[ … ]`) `:=` / `: T =`.
  Empty record `{}`, empty list `{;}`, empty do-block `[]`. Immediate mode `.zti`
  is unchanged (arrays stay `[ … ]`). v0 spec docs, the language manual, and stdlib
  notes were migrated to this grammar; the `v0_spec` doc-fence acceptance test was
  updated to the new accepting set (decl-only `.zt` snippets now form complete
  programs that evaluate to `()`).
- Immediate mode parses `.zti` data through selectable parser backends
  (standard + SIMD/NEON).
- General mode parses `.zt`, lowers to HIR, type-checks through THIR, and
  elaborates complete programs to TLC.
- THIR covers v0 plus implemented v1/v2/v3-adjacent semantics:
  row-polymorphic records/unions, `select`, constraints/witnesses, `derive`,
  method-level type params, higher-kinded constraints, algebraic-effect typing,
  higher-rank annotation checking, predicative inference, guarded recursive type
  aliases, stream-backed generator sugar, and standard host capability/effect-row
  checking.
- TLC covers row variables, effect rows, explicit dictionary passing, witnessed
  operator lowering, source effect markers, higher-rank `ForAll`/`TyLam`/`TyApp`
  elaboration, CPS elaboration for handled effects, equirecursive alias identity,
  runtime `io.print` lowering through ordinary TLC function values, and residual
  host-effect grant gating before Dataflow Core.
  Constraint-attached derive recipes are stored through Syntax/HIR/THIR and
  drive specialized TLC Show/Ord dictionary synthesis; `witness C @T` reflects
  resolvable dictionaries using the same concrete/conditional lookup as implicit
  method dispatch. Runtime type reflection includes `fields`, `variants`, and
  `schema` views.
- THIR and TLC carry internal universe levels for surface `Type`. Explicit level
  syntax is still unsupported; level-polymorphic type constructors default to
  the lowest consistent universe and erase before runtime/backend lowering.
- Dataflow Core, ANF, SSA, and LLVM IR text emission exist and are test-covered.
  Record/tuple access is slot-indexed; union construction now uses dense
  per-union tags; ambient `io.print` lowers to a runtime `HostPrint` path;
  granted v2 host operations lower to explicit `HostOp` nodes through
  Dataflow/ANF/SSA/LLVM/runtime; recursive and generic recursive aliases lower
  to finite cyclic `DfTyId` graphs; codegen emits static descriptors for
  `zutai.show`; `@main` renders through the type-directed runtime display path
  and rejects function / `Type` results. **`.zti` data imports and `.zt` pure
  value/function imports compile natively** via one-arena Dataflow Core merge
  (Phase A): imported modules are lowered into the same graph under a `$dep{idx}$`
  namespace prefix; the root references the dep's module-value global
  (`$dep{idx}$$value`). Modules that export typeclass witnesses are rejected before
  DC by the witness gate (cross-module witness dispatch is still interpreter-only).
- `compile --emit=llvm|obj|bin` selects LLVM text, object, or native binary
  output. Object/binary modes invoke `llc`/`clang`, link `libzutai_rt`, emit
  actionable diagnostics when the host toolchain is absent, and produce
  PIE-capable Linux binaries without `-no-pie`.
- `zutai-eval` has both the THIR oracle and TLC evaluator. Differential coverage
  includes constraints, optionals, `.zti` imports, `.zt` imports, imported
  functions, transitive imports, imported witness dictionaries, record update,
  config overlay, effects, reflection/type-value boundaries, polymorphic
  curried helpers, repeated nested destructures, and name-sorted record display.
- `print` remains a prelude compatibility binding, but its type is now
  `Text -> Text ! { io.print : Text -> Text }`. TLC lowers the builtin value to
  a runtime-dispatching function; source handlers can intercept `io.print`, and
  the host `run`, `compile`, and `dataflow` paths dispatch ambient `io.print` at
  runtime instead of replaying compile-time captured output.
- `compile` and `dataflow` no longer fold effectful entry programs through the
  evaluator before Dataflow Core. Residual non-`io.print` effect markers and
  unsupported effect rows stay gated by `residual_effect_reason` /
  `zutai_dataflow::try_lower_tlc`; `io.print`-only function rows lower through
  the runtime `HostPrint` path.
- `compile` and `dataflow` still fold renderable compile-time reflection
  programs through the THIR type-value evaluator before Dataflow Core.
  Reflection combined with effectful code remains rejected so AOT reflection does
  not consume host effects at compile time.
- Supported full config-overlay calls lower before Dataflow Core: patch-first
  `overlay`/`overlayDeep` applications with record-literal patch values become
  ordinary record updates, and required nested records merge recursively.
- Unsupported residual overlay forms, optional nested-record deep overlays,
  reflection combined with effectful code, non-`io.print` residual effects,
  unsupported effect rows, function entries, and `Type` entries still reject
  before DC.

## Validation notes

- Optional value syntax remains `T? = Optional T` with `#none` / `#some (v)`.
  Optional field access preserves physical presence as `Maybe T` with `#absent`
  / `#present (v)`, so `field? : T?` yields `Maybe (Optional T)`. `?.` works on
  both `Optional` and `Maybe`; `??` unwraps exactly one layer.
- v0 docs use parser-accepted typed bindings (`name :: Type = value`) and
  semicolon-terminated record/tagged patterns. Fixtures pin stale syntax
  rejections.
- `Int??` lexes as `Int` + `??`, not a double optional. Write `(Int?)?` for a
  nested optional.
- CLI native binary coverage includes primitive, record, tuple, union, text,
  atom, and posit entry values; the Linux PIE matrix is verified with
  `llc -relocation-model=pic` and `clang -pie`.

## Open work pointer

Open milestones and unresolved work live in [`TBD.md`](TBD.md). When an item
finishes, move a short status summary into "Completed milestones, newest first"
and keep any unresolved follow-up in `TBD.md`.

## Archived backlog decisions

These closed stabilization items stay here so old risk decisions remain visible.
New unresolved work should become an open milestone/TBD item in `TBD.md`.

- [x] **Compiler entry-type gate cleanup** — CLI `compile` and `dataflow`
  reject final runtime `Type` values before TLC→DC/LLVM lowering, including raw
  `type Int` entries and alias-value entries such as `MyInt :: type Int; MyInt`.
- [x] **v0 spec conformance sweep** — code fences from `docs/spec/v0/` are
  extracted and routed through `check`/`run` for `.zt` survivors and the immediate
  parser for `.zti` survivors; stable survivors are promoted to acceptance tests.
- [x] **Diagnostic polish** — record-vs-record type mismatches render source-like
  record shapes, including optional fields and row tails; row-tail spread
  overlaps report the spread source and existing/incoming shapes.
- [x] **TLC-first evaluator cutover** — default evaluation runs through TLC for
  executable value programs; THIR remains the explicit regression oracle and
  runtime `Type`/reflection boundary.

## Completed milestones, newest first

### `import` unified as an expression ✅

_Completed 2026-06-26. `import <source>` is now an expression atom; the dedicated
`name :: import source` declaration form was **removed**. A plain import binding is
the ordinary inferred binding `name ::= import "path"`, and module members
destructure straight off the import in one binding:
`{ map; fold; } ::= import stdlib.stream;`. The `import` source remains a literal
(string or dotted path), never a runtime value, so resolution stays fully static
and pure — `import` in expression position cannot create runtime-selected loading._

Mechanics: added `Expr::Import` (parser atom in
`crates/general/syntax/src/parser/expr/atom.rs`, lowering to the existing
`HirExprKind::Import`/`ThirExprKind::Import`) and dropped `Decl::Import` and the
`BindingKind::TopImport` kind. THIR identifies an import binding structurally (a
value decl whose value is an `Import` expr) instead of by binding kind, in
`predeclare_import_decls` / `lower_decl` / `lower_type_apply`. The import resolver
was already expr-arena-based, so discovery was unchanged. Support level unchanged
(reference-interpreter; module imports remain gated out of the native backend).

### Applied imported type constructors ✅

_Completed 2026-06-26. Lifts the last V3-G6 import residual: a parametric
imported type constructor can now be **applied** in an annotation
(`x :: s.Stream Int`) for arbitrary user modules, not just the embedded stdlib.
Reference-interpreter support level — module imports remain gated out of the
native backend (unchanged), so these run in `zutai-eval`'s THIR oracle._

- **Binder-preserving export.** `export_type_value` (`thir/src/export.rs`)
  preserves a parametric type alias's binder as a new descriptor variant
  `ImportedType::TypeCon { params, body }`; a saturated application of a
  parametric alias exports as `ImportedType::ConApply { ctor, args }` (a *bounded*
  reference, never unfolded), so a recursive body (`tail: Stream A`) terminates
  and a sibling combinator signature (`empty :: <A> Stream A`) references the same
  constructor. `enrich_with_type_denotations` (`semantic/src/import.rs`) builds the
  `TypeCon` for `Type`-valued fields. Non-parametric aliases (`serverLib.Server`)
  export unchanged.
- **Import-side rebuild as a local alias.** `BindingId`s are HIR-owned and cannot
  be minted in THIR, so the importer allocates *synthetic* bindings
  (`alloc_synthetic_binding`, name/kind reads routed through
  `binding_name`/`binding_kind`) for the constructor and one per parameter,
  registers `alias_params`/`aliases` and an `import_type_constructors` lookup, and
  materializes a `ThirDeclKind::TypeAlias` decl appended to `ThirFile::decls`.
  Interning is two-pass (declare all constructors, then intern bodies) so
  sibling/recursive `ConApply`s resolve. This reuses the existing `resolve_alias`,
  capture-avoiding `instantiate_type_vars`, and equirecursive matcher verbatim —
  the lowest-soundness-risk path. TLC and both evaluators then treat the imported
  constructor as an ordinary local parametric alias.
- **Application + annotation.** `lower_type_apply` resolves an `Access` head
  (`s.Stream`) to its synthetic constructor and reuses the named-alias path
  (saturated → `AliasApply`, partial → curried `Apply`). A bare `s.Stream` (no
  args) is a zero-arity `TypeConstructorArityMismatch`, matching local generics.
- **v1 scope / refusals.** Higher-kinded constructor parameters are refused at
  export (detected by use, since HIR drops an alias param's kind annotation). A
  `ConApply` to a constructor the module does not export degrades to an
  unconstrained position (a safe opaque pass-through, as un-exportable types
  already do) rather than a hard error. TLC *evaluation* of any module that
  exports a type value is still refused by the pre-existing runtime-type-value
  gate (`has_runtime_type_values`) — type elaboration of `s.Stream Int` itself
  succeeds through TLC; only the runtime walker is gated.
- **Tests.** Export units (`thir/.../tests/export_types.rs`), eval round-trip +
  multiple-instantiation + ambient-vs-imported parity + refusal tests
  (`eval/src/tests/imports.rs`, fixtures `stream_module.zt`/`hkt_module.zt`).

### Import ergonomics: embedded stdlib, type export, destructuring ✅

_Completed 2026-06-26. Closes the three V3-G6 import-ergonomics follow-ups
(see `docs/TBD.md`). Reference-interpreter support level — module imports remain
gated out of the native backend (unchanged), so these run in `zutai-eval`._

- **Embedded stdlib (`import stdlib.stream`).** A `stdlib.<name>` dotted import
  resolves to in-binary source, addressed through a registry (`stdlib_source`,
  seeded with `stream` = `zutai_hir::STREAM_MODULE_SRC`, one source of truth with
  the ambient prelude). Resolution uses a synthetic cache key (`<stdlib>/<name>.zt`)
  so cycle detection and the analysis cache apply without touching the filesystem
  or the path-relative subtree-confinement check (`semantic/src/import.rs`). Unknown
  names give a precise `UnknownStdlibModule` diagnostic. `resolve_zt` was refactored
  into `analyze_zt` + `register_zt_module` shared by the filesystem and embedded paths.
- **`Stream`/`Step` type export.** Added to `stream.zt`'s export record, so both are
  selectable/destructurable record fields. (Applying a parametric imported type
  constructor in an annotation — `s.Stream Int` — was unsupported at the time of
  this milestone; it landed shortly after — see "Applied imported type
  constructors" above.)
- **Selective import via destructuring binding.** New `Decl::Destructure`
  (`{ a; b; } ::= rec;`) reuses the select-field list syntax on the left of `::=`.
  It lowers in HIR to a synthetic single-eval receiver binding plus one
  `field ::= receiver.field` value decl per name (`lower_destructure_decl`), so the
  selected members are in scope unqualified. The RHS is any record expression
  (composes with `>>=` and a prior import). Non-fields are type errors; collisions
  are duplicate-binding errors; a `{ … }` record final-expression (no `::=`) is
  unaffected.

### V3-G2 residual: List interop (`toList`/`fromList`/`takeList`) ✅

_Completed 2026-06-26. Ships the stream↔list interop combinators — the last V3-G2
residual — closing V3-G2. Native compile + interpreter (THIR & TLC) oracle parity._

- **The gap.** The builtin `List` has no source-level head/tail ops: the `Pattern`
  enum has no list/cons pattern, and the runtime had `list_cons`/`list_nil`
  constructors but no destructor. So `.zt` source could neither build a `List` of
  dynamic length nor take one apart — which is why `toList`/`fromList` could not be
  written in `.zt`. (The interpreter represents `List` as a flat `Rc<[Thunk]>`; the
  native backend as a `TAG_NIL`/`TAG_CONS` cons-cell list.)
- **Design — scalar bridge primitives + `.zt` combinators.** Rather than a single
  list-destructor node (which would force a branching CFG inside the per-instruction
  codegen to build a result variant), five *scalar* bridge primitives map 1:1 to
  pure `i64→i64` runtime calls, and the `if`/`match` branching lives in the shared
  `stream.zt` source: `listEmpty :: <A> Unit -> List A` (lowers to an empty list
  literal), `listCons`, `listIsNil`, `listHead`, `listTail`. The combinators are
  ordinary `.zt` recursion over them (`toList` via `match s ()`; `fromList` via
  `if listIsNil xs then #nil else #cons {…}`), riding the proven native stream path.
  `take` stays `Stream → Stream`; `takeList = toList ∘ take` is the named
  `take → List` form. The primitives are globally-visible builtin values
  (`BUILTIN_VALUE_NAMES`), documented as internal bridge ops.
- **Pipeline.** THIR builtin types (`thir/.../builtins.rs`); interpreter `BuiltinFn`
  variants on both walkers (`eval/binary.rs`, `eval_tlc/effects.rs`, laziness
  preserved — `listCons` keeps its head thunk unforced); native lowering via a new
  `DfNodeKind::ListPrim { op, args }` intercepted in the DC lowerer's saturated-App
  spine, threaded through validate/ANF/SSA/codegen; runtime ABI gains read-only
  `zutai.list_is_nil`/`list_head`/`list_tail`. No backend-gate change (the new
  primitives are not in the reflection/overlay reject lists). GC/codata invariants
  hold: standard cons cells + a standard codata closure, no new heap shape, no
  write barrier.
- **Tests.** Native == oracle: `compile_prelude_stream_tolist_matches_oracle`,
  `_fromlist_`, `_takelist_infinite_`, `_tolist_empty_`, and
  `compile_zt_imported_stream_list_interop_matches_oracle` (qualified `s.toList`/
  `s.fromList`). Interpreter unit + THIR↔TLC differential battery cover finite
  `toList`, `fromList` round-trip, empty edges, and bounded `takeList` over an
  infinite generator. 1647 workspace tests pass.

### BindingRef instantiation site — polymorphic values (ships `empty`) ✅

_Completed 2026-06-25. Makes a `BindingRef` a first-class instantiation site so a
polymorphic *value* (not just an applied function) instantiates its `<A>` per use.
This unblocks any rank-1 polymorphic value referenced outside callee position; the
motivating case is the stream combinator `empty :: <A> Stream A`._

- **Root cause.** Explicit `<A>` annotations lower to rigid `TypeVar`s
  (`thir/.../decl.rs`). The only per-use freshening was at an application's callee
  (`thir/.../expr/call.rs::lower_apply_expr`). A polymorphic value used as an
  argument / returned / bound is never a callee, so its `<A>` stayed one rigid
  variable shared program-wide — fine while unconstrained (`take 3 empty`), but it
  failed once a consumer pinned `A` (`cons 5 empty`).
- **Producer.** `ThirExprKind::BindingRef` becomes a struct variant carrying
  `instantiation` / `forall_instantiation` (mirroring `Apply`). `lower_binding_ref`
  freshens the binding's free `TypeVar`s to InferVars and records them — but only
  for **top-level** bindings (a `Param`/`Local`'s type vars are inherited and
  rigid) and only when the value is **not** checked against a higher-rank `ForAll`
  parameter (where it must stay polymorphic). `lower_apply_expr` forwards a
  rank-1 `BindingRef` callee's recorded instantiation onto the `Apply` so callee
  dispatch is unchanged.
- **Consumer.** The TLC `Apply` handler's constraint-method / explicit-params
  dispatch is extracted into a shared `lower_instantiated_callee`, reused by the
  standalone `BindingRef` path so a polymorphic value emits the same `TyApp` +
  dictionary `App` prefix (witness threading intact). No eval change was needed —
  TLC dictionary-passing carries it.
- **Guardrail.** The 13 witness/constraint fixtures (conditional/imported witnesses,
  indirect bounded calls, higher-rank `apply`/`show`) that a naive blanket-freshen
  regressed all stay green; the `Param`/`Local` and `ForAll` guards are what keep
  them so.
- **Tests.** `compile_prelude_stream_empty_matches_oracle`,
  `compile_prelude_stream_empty_polymorphic_matches_oracle` (`empty` at `Stream
  Bool` and `Stream Int` in one program), and
  `compile_zt_imported_stream_empty_matches_oracle` — all native == interpreter
  oracle. `empty` added to `stream.zt` (ambient + export).

### V3-G2 residual: `unfold` combinator ✅

_Completed 2026-06-25. Ships `unfold` — the canonical codata producer (step
function + seed) — as both an ambient prelude combinator and an importable
`stream.zt` export, closing the more valuable half of the deferred
`empty`/`unfold` residual. `empty` stays deferred (precise diagnosis below)._

- **Combinator + `Step` type.** `crates/general/hir/src/lower/prelude/stream.zt`
  gains `Step :: <S, A> type { #done; #yield : { item : A; next : S; }; }` and
  `unfold :: <S, A> (S -> Step S A) -> S -> Stream A = f s _ => match f s { … }`,
  demand-driven (the trailing `_ =>` thunk defers stepping until forced). The
  export record adds `unfold`. No ABI change; the new `Step` union crosses the
  import boundary structurally inside `unfold`'s signature, the same way `Stream`
  does (no further validator change needed).
- **Why `Step`, not `Optional`.** The documented signature used
  `Optional { item; next }`, but the builtin `Optional`'s `#some` payload is
  represented as a positional 1-tuple (`thir/.../expr/tagged.rs`), which does not
  compose with a record payload at the surface (`expected record, found tuple`).
  A plain structural `Step` union sidesteps it with no type-system work — the
  documented "type-inference edge case" for `unfold`.
- **`empty` (then) deferred — now shipped.** `empty :: <A> Stream A` was deferred
  here pending a type-system fix; it landed separately once `BindingRef` became a
  first-class instantiation site (see "BindingRef instantiation site" below). The
  earlier diagnosis blamed generalization narrowing the union; the true cause was
  that a `<A>` reference outside callee position never freshened its type variable.
- **Tests.** `compile_prelude_stream_unfold_matches_oracle` (ambient: an infinite
  `unfold` gated by `take`, fold = 10) and
  `compile_zt_imported_stream_unfold_matches_oracle` (`s.unfold` through the import
  boundary with a local `Step` alias on the step annotation), both asserting
  native == interpreter oracle.

### V3-G6: Importable `stream.zt` module ✅

_Completed 2026-06-25. Closes the last structural V3-G2 residual: the codata
`Stream` combinators are now a real importable `.zt` module, not only an embedded
ambient string. Unblocked by cross-module polymorphism (XM-1…3). Single source of
truth, both surfaces preserved; path-relative resolution (stdlib-root resolution
stays deferred)._

- **Single-source file.** `crates/general/hir/src/lower/prelude/stream.zt` holds
  the `Stream` type plus the eight combinators (`cons`, `singleton`, `map`,
  `filter`, `take`, `drop`, `fold`, `uncons`) and ends in a record exporting all
  eight. The HIR lowerer's ambient prelude now `include_str!`s this file (exposed
  as `zutai_hir::STREAM_MODULE_SRC`) instead of an inline literal; the ambient path
  reads only the *declarations* and ignores the final record, so ambient behavior
  is byte-for-byte unchanged (the fallback still yields to user/constraint names).
  The import path uses the final record as the module's exported value, so
  `s ::= import "stream.zt"` gives `s.map`, `s.fold`, … qualified.
- **Backend fix — cross-module global-ref compat.** The recursive `Stream` codata
  type cannot be reconstructed structurally through the finite `ImportedType`
  boundary, so the import abstracts it to a fresh `TyVar` at the recursion horizon,
  while the dependency's real exported value is fully structural. The Dataflow Core
  structural validator's `GlobalRef` check (`validate/refs.rs::is_instantiation_of`)
  was made **symmetric**: an abstract leaf (`TyVar`/`Opaque`/`Error`/`Type`) on the
  *use-site* side now wildcards any definition shape, mirroring the existing
  def-side `TyVar` wildcard. Sound under the untagged-i64 ABI (D-0002): an opaque
  use-site never inspects the value's structure, and a one-word value is
  layout-identical to the concrete definition it stands in for. Non-abstract
  structure (record/union/tuple shape, arity, field names) is still matched exactly,
  so genuine mismatches stay rejected.
- **Tests.** `compile_zt_imported_stream_module_matches_oracle` builds a finite
  stream and runs `filter`/`map`/`take`/`fold` through the import boundary,
  asserting native == interpreter oracle == 12 (the recursive-`Stream`-across-the-
  boundary case that drove the compat fix). `ambient_stream_prelude_matches_imported_module`
  confirms the ambient surface still resolves the same combinators. Workspace at
  1633 tests.
- **Deferred (open questions).** Stdlib-root resolution (a shared install location
  and the dotted `import stdlib.stream` form, with an allowance past the
  subtree-confinement check) and selective/open-import binding (unqualified names
  after import) stay out of scope — see `TBD.md`.

### GC default-on (D-0008 reversal) ✅

_Completed 2026-06-25. The conservative mark-sweep collector (Phase 34), shipped
opt-in, is now **on by default** wherever the conservative stack scan is wired up
(macOS, Linux). This reverses the original D-0008 leak-by-default commitment;
`ZUTAI_GC=0` (or `false`/`no`/`off`) opts back out, and platforms with no
stack-bounds path stay leak-by-default regardless. Supersedes the V3-G5 "keep
opt-in" decision below._

- **Change.** `gc_mode()` in `crates/general/runtime/src/lib.rs` now enables the
  collector unless explicitly opted out: `enabled = (stress || !env_falsy("ZUTAI_GC"))
  && stack_base().is_some()`. A new `env_falsy` helper recognizes `0`/`false`/`no`/
  `off`; `ZUTAI_GC_STRESS` still forces collection and overrides the opt-out. No
  ABI change (D-0002 untagged `i64` not reopened); the arena, cap, and conservative
  scan are unchanged — only the default gate flipped.
- **Effect.** A bounded-live / unbounded-allocation program now holds steady-state
  memory flat with no env var: the `n = 800k` accumulator and the unbounded stream
  pipeline both stay at 1 MiB peak committed by default, where `ZUTAI_GC=0`
  restores the ~13 MiB / ~269 MiB leak. Output is unchanged on both paths.
- **Tests.** `compile_emit_bin_gc_is_default_on_with_opt_out` (no-env run collects
  and stays small; `ZUTAI_GC=0` leaks; both correct). The leak-baseline tests
  (`compile_emit_bin_accumulator_garbage_dominates_gc_gate` via `run_with_heap_stats`,
  and `compile_emit_bin_heap_stress_aborts_over_cap`) now pin `ZUTAI_GC=0`
  explicitly so they still measure the leak baseline / cap-abort guard.
- **Still future.** The precise/moving (Cheney) endgame and a lazy backend remain
  deferred (`TBD.md` "GC residual"); strict-plus-TCO stays committed.

### V3-G5: GC keeps unbounded stream pipelines bounded ✅

_Completed 2026-06-25. Acceptance met: a long-running `unfold`/stream pipeline
holds steady-state RSS flat under collection while producing correct output. No
GC or compiler code changed — the Phase 34 conservative collector already keeps
demand-driven streams bounded; this milestone characterizes that for the unbounded
stream workload (GC gate condition (a), enabled by V3-G1) and records the
default-on policy decision. **Track 1 (generators & streams) is complete.**_

- **Measurement.** `fold (+) 0 (take n (countFrom 1))` over an infinite recursive
  generator (V3-G3) has an O(1) live set (one in-flight cell + the fold
  accumulator) against O(n) allocation. Under `ZUTAI_GC`, peak committed stays
  **flat at 1 MiB** for `n = 100k` and `n = 800k` (8×), where leak-by-default
  grows ~linearly (34 MiB → 269 MiB); the collector reclaims ~268 MiB across 269
  cycles. Output is correct on both paths, and the pipeline stays correct under
  `ZUTAI_GC_STRESS` (collect before every allocation), proving the conservative
  root/heap scan retains the in-flight cell and accumulator.
- **Policy decision (default-on).** As landed, G5 kept the collector **opt-in**
  (the committed D-0008 leak-by-default), with streams as first-class
  beneficiaries. **Superseded later the same day:** the default was flipped to
  **GC on by default** with a `ZUTAI_GC=0` opt-out — see "GC default-on (D-0008
  reversal)" above. (Auto-enabling GC for "stream programs" specifically was *not*
  taken — a global default-on with opt-out is simpler than a fragile static
  "is-a-stream-program" heuristic.)
- **Tests.** `compile_emit_bin_gc_keeps_stream_footprint_flat` (flat peak at N vs
  8N, correct output, reclaimed ≫ peak) and
  `compile_emit_bin_gc_stress_preserves_stream_output` (soundness under stress),
  alongside the Phase 34 accumulator GC tests.

### V3-G4: Effectful generators (reference-interpreter) ✅

_Completed 2026-06-25. An effectful generator runs under a granting handler on
the interpreter; native lowering of its (non-`io.print`) effects stays refused by
the committed strict-AOT-rejects boundary (Phase 35). Support level: **check +
reference-interpreter**. No new effectful-codata type or compiler feature was
added — the existing effect machinery already carries it; this milestone
characterizes and locks in the support boundary with tests and docs._

- **Mechanism.** A `yield perform op …` defers the operation into a *lazy cell
  field*, so the effect is charged to whoever **forces** that field, not to the
  constructor. The supported idiom: the producer performs in its cells
  (`stream { yield perform tick (); }`), a consumer that *strictly* forces each
  element declares the effect in its own row
  (`sumEff :: (Unit -> Cell) -> Int ! { tick … }`), and the whole consumption runs
  under a handler (`handle (sumEff gen) with { tick = \_. resume 5; }` → `10`).
- **Boundaries (each refused, never miscompiled).** No handler / pure consumer →
  the effect escapes the ambient row (type error). Pure `Stream A` annotation of
  an effectful producer → rejected (the deferred effect cannot satisfy the pure
  thunk the alias demands; effectful streams are not the pure `Stream` alias and
  do not interoperate with the pure prelude combinators). Lazy escape (returning
  an unforced effectful head) → runtime "unhandled effect" refusal, consistent
  with demand-driven ordering. Native → residual-effect gate refuses any
  non-`io.print` effectful generator.
- **Consequence.** Resource host effects (`fs.read`, networking, clocks,
  randomness) reach only the interpreter behind an explicit grant; they have no
  native path. Cancellation/finalization and resource lifetime remain open.
- **Tests.** `effectful_generator_runs_under_granted_handler`,
  `effectful_generator_without_a_handler_is_rejected`,
  `effectful_stream_generator_against_pure_stream_alias_is_rejected` (eval);
  `compile_effectful_generator_stays_gated` (CLI, native refusal).

### V3-G3: Richer `yield` ✅

_Completed 2026-06-25. `yield` may now appear under conditionals and recursion
inside a `stream { … }` generator, settling the open question: richer `yield` is
**statement syntax desugared by continuation-passing** onto the V3-G1 codata
cell, not handler sugar. A recursive/loop generator type-checks and evaluates —
interpreter and native — to the same `Stream` the equivalent `unfold` produces._

- **Surface (parser + AST).** A generator block is now a statement list
  (`ast::GenStmt`): `yield e;`, `yield from e;` (delegating yield), and
  `if cond then { … } [else { … }]` (conditional yield, branches are
  statement blocks). `stream` stays contextual — a generator now starts on a
  leading `yield` *or* a guarding `if`; parenthesise (`stream ({ if … })`) to
  force application of a block whose head is `if`.
- **Desugar (HIR).** `lower_gen_stmts` lowers a block against its *continuation*
  (the stream that follows): `yield e` conses a `\_. #cons { head = e; tail = … }`
  thunk; a conditional yields per branch, sharing the continuation (bound to a
  fresh synthetic local when non-tail so both branches reference it without
  aliasing a node); a **tail** `yield from s` is the stream `s` itself. The
  codata cell has no shared append, so a **non-tail** `yield from` is refused with
  a new `NonTailYieldFrom` HIR diagnostic — never miscompiled.
- **Arity (THIR).** The clause-arity check was relaxed: a clause may bind a
  *prefix* of the flattened parameters and return the residual function as its
  body (ordinary currying), so a generator function (`range lo hi = stream { … }`)
  supplies the codata `Unit` from its desugared thunk rather than spelling it.
  The bound arity must be **uniform** across clauses (every later stage keys on
  `clauses[0]`'s arity) and may not exceed the signature; a mixed-arity
  definition is refused. Exhaustiveness is still checked over the bound prefix.
- **Tests.** Parser tests (conditional + delegating yield, else-less `if`); eval
  tests (`recursive_generator_matches_unfold_semantics`,
  `conditional_yield_emits_only_on_the_true_branch`,
  `non_tail_yield_from_is_refused`); CLI oracle-parity tests
  (`compile_g3_recursive_generator_matches_oracle`,
  `compile_g3_conditional_infinite_generator_matches_oracle`); and a THIR
  regression for non-uniform arity (`non_uniform_clause_arity_is_reported`).

### Cross-module polymorphism — multi-type (XM-1…3) ✅

_Completed 2026-06-25. An imported polymorphic value (a module exporting
`id :: <A> A -> A`, or a record of generic functions — the importable-stdlib
shape) can now be used at **multiple concrete types in one program**: it
type-checks and lowers natively, matching the interpreter. Builds on the
single-type validator relaxation below._

- **Boundary scheme (XM-1).** `ImportedType::TyVar(u32)` represents an exported
  type parameter (`crates/general/thir/src/import.rs`).
- **Generalize on export (XM-2).** `export.rs` turns a free `TypeVar`/`InferVar`
  in an exported value's type into a `TyVar` (the two id spaces kept disjoint via
  a high-bit tag); `ForAll` exports its body. Previously these flattened to
  `Unknown`.
- **Instantiate on import (XM-3).** Interning maps each exported `TyVar` id to one
  fresh inference variable (cached, so `∀A. A -> A` stays `?a -> ?a`, preserving
  `A = A`). The import binding is generalized in the main decl pass over **only**
  those exported-parameter vars (recorded in `import_poly_candidates`), so each
  reference instantiates fresh — while `Unknown` (un-exportable) positions are
  deliberately excluded and stay monomorphic-by-use. Native lowering reuses the
  single-type validator relaxation (no further Dataflow work).
- **Acceptance.** Native==interpreter oracle for an imported `id` used at Bool and
  Int (`compile_zt_imported_generic_multitype_matches_oracle`), and a record
  `apply` used at `Int->Int` and `Bool->Bool`
  (`compile_zt_imported_generic_record_multitype_matches_oracle`). A value of an
  un-exportable type used at two types is **cleanly rejected**, never made
  polymorphic (`compile_zt_imported_unexportable_value_stays_monomorphic`).
  Reviewer (two rounds) found and fixed a round-1 P0 (generalizing `Unknown`-derived
  vars), with no residual soundness issue from the candidate-based fix. 1619
  workspace tests pass.
- **Follow-up: pre-existing ICE fixed.** An un-exportable import value passed only
  to a generic that never pins its type (e.g. `ign :: <A> A -> Int = _ => 0;
  ign dep`) used to leak an unconstrained inference variable into Dataflow Core
  and ICE (present on the prior baseline too). Fixed by also skipping the Dataflow
  `GlobalRef` structural check when the *use-site* type is opaque
  (`is_opaque_shape_type(node.ty)`): a `GlobalRef` lowers to a symbolic by-name
  reference and any concrete access is a separate, separately-checked node, so
  under untagged-i64 it is a machine-safe pass-through — it now compiles and
  matches the interpreter
  (`compile_zt_imported_unexportable_value_through_generic_matches_oracle`).
  Reviewed sound (no P0/P1).

### Cross-module polymorphism (single-type) ✅

_Completed 2026-06-25. A module exporting a polymorphic value (`id :: <A> A -> A`,
or a record of generic combinators — the importable-stdlib shape) used at a
single concrete type per program now compiles natively and matches the
interpreter. Previously this ICEd in Dataflow._

- **Root cause.** The import boundary erases polymorphism: a polymorphic export
  is lowered with the dependency's free-`TyVar` type (`Fun(TyVar, TyVar)`) while
  the importer's use site is concrete (`Fun(Int, Int)`), so the cross-module
  `GlobalRef` failed `validate_structural` with a `TypeMismatch` → `internal
  compiler error` panic.
- **Fix (ABI-justified, not the full boundary rework).** Under untagged-i64
  (D-0002) a parametric value is compiled exactly once and is bit-identical across
  all instantiations (parametricity), so the dependency `GlobalRef` points at the
  same machine code regardless of the use type. `is_instantiation_of`
  (`dataflow/src/validate/refs.rs`) accepts a use type that is a sound structural
  instantiation of the generic definition (a definition-side `TyVar` matches any
  use subterm; every other constructor must match exactly, so record-vs-tuple and
  arity/tag/field mismatches stay rejected). Wired into the `GlobalRef` check in
  `validate/compat.rs`.
- **Acceptance.** Native==interpreter oracle for a bare imported generic function
  (`compile_zt_imported_generic_fn_matches_oracle`, = 42) and an imported record
  of generic functions (`compile_zt_imported_generic_record_matches_oracle`, = 42);
  multi-type cross-module use is a **clean rejection, never an ICE**
  (`compile_zt_imported_generic_multitype_rejected_cleanly`). Reviewer found no
  P0/P1/P2 soundness issues. 1617 workspace tests pass.
- **Residual (deferred).** Multi-type use — one program using an imported generic
  at several types — still needs the import-boundary scheme rework (XM-1…3 in
  `docs/TBD.md`), because the boundary currently monomorphizes by first use. The
  native lowering is already done, so only the THIR type-side remains.

### V3-G2: Stdlib `Stream` API via prelude ✅

_Completed 2026-06-25. Second phase of the V3 generator/stream spine. Ships the
core `Stream` combinators as **ambient prelude functions** (no import), the
native-complete packaging — the originally-chosen importable-module packaging is
blocked by a backend gap (see `docs/TBD.md` "Cross-module polymorphism")._

- **API in the prelude** (`crates/general/hir/src/lower/mod.rs` `PRELUDE_SRC`):
  `cons`, `singleton`, `map`, `filter`, `take`, `drop`, `fold`, `uncons` —
  demand-driven `.zt` over the codata `Stream` cell, alongside the `Stream` type.
- **Prelude is a fallback.** Each prelude declaration is defined only when its
  name is not already owned by a user binding or constraint method (all share the
  top scope), so e.g. a `Functor` method named `map` wins with no collision; and
  a declaration is lowered into a module only when that module references it
  (reachability over type *and* value `BindingRef`s), keeping unused builtins out
  of THIR/TLC/codegen.
- **Acceptance.** Native==interpreter oracle for a `map`/`filter`/`take`/`drop`/
  `fold` pipeline (`compile_prelude_stream_pipeline_matches_oracle`, = 120) and
  `cons`/`singleton`/`uncons` (`compile_prelude_stream_cons_uncons_matches_oracle`,
  = 99); the prelude-fallback property is tested against the higher-kinded
  `Functor` check (`prelude_stream_name_yields_to_user_definition`). 1614
  workspace tests pass.
- **Deferred (status as of original V3-G2).** `empty` and `unfold` hit
  type-inference edge cases (a polymorphic nullary value; a self-referential
  producer union); the `List`-interop subset (`toList`/`fromList`/`take -> List`)
  needs source-level list construction the language lacks; the importable-module
  packaging waits on cross-module polymorphism. _Later closed:_ importable module
  (V3-G6) and `unfold` (V3-G2 residual: `unfold` combinator, above — shipped via a
  structural `Step` union). `empty` remains deferred.

### V3-G1: Codata `Stream` representation ✅

_Completed 2026-06-25. First phase of the V3 generator/stream spine
(`docs/v3_spec/02-roadmap.md`). Turns the builtin `Stream A` from a strict
`List A` alias into demand-driven **codata** — `Stream A ≡ Unit -> StreamCell A`,
`StreamCell A ≡ { #nil; #cons : { head : A; tail : Stream A; }; }` — so infinite
streams are representable and finite generators keep working, all within the
committed strict+TCO / write-barrier-free-GC backend. No new backend capability
was needed: the exploration confirmed recursive types (Phase 25), recursive
unions with a function field (Phase 35), and nullary closures (D-0003) already
lower and evaluate._

- **Builtin source prelude (G1-P).** `crates/general/hir/src/lower/mod.rs` parses
  a fixed prelude (`PRELUDE_SRC`) declaring `Stream` as a recursive `type` alias
  and lowers it into every module. Prelude decls are appended after user decls
  (keeping user binding ids / decl positions stable) and **included only when the
  user program references a prelude name** (reachability scan over `type_arena`
  for `HirTypeKind::BindingRef`), so unused builtins never reach THIR/TLC/codegen.
- **Stream as codata alias (G1.1).** `Stream` removed from the builtin type-name
  list and from the `List`-reduction arms (`thir/lower/types/{apply,alias,levels}`);
  it now resolves through the ordinary recursive-alias path, and Dataflow ties the
  cyclic knot via the alias binding.
- **Generator desugaring (G1.2).** `stream { yield e1; yield e2; }` lowers (HIR)
  to nested unit-thunks + `#cons`/`#nil` cell literals
  (`\_. #cons { head = e1; tail = \_. #cons { head = e2; tail = \_. #nil } }`).
- **Observability (G1.4).** A `Stream` value is now a closure, observed by forcing
  (`s ()` → `#nil`/`#cons` cell) and folding, not printed as a list.
- **Acceptance.** Finite generator folds to the same value on interpreter and
  native (`compile_codata_stream_finite_generator_matches_oracle`); an `unfold`-
  style infinite `nats` stream bounded by `takeSum 5` **terminates** with the
  correct prefix on both paths (`compile_codata_stream_infinite_take_matches_oracle`).
  1611 workspace tests pass.
- **Effectful generators deferred.** Under codata a `yield perform …` defers its
  effect into the cell thunk, so it no longer threads through a pure `Stream A`;
  effectful / resource-backed generators are V3-G4 and are now *rejected* (refused,
  never miscompiled), not silently dropped.

### Phase 34: Conservative mark-sweep GC (opt-in bridge collector) ✅

_Completed 2026-06-24. Built after the gate condition (b) was instrumented and
shown met — the post-Phase-33 accumulator's footprint is O(n) garbage against an
O(1) live set (`compile_emit_bin_accumulator_garbage_dominates_gc_gate`). The
collector landed **opt-in**; the committed default was leak-by-default (D-0008), so
all pre-existing behavior and tests were unchanged. (**Later flipped to on-by-default
2026-06-25** — see "GC default-on (D-0008 reversal)" above; the machinery here is
unchanged, only the default gate.)_

**Outcome: a zero-ABI conservative non-moving mark-sweep collector
(`crates/general/runtime/src/lib.rs`), enabled by `ZUTAI_GC` (and
`ZUTAI_GC_STRESS` = collect before every allocation).** With it enabled, an
accumulator's realized footprint (peak committed) stays **flat** as work grows 8×
where the leak-by-default arena grows ~linearly:

| n | leak-by-default peak | GC peak |
| --- | --- | --- |
| 100k | 2 MiB | 1 MiB |
| 800k | 13 MiB | **1 MiB** |

- **Design.** Every `arena_alloc` is recorded in a side table (`BTreeMap<start,
  size>`); the bump arena gains a free list. Collection (a) finds roots by
  flushing callee-saved registers with `setjmp` and conservatively scanning the
  active machine stack `[sp, pthread_get_stackaddr_np)`, every word a candidate
  pointer; (b) traces reachable objects by scanning their words the same way
  (interior pointers resolve via a range query); (c) sweeps unmarked objects to
  the free list (first-fit + coalescing). Allocation prefers free-list reuse,
  then bump, then collect-and-retry under pressure, then a new chunk — so steady
  state stops growing committed memory.
- **No ABI change (D-0008 endgame, step 1).** Conservative scanning accepts false
  retention precisely to avoid the shadow-stack / stack-map calling-convention
  change a precise collector would need; D-0002 (untagged `i64`) is not reopened.
- **Safe direction on failure.** If the stack bounds cannot be established the
  cycle is abandoned *before sweeping* (retain/leak, never free a live object).
  Stack bounds are wired up for macOS (`pthread_get_stackaddr_np`) and Linux
  (`pthread_getattr_np` + `pthread_attr_getstack`); other targets keep the
  collector off (leak-by-default) regardless of the env var. The Linux path is
  verified in a glibc/aarch64 container by an in-process stress test
  (`collector_retains_live_objects_through_stress`) that retains stack-only-rooted
  objects through collect-before-every-allocation.
- **Soundness.** Collection runs only at allocation safe points (synchronously
  inside `try_alloc`). Validated by summing a fully-built 2000-node live list
  with the collector running before *every* allocation
  (`compile_emit_bin_gc_stress_preserves_live_structure`) — a missed root would
  corrupt the list and break the sum. The macOS/arm64 `setjmp` register-save set
  (x19–x28, including x19/x20 despite a stale SDK header comment) was verified
  empirically; the load-bearing requirement is documented at the `setjmp` extern.
  Collector internals (free-list split/coalesce, object-table range lookup, chunk
  classification) have direct unit tests.
- **Reporting.** `ZUTAI_HEAP_STATS` gains a `zutai gc stats:` line (collections,
  bytes/objects reclaimed).
- **Still gated.** Lazy backend (write barrier) and the precise moving (Cheney)
  endgame stay future work; strict-plus-TCO remains committed.

### Phase 35: Escaping-effect residual-ABI spike — go/no-go ✅

_Completed 2026-06-24. Time-boxed feasibility spike from `docs/TBD.md`
"Phase 35". Question: can a reified `Free Op A` free-monad encoding lower over
the cyclic `DfTyId` types (Phase 25) to carry the genuinely-escaping effects the
backend still rejects (recursive/self-tail effectful callees, polymorphic /
higher-order effectful values, partial applications, open effect rows)? The
representational blocker `tlc-core.md` §9 named — DC types being finite
structural trees — was already lifted by Phase 25; what remained was
investigation, not design._

**Outcome: representation proven viable; strict-AOT-rejects stays the committed
behavior; spike closed.** The encoding is de-risked and ready to scope if a real
workload ever demands native recursive effects — the same demand-gated posture
as the Phase 34 GC.

- **Encode (✓).** `Free Op A = #pure { value: A } | #op { payload; resume: R ->
  Free }` is an ordinary recursive union whose operation arm holds a function
  field whose codomain is the recursive type — structurally identical to `Tree`,
  and it lowers through the same equirecursive cyclic-`DfTyId` knot-tying. No new
  TLC node is required; the perform spine is a real DC value built from existing
  `Variant`/`Record`/`Lam` vocabulary.
- **Lower one case (✓).** The hand-defunctionalized equivalent of a recursive,
  self-tail effectful callee (the simplest rejected case) compiles
  DC → ANF → SSA → native and matches the `zutai-eval` oracle, including
  threading the resumed value back through the stored `resume : Int -> Free`
  closure across an unbounded fold (`compiled_free_monad_spine_matches_oracle`,
  `crates/cli/tests/cli.rs`). An analogous recursive, self-tail effectful callee
  written directly with `perform`/`handle` still runs in the interpreter but is
  refused by the backend (`compile_rejects_recursive_effectful_callee`),
  confirming both the rejection baseline and that the encoding crosses it.
- **Cost it (✓).** Measured with `ZUTAI_HEAP_STATS` at 10 operations: the reified
  spine allocates ~1040 B / 33 objects (one boxed variant + one payload record +
  one `resume` closure per op); the handler-passing CPS path the backend already
  lowers allocates ~512 B / 31 objects (two closures + one arg tuple per op) —
  roughly **2× the bytes**, because the free-monad path materializes the whole
  perform spine as inspectable heap data instead of only continuation closures.
  The CPS comparison is necessarily non-recursive (the recursive CPS case is
  exactly what is rejected), so the trade is: the encoding reaches a case CPS
  cannot, at ~2× allocation.
- **Cases the encoding does NOT reach.** It covers the **monomorphic,
  closed-row recursive/self-tail** callee. It does not by itself reach
  polymorphic effectful values (the operation summand and result must be
  monomorphized; genuine HKT-style effectful values stay check-only by the v1
  residual design, `unify.rs` "a refused check is the safe direction"),
  higher-order effectful values whose operation set is not statically known (the
  `Op` union cannot be enumerated to defunctionalize), or **open effect rows**
  (`RVar` is an unbounded operation set the closed union cannot represent;
  rejected at the gate). Partial applications are an orthogonal
  saturation/elaboration concern, not a representational one.
- **Why no-go on delivery.** Three reasons, all consistent with the repo's
  demand-gated posture: (1) ~2× allocation is a non-trivial standing cost; (2)
  the cases reached are narrow (monomorphic closed-row recursive only) while the
  broader rejected set stays out of reach; (3) no current workload needs native
  recursive effects — strict-AOT-rejects (a refused compile, never a
  miscompile) remains correct and safe. The remaining delivery work, if demand
  appears, is an elaboration pass that reifies recursive effectful callees into
  `Free Op A` data plus a driver loop and wires it past the residual gate; the
  representation it would target is now proven to lower and run.

### V2-A: Explicit universe-level syntax ✅

_Completed 2026-06-24. Implements the v2 spec §"Explicit Level Syntax"
(`docs/v2_spec/04-universe-levels.md`): the opt-in surface forms `$ℓ` (`$0`,
`$l`, `$(l + n)`, `$(max a b)`) and the `<$l>` level binder now parse, resolve,
and check. Phase 24 had already landed the internal level algebra; only the
surface syntax was missing._

- **Front-end only.** Parser → HIR → THIR; levels erase before TLC / Dataflow
  Core (TLC still maps `TypeKind::Type` to `PrimTy::Nothing`), so backend,
  runtime, and value semantics are unchanged.
- **`TypeKind::Type` carries a `UniverseLevel`.** Previously a flat unit variant
  hardcoded to universe 1; it now holds its level so `type_universe(Type(ℓ)) =
  ℓ+1`, distinguishing `$0 : $1` from `$1 : $2`. Bare `Type` lowers to a fresh
  level meta (unchanged inference); explicit `$ℓ` lowers to the named level.
- **Per-use linking, not prenex polymorphism.** Each `<$l>` binder mints one
  shared meta for every `$l` occurrence in a signature, solved from the use site
  and defaulted to the lowest consistent universe exactly like bare `Type`. The
  verification gate holds: explicit levels reject nothing a well-founded
  bare-`Type` program already accepts.
- **Four diagnostics.** Explicit level too low (`Bad :: $0 = $0`, THIR
  `ExplicitLevelTooLow` via cumulativity over `constrain_level_leq`), level
  variable used as a type, non-level name used as a level, and unknown level
  variable (the latter three in HIR resolution, reusing the row-tail-target
  cross-kind pattern). A declared-but-unused `<$l>` is reported.
- Parser/HIR/THIR test modules cover round-trip, resolution, the six spec
  examples, per-use linking, the four diagnostics, and the bare-`Type` corpus.

### Track B: Host-capability entry boundary ✅

_Completed 2026-06-24. Implements the v2 spec §"Entry Boundary"
(`docs/v2_spec/02-host-capabilities.md`): a program may declare the host
capabilities it needs as its entry parameter and the host supplies them. The six
standard host ops already ran end-to-end via direct `perform` (the CLI grants
`HostEffectSet::ALL`); the only gap was the idiomatic `main :: { caps } -> Result`
shape, rejected as "entry returns a function" with no way to obtain a capability
value._

- **Mechanism.** A TLC pass `apply_entry_capabilities`
  (`crates/general/tlc/src/entry.rs`), run in `lower_thir` so the interpreter
  (`run`) and native (`compile`) paths share it, applies the entry to synthesized
  advisory capability tokens while its leading parameter is capability-shaped (an
  `Opaque` capability type, or a closed record of them), iterating so curried
  `FsRead -> Env -> R` parameters are all supplied. The entry then has the
  `Result` type the backend renders, and its granted host effects lower to the
  existing `HostOp`/`HostPrint` path.
- **Advisory tokens.** Capability values are never inspected (authority is the
  effect row, not the value), so each token is a `0` literal stamped with the
  capability's opaque type. DC `Lit` validation is a no-op, so the literal kind
  need not match the opaque type; codegen emits an `i64 0` the program never
  reads. A non-capability parameter stops the supply, leaving a genuinely
  function-valued entry rejected as before.
- **`main` symbol fix.** Escaped the reserved C entry symbol in codegen `mangle`:
  a user binding named `main` mangled verbatim to `@main`, redefining
  `define i32 @main`. `$` cannot occur in a source identifier (UAX #31), so the
  rename `main` → `main$user` is collision-free with both source names and the
  `$dep…` witness scheme. A pre-existing latent bug, surfaced because
  function-named entries used to be rejected before code generation.
- **Validation.** Run-vs-compile parity tests for single, record, and curried
  capability entries (reading a temp file through the boundary); plus a
  user-function-named-`main` regression and a non-capability-entry rejection
  test. 1567 workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Phase A: Cross-function effect handler-passing ✅

_Completed 2026-06-24. Closes a run-vs-compile parity gap in algebraic-effect
lowering: a `perform` reached only through a call to a separate effectful
function — handled at the call site — previously ran in the interpreter but was
refused by the native backend ("algebraic effects remain after TLC lowering")._

- **Mechanism (inline-specialization).** A pre-pass in `lower_thir`
  (`crates/general/tlc/src/lower/effects/inline.rs`) beta-reduces fully-saturated
  direct calls to monomorphic, non-recursive, effectful top-level functions into
  their call sites (`f a` → `let p = a in body`, capture-avoiding via binder
  freshening; arguments `let`-bound, never substituted, in left-to-right order).
  The relocated `perform` becomes lexically enclosed by the call site's handler,
  and the existing trusted lexical CPS elaborator discharges it. Sound because
  the interpreter resolves handlers dynamically at perform time (closures carry
  no handler stack), so a directly-applied static lambda inlined at its call
  site reproduces the exact call-time handler stack.
- **CPS over `Case`.** `cps` now handles `Case` by reifying the post-case
  continuation as a join lambda each arm invokes once — needed because pattern
  and curried lambdas lower to `Lam(scrut, Case(…))` / tuple-pattern `Case`s, and
  it also repairs a pre-existing eligibility/`cps` mismatch (eligibility accepted
  a `Case`-bodied handle expression but `cps` left its `perform` residual).
- **Gate + DCE.** Inlined-away effectful callees are dead-code-eliminated from
  `module.decls` (fixpoint), and the residual-effect gate's effectful-function-
  type clause is scoped to types reachable from `reachable_exprs` so an
  inlined-away callee's orphaned `Fun(…!{e})` type (la_arena never frees nodes)
  no longer falsely rejects. This also closed a latent parity bug: an *unused*
  effectful top-level declaration now compiles (matching the interpreter) instead
  of being rejected for merely existing.
- **Stays gated (refused, never miscompiled).** Recursive/mutually-recursive
  effectful callees (SCC guard), polymorphic (`TyLam`) and higher-order effectful
  values, partial applications, and effects escaping the entry boundary.
- **Validation.** Differential `COMPILED_EFFECT_FIXTURES` (single-arg, curried,
  resuming, arg-effect ordering, chained, two-call-site, unused-decl) confirm
  run==compile parity; gate-rejection tests confirm recursion and higher-order
  refuse. Gate stack: 1562 workspace tests pass; `cargo fmt` and `cargo clippy`
  clean.

### Phase D: Open-union match lowering ✅

_Completed 2026-06-24. Closes the Track 1 Phase D item in `TBD.md` and the
open-union half of the v1 row-polymorphism gap. A polymorphic match over a
`<Rest>`-tailed open union now type-checks and compiles with parity, completing
native row polymorphism._

- **Finding.** An anonymous `...` open-union match (`RowTail::Open`) already
  type-checked, ran, and compiled (union matches are tag-dispatched). Only the
  named `<Rest>` form (`RowTail::Param`) failed — at type-checking, with
  "type mismatch: expected union, found #dev". Unlike record selects (Phase C),
  union dispatch is by tag, not by slot, so there is no slot hazard and no
  monomorphization is needed.
- **Fix** (`crates/general/thir/src/lower/types/match_.rs`, `union_rows_match`):
  the rigid `RowTail::Param(p)` case required the found tail to equal the row
  variable (`ft == RowTail::Param(p)`), rejecting a closed member pattern
  (`#dev`, tail `Closed`). It now also accepts a closed found whose members are
  all explicit members of the rigid open union (`extras.is_empty() && (ft ==
  Closed || ft == Param(p))`) — such a value/pattern is a valid case of the
  union; a different rigid tail or an open/flexible found stays rejected as not
  provably covered. The backend's existing tag dispatch lowers the match
  unchanged. Exhaustiveness is unaffected: a `<Rest>` match still needs a
  wildcard (the rigid tail has unknown members).
- Tests: `compiled_open_union_rest_match_matches_oracle` and
  `open_union_rest_match_without_wildcard_is_non_exhaustive` (cli),
  `rest_tailed_union_match_typechecks` (thir). Gate stack: 1560 workspace tests
  pass; `cargo fmt` and `cargo clippy` clean.

### Phase C: Open-row select lowering ✅

_Completed 2026-06-24. Closes the Track 1 Phase C item in `TBD.md` and the
open-row-select half of the v1 row-polymorphism gap. Open-row field reads
(`getN :: { n : Int; ...; } -> Int = x => x.n`) now compile natively with
parity; the field's runtime slot is recomputed for each concrete record._

- **The bug.** A record field's slot is its name-sorted rank, so it depends on
  the sibling fields. An open-row parameter `{ n; ... }` views `n` at slot 0, but
  a concrete `{ extra; n }` puts `n` at slot 1 — the slot-based backend would read
  the wrong field. Previously gated before Dataflow Core.
- **Row-erased monomorphization** (`crates/general/tlc/src/monomorphize.rs`,
  `monomorphize_open_row_selects`, run from the CLI on the backend module before
  Dataflow Core): a concrete-argument call to an open-row-selecting function is
  inlined to `let param = arg in clone(body)`, substituting the parameter's row
  variable by the argument's *extra* fields throughout the cloned body's types.
  The inlined field selects then see the concrete record type and DC computes the
  correct slot. The concrete field set is read from the argument expression (a
  record literal names its fields; the call-site *type* is the open parameter
  view and cannot be used). The cloned body's top type is overridden with the
  call's result type (a clause `Case` body is recorded with the surrounding
  function type — tolerated in lambda-body position, but it would otherwise become
  the entry type). Fully-inlined declarations are dropped (reachability fixpoint).
- **Gate** (`open_row_select_reason`, `crates/general/dataflow/src/lib.rs`) is now
  reachability-scoped (`reachable_exprs`) — the arena retains inlined-away exprs,
  so an arena-wide scan would falsely reject. Genuinely-polymorphic open-row
  selects (a function applied to a still-open argument) stay gated by design; the
  interpreter, which resolves fields by name, runs them and is the oracle.
- Tests (`crates/cli/tests/cli.rs`): `compile_open_row_select_lowers_to_llvm`,
  `compile_bin_open_row_select_matches_oracle`,
  `compile_open_row_select_discriminates_slot_per_concrete_record`,
  `compile_unspecializable_open_row_select_stays_gated`; flipped
  `open_row_select_lowers_after_monomorphization` and the host-grant variant in
  `crates/general/dataflow/src/tests.rs`. Gate stack: 1556 workspace tests pass;
  `cargo fmt` and `cargo clippy` clean.

### Phase 33: Uncurrying / known-call optimization ✅

_Completed 2026-06-24. Closes the Track 2 Phase 33 item in `TBD.md` and the
uncurrying prerequisite of the deferred GC trajectory. On accumulator loops the
calling-convention churn — one closure + one arg-tuple per curried call — is
eliminated entirely; values are unchanged._

- **New SSA op `CallKnown { func, args, tail }`** (`crates/general/ssa/src/lib.rs`):
  a direct multi-argument call to a named worker, emitted by codegen as
  `[musttail] call i64 @func(args…)` (`crates/general/codegen/src/instr.rs`).
- **Uncurrying pass** (`crates/general/ssa/src/uncurry.rs`, run from `lower_anf`
  after lowering, before TCO): from the ANF module it recovers each top-level
  curried function's arity and fully-applied body (peeling nested lambdas, past
  erased `TyLam`s), generates a multi-parameter worker (`$uncurried`) by
  re-lowering that body with every argument as a direct SSA parameter, and
  rewrites every *saturated known-call chain* — a root `ApplyClosure` whose
  closure resolves to a known function (directly or through a materialized
  `Alias { GlobalClosure }`), followed by single-use result→closure links
  totalling `arity` (links need not be consecutive — ANF computes the next
  argument between applications) — into one `CallKnown` of the worker, deleting
  the now-dead intermediate applications. The original curried function is kept
  for value-use and partial application.
- **Tuple scalar-replacement** (`scalar_replace_tuples`): the multi-parameter
  clause `n acc => …` desugars to `match (n, acc) { … }`, building an arg-tuple
  each call; a tuple used only as the base of constant-slot `Select`s is replaced
  by direct aliases to its elements and dropped, removing the surviving
  per-call arg-tuple inside the worker.
- **TCO** (`crates/general/ssa/src/tco.rs`): a return-position `CallKnown` is
  marked `musttail` when its argument count equals the caller's parameter count
  (matching all-`i64` signature), so a self-recursive worker loops in O(1) stack.
- Measured on `HEAP_STRESS_SRC` (`sum 4000 0`): `tuple 4001 → 0`,
  `closure/raw 4001 → 0` (12003 → 4001 heap objects, ~2/3 reduction); result
  unchanged (`8002000`). Regressions:
  `compile_emit_bin_uncurried_accumulator_drops_call_churn` (cli),
  `uncurrying_collapses_saturated_recursive_call` (ssa). Gate stack: 1554
  workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Phase B: Conditional cross-module witnesses ✅

_Completed 2026-06-24. Closes the "Conditional cross-module witnesses" gap of the
v1 native-backend constraints/witnesses item in `TBD.md`. Imported parametric
witnesses (`Eq @(List A)`, `Eq @(Pair A)`, `Eq @(Optional A)`) now dispatch on
both the native backend and the `eval_tlc` interpreter, with differential parity._

- **Structural witness pattern.** A new arena-independent
  `zutai_thir::WitnessPattern` (`crates/general/thir/src/witness_pattern.rs`,
  `export_witness_pattern`) captures a conditional witness target with parameter
  holes. `WitnessExport.conditional` (`crates/general/semantic/src/import.rs`)
  carries the pattern plus per-parameter component-constraint names across the
  import boundary.
- **Native backend.** The CLI gate is narrowed: a parametric witness export with
  a buildable pattern is allowed; only a non-matchable target (e.g. higher-kinded)
  still routes to the interpreter (`extern_witness_tables`,
  `crates/cli/src/commands/mod.rs`). The TLC root lowerer matches the pattern
  against the concrete call-site THIR type (`match_witness_pattern`, mirroring
  `unify_env`), recovers each parameter, and emits the dep-namespaced witness
  global (`$dep{idx}${constraint}$w{binding_id}`) applied via `TyApp`/`App` to the
  recursively-resolved component dicts (`try_extern_conditional_witness`,
  `crates/general/tlc/src/lower/witness_resolve.rs`). Component dicts resolve by
  constraint *name* against the extern tables, so a component constraint the
  importer never declares (`Eq @(List A) :: <A: Show>`) still resolves.
- **Interpreter.** `eval_tlc` collects each dep's conditional witness function
  values and concrete dicts, then instantiates an imported conditional witness on
  demand for every concrete dispatch key the root needs (string-template match of
  the pattern against the `structural_witness_key` dispatch key, then applies the
  witness function to recursively-materialized component dicts), registering the
  result into `operator_witnesses` so the existing `imported_method` dispatch finds
  it (`crates/general/eval/src/tlc_entry.rs`).
- **Two latent bugs fixed.** (1) `alloc_virtual_binding` counted *downward* from
  `len+1`, so the third-plus virtual extern-witness global collided with a real
  `BindingId` (a `GlobalRef("ys")` got the placeholder empty-record type) — now
  counts upward, above the real range; also hardens the concrete-witness path for
  3+ imported witnesses at one site. (2) `structural_witness_key_subst` substituted
  only a top-level type variable, so a nested applied alias (`List (Pair Int)`)
  keyed as `[{fst:@N,...}]`; the key walk now threads the substitution env through
  every position (`structural_witness_key_env`), keying it `[{fst:Int,snd:Int}]`.
- Differential parity tests (`crates/cli/tests/cli.rs`):
  `compile_zt_imported_conditional_pair_witness_matches_oracle`,
  `…_list_…`, `…_nested_…`, `…_optional_…`, `…_cross_constraint_component_…`,
  `…_digit_suffix_record_…`. Gate stack: 1552 workspace tests pass; `cargo fmt`
  and `cargo clippy` clean.

### Interpreter oracle consistency: record equality + imported-method dispatch ✅

_Completed 2026-06-24. Two latent interpreter (TLC-evaluator) correctness bugs found
while probing cross-module conditional witnesses; both fixed independently of the
still-open Phase B native conditional dispatch._

- **Record equality was nondeterministic.** `Value`'s `PartialEq` (the default
  `run`/TLC path) sorted record fields by the field-name string's POINTER ADDRESS
  (`n.as_ref() as *const str`), so two records' independently-allocated name `Rc<str>`s
  sorted in unrelated orders and the zipped comparison mismatched fields. `{fst=1;snd=2}
  == {fst=1;snd=2}` flipped `true`/`false` across runs (ASLR/alloc-order entropy). Fixed
  to sort by name CONTENT (`crates/general/eval/src/value.rs`), matching the THIR oracle
  `values_equal`. Regression: `record_equality_is_order_independent`.
- **Imported-method dispatch was type-unaware.** `imported_method_by_name` resolved an
  imported witness method by NAME only, ignoring the operand type: two same-method
  concrete instances (`Eq @Int` + `Eq @Bool`) were ambiguous → refused (`UnboundBinding`),
  and an imported conditional witness silently ran the wrong instance. The `GetField`
  node carries only the *generic* method scheme, so the concrete operand type is recorded
  at lowering in a new `TlcModule::dict_dispatch_keys` side table (operand `target_key`
  per constraint-method `GetField`, carried through effect elaboration's `alloc_like`).
  `imported_method` now dispatches type-directed: matching instance → correct value;
  no match (including parametric/conditional or abstract operand) → refuse, never a
  type-mismatched by-name pick. Fixes multi-instance concrete dispatch; makes imported
  conditional witnesses refuse cleanly instead of silently miscomputing. Regressions:
  `dispatch_imported_type_directed_witness_selection` (eval),
  `compile_zt_imported_multi_instance_witness_matches_oracle` (cli differential).
- Gate stack: 1546 workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Phase A: Module-import native lowering (.zti + .zt) + witness naming fix ✅

_Completed 2026-06-24. Closes the "Module imports" sub-item of the v1 native-backend
constraints/witnesses item in `TBD.md`; also fixes a conditional-witness dispatch segfault._

- **Phase A.a** (`05fa320`): `.zti` data imports lower inline to Dataflow Core
  constants. `ImportEnv { zti }` threaded through `Lowerer`; CLI builds the map
  from `analysis.import_values`.
- **Phase A.b + A.c**: `.zt` value/function imports — plus transitive and diamond
  chains — compile natively via one-arena Dataflow Core merge.
  - `ImportEnv` replaced by `ImportTarget` (Zti/Zt) + `ModuleInput` + `ProgramInput`.
  - `try_lower_tlc_with_host_grants_and_imports` → `try_lower_program_with_host_grants`.
  - `Lowerer` extended with `enter_module` (clears per-module `BindingId`/`TlcTypeId`
    caches; preserves shared arenas) and `lower_dep_module` / `lower_root_module`.
  - Dependency globals namespaced under `$dep{idx}$`; dep module-value synthetic
    global `$dep{idx}$$value`; collision-free under `mangle`.
  - CLI `collect_dep_analyses` DFS post-order; `build_module_imports` keyed by
    `Rc` pointer (not source string) for correct diamond dedup.
  - Witness gate: modules that export typeclass witnesses are rejected before DC
    (`IMPORT_WITNESS_REASON`); cross-module witness dispatch is still interpreter-only.
- **Witness instance naming** (`5b6d90d`): `Eq @Int` and `Eq @(Pair A)` both got HIR
  binding name `"Eq"`, causing DC to overwrite the concrete dict with the conditional
  TyLam. The conditional dispatch then passed the TyLam (a closure) as the concrete
  `Eq_A_dict`, leading to a `GetField` on a closure at runtime — segfault. Fixed in
  `collect_globals` by appending `$w{binding_id}` to `TopWitness` global names,
  making every witness instance unique. Adds `conditional_pair_witness` to
  `COMPILED_WITNESS_FIXTURES` (differential coverage for field-access inside conditional).
- Gate stack: 1540 workspace tests pass; `cargo fmt` and `cargo clippy` clean.

### Near-term backend hardening: witness dispatch, open-row gate, corpus ✅

_Completed 2026-06-23. Closes both "Near-term hardening" TBD items and advances
the v1 native-backend constraints/witnesses and row-polymorphism items._

- **Per-layer forall-lambda typing** (`crates/general/tlc/src/lower/expr.rs`).
  `lower_lambda` wrapped every TyLam/dict-`Lam` layer and the value-parameter
  peel with the lambda's full `outer_ty`. For a lambda checked against a rank-2
  annotation (`apply (\x. x)` where `apply :: (<A> A -> A) -> Int`) `outer_ty`
  is a `ForAll`, so the value-parameter peel never advanced and every value
  `Lam` was typed `ForAll`; the Dataflow Core structural validator (which
  requires a `Lam` type to be `Fun`) aborted with an ICE while the interpreter
  ran the same program. Now the forall/dict prefix is peeled one
  quantifier/arrow per layer, mirroring `lower/decl.rs`. Confirmed live, not
  just defensive. Commit `accf422`.
- **LLVM string quote escaping** (`crates/general/codegen/src/descriptors.rs`).
  `llvm_string_bytes` emitted a double quote as `\"` inside a `c"..."`
  constant, which closed the literal early and made `llc` reject the constant
  with a length mismatch. Any compiled program with a quote in a rendered text
  value failed to assemble. Emit the `\22` hex escape. Commit `3efa936`.
- **Differential value-rendering corpus** (`crates/cli/tests/cli.rs`). Expanded
  `COMPILED_SHOW_FIXTURES` to cover non-alphabetical records (flat/nested),
  user-union variants, nested tuples, text escaping, and negative integers so a
  compiled-vs-interpreter rendering divergence fails a test. Commit `a654c1a`.
- **Witness dict field-slot preservation** (`crates/general/tlc/src/lower/effects/rewrite.rs`).
  `elaborate_effects` rebuilds expressions through `alloc_like`, which copied
  `expr_types` and `spans` but not `dict_field_slots`. A `GetField` selecting a
  witness method by its sorted dictionary slot lost the slot during effect
  rewriting and the backend fell back to slot 0, dispatching the wrong method:
  `lt 1 2` against an `Ord` witness (sorted slots `gt=0`, `lt=1`) compiled to
  `gt` and returned `false` where the interpreter returned `true`. `alloc_like`
  now propagates `dict_field_slots`. A new `COMPILED_WITNESS_FIXTURES` corpus
  confirms native parity for two-method sorted-slot dispatch, derived record
  equality, a conditional list witness, and a method-level type parameter —
  evidence the prior "zero native support" note for witnesses was stale for
  these shapes. Commits `69e6758`, `608f5f1`.
- **Open-row field-select backend gate** (`crates/general/dataflow/src/lib.rs`,
  `crates/cli/src/commands/mod.rs`). Selecting a field from an open record type
  silently miscompiled: a parameter typed `{ n : Int; ...; }` hides its tail
  fields, so the slot computed from the view type disagreed with the concrete
  runtime layout (`getN { extra = 7; n = 5; }` returned 7 natively vs 5 in the
  interpreter). The slot-based record ABI carries no field names or offsets, so
  open-row select cannot lower soundly without row-erased specialization or
  runtime descriptors. `open_row_select_reason` now gates value-record
  `GetField` on open rows out of Dataflow Core in both `try_lower_tlc` paths,
  and the CLI surfaces it as a clean compile error. Dict-method selects and
  closed records are unaffected; the interpreter still evaluates open rows.
  Commits `b9012d6`, `347d82d`.
- **`variants`/`witness` reflection fold-or-reject** (`crates/general/semantic/src/lib.rs`,
  `crates/cli/src/commands/mod.rs`). The compile-time reflection gate only
  detected `fields`/`schema`, so `variants` reflection silently miscompiled to
  an empty result and the `witness C @T` reflection expression (a dedicated
  `WitnessReflect` THIR node) panicked the Dataflow Core structural validator
  with a `TypeMismatch` ICE. `reflection_builtin_program` also routes the
  run-time evaluator to the THIR oracle, which cannot dispatch through a witness
  dict, so widening it would have regressed `witness`/`variants` evaluation.
  Added `aot_reflection_program` (a superset covering `fields`/`schema`/`variants`/`witness`)
  used only by the `compile`/`dataflow` fold-or-reject gate; the routing detector
  stays at `fields`/`schema`, keeping `witness`/`variants` fold evaluation on the
  TLC path. Now `variants (Color)` and `(witness Show @Point).show p` fold to the
  interpreter's value, and a bare non-serializable `witness` dictionary is
  rejected cleanly instead of crashing the compiler. Commits `3033284`, `14aff1b`.
- **Module-import backend gate** (`crates/general/dataflow/src/lib.rs`). A
  compiled program that imports a `.zt`/`.zti` module crashed at runtime: TLC→DC
  lowers `TlcExpr::Import` to `DfNodeKind::Import`, which ANF turns into an
  `AnfExpr::Error` leaf, and the imported module is never lowered or linked.
  Programs segfaulted, and an imported operator witness silently dispatched to
  the builtin operator (compiled `1 == 1` against an imported
  `Eq @Int { (==) = \a b. false; }` returned true vs the interpreter's false).
  `import_reason` now gates any module containing an import out of Dataflow Core
  in both `try_lower_tlc` paths; THIR already rejected bare-binding imports as
  "unsupported feature: imports", and this catches record-valued imports that
  slipped past. Cross-module backend linking remains unimplemented; the
  interpreter still resolves imports. Commits `5a6d070`, `aad96ea`.
- Verification: `cargo fmt`; `cargo clippy --workspace --all-targets` (clean);
  `cargo test --workspace` (all pass). Touched-code coverage exceeds 85%; the
  only unhit added line is a defensive `else continue` guard in
  `open_row_select_reason`.

### Documentation spec tree merge ✅

- Merged the v0 and v1 language specifications under `docs/spec/` as
  `docs/spec/v0/` and `docs/spec/v1/`, with `docs/spec/README.md` as the
  versioned specification entry point.
- Updated repository docs, roadmap/archive links, v2 cross-links, the v0 spec
  conformance test fixture paths, and the local `zutai-language` skill routing
  to the new spec paths.
- Recent implementation docs now reflect Unicode XID names, canonical
  interpreter/backend record display, nested destructure SSA hardening,
  per-layer curried lambda typing, TLC evaluator tail-call trampolining, and the
  v1 native-backend backlog.

### Unicode identifiers, atoms, and field names ✅

- General mode (`.zt`) and immediate mode (`.zti`) now accept Unicode UAX #31
  XID names for binding identifiers, field names, and atom bodies, with `_`
  accepted explicitly and atom bodies additionally accepting `-`. Names compare
  by raw Unicode scalar sequence with no normalization. The standard and
  SIMD/NEON immediate parsers share the same behavior; general-mode parsing,
  lossless tokenization, diagnostics lookaheads, type checking, evaluation, and
  the CLI `run` path all cover Unicode bindings such as `café ::= 42`.

### Phase 32: TLC evaluator tail-call trampoline ✅

- The reference TLC evaluator (`crates/general/eval/src/eval_tlc`) is the
  default semantics oracle for executable value programs. It walks expressions
  in continuation-passing style and previously recursed on the host stack for
  every tail call, so deep tail recursion overflowed even the CLI's 256 MiB
  worker stack near depth ~6000 — far short of the backend, which now compiles
  the same recursion to constant-stack `musttail` calls (Phase 31).
- Added an `EvalControl::Tail { ev, id, env, resume }` variant and a `settle`
  driver loop that bounces tail positions instead of recursing, reusing the
  same trampoline shape the evaluator already used for algebraic-effect
  `Perform`/`resume`. `eval_control` is now a thin driver over a renamed
  `eval_step`; the eight tail positions (`TyLam`/`TyApp` bodies, `Let`/`Letrec`
  bodies, closure application, and both `eval_case` branch bodies) emit a `Tail`
  rather than a recursive `eval_control` call. Sub-expression evaluation (call
  arguments, scrutinees, operands) still recurses, bounded by expression nesting
  not recursion depth.
- Every site that matches on an `EvalControl` (`bind_rc`, `finish_top`,
  `handle_control`) settles it first, so a `Tail` never escapes into a matcher;
  effect semantics are unchanged.
- Effect: the default evaluator runs tail-recursive programs in constant
  host-stack space (`sum 1000000` now evaluates to `500000500000`, matching the
  compiled binary), so the differential oracle keeps pace with the backend at
  depth. Non-tail recursion is still O(depth) by nature, as in the backend. The
  secondary THIR walker (`eval`, used only for runtime `Type`/reflection
  programs) is a direct recursive tree-walker and retains the host-stack limit;
  it is exercised only at modest depth.
- Verification: `cargo fmt --check`; `cargo test --workspace` (1504 passed,
  including two new constant-stack regression tests at depth 100_000 on the
  default test-thread stack); `cargo clippy --workspace --all-targets`; manual
  differential check — interpreter and compiled `sum 1000000` agree.

### Phase 31: Backend tail-call optimization (`musttail`) ✅

- New SSA pass `crates/general/ssa/src/tco.rs`, run by `lower_anf`. **Return
  sinking** collapses tail-position `phi`-then-`return` join blocks into direct
  returns from each predecessor — an always-safe CFG cleanup applied to a
  fixpoint so nested tail matches peel from the outside in. **Tail marking**
  then flags any return-position `ApplyClosure` (last instruction whose result
  the block returns).
- Codegen emits marked calls as LLVM `musttail call`, which guarantees
  constant-stack tail recursion regardless of `llc` opt level. Added
  `tail: bool` to `SsaOp::ApplyClosure`.
- Marking is gated to two-parameter closure-code functions (`i64(i64, i64)`,
  matching the indirect callee type). The zero-parameter entry/thunks keep a
  plain `call`, because `musttail` requires matching caller/callee parameter
  lists — one extra stack frame, not unbounded depth.
- Effect: deep tail recursion that previously overflowed the native stack
  (`loop 5000000`, curried `sum 1000000 0`) now runs in O(1) stack, so the
  binding constraint flips from the native stack to the heap ceiling. This is
  the decision-(A) commitment: strict backend plus TCO, GC deferred (see
  `docs/runtime-abi.md` and `TBD.md`).
- The THIR reference interpreter still recurses on the host stack (no TCO), so
  the compiled backend now reaches greater depth than `run`; differential
  fixtures stay at modest depth (e.g. `sum 4000`).
- Verification: `cargo fmt --check`; `cargo test --workspace` (1502 passed);
  `cargo clippy --workspace --all-targets`; manual e2e — `loop 5000000` and
  `sum 1000000 0` exit 0 with correct values; a tight `ZUTAI_HEAP_MAX` now
  yields `heap limit exceeded` instead of SIGSEGV.

### Phase 30: Runtime heap ceiling and allocation telemetry ✅

- Replaced the global `LazyLock`/`Mutex` bump arena with a `thread_local`
  `Arena` of 1 MiB, 16-byte-aligned `Box<[u128]>` chunks: no hot-path lock, and
  per-thread arenas keep the `rlib` sound under a multi-threaded host.
- Heap ceiling (default 2 GiB; `ZUTAI_HEAP_MAX` accepts `k`/`m`/`g`/`0`/
  `unlimited`/`none`): allocating past the cap prints `zutai runtime error:
  heap limit exceeded …` and exits 1 instead of leaking until the OS OOM-kills
  the process. `nil` is now a process-static 16-byte-aligned object.
- `ZUTAI_HEAP_STATS=1` registers an `atexit` dump reporting total bytes and
  objects, average size, peak committed, the cap, and per-kind counts
  (record/tuple/cons/variant/text/closure-or-raw) from always-on
  relaxed-atomic counters.
- Documented in `docs/runtime-abi.md` (D-0008, "Memory model: thread-local
  bump arena, capped leak-by-default").
- Verification: arena unit tests; subprocess tests
  `crates/general/runtime/tests/heap_cap.rs` and `heap_stats.rs`; CLI e2e
  `compile_emit_bin_heap_stats_dump_reports_allocations`.

### Phase 29: Stream-backed generator syntax ✅

- Added contextual generator syntax `stream { yield expr; ... }`. It parses only
  when a `stream` block starts with `yield`, preserving ordinary `stream`
  identifier usage and record application, including `yield` as a record field.
- `Expr::Generator` is source-preserving at syntax/display/span boundaries and
  desugars during HIR lowering to the existing lazy list representation. No
  second effect system or iterator IR was introduced.
- `Stream A` is accepted as a standard one-argument type constructor and
  currently lowers transparently to `List A`; THIR type application, alias
  normalization, and universe-level computation all share that treatment.
- Generator body effects use the existing expression/list effect machinery:
  resource-backed examples require the same capability/effect-row declarations
  as ordinary expressions, and unsupported residual host operations still reject.
- Verification: `cargo fmt --check`; `cargo test --workspace` (1432 passed);
  `cargo clippy --workspace --all-targets`; `cargo llvm-cov nextest --workspace`
  (function coverage 87.91%; line coverage 81.16%).

### Phase 28: Derive recipes and witness reflection ✅

- Constraint declarations can carry `derive = <T> => ...` recipe bodies through
  Syntax, HIR, and THIR; recipe expressions are type-checked before TLC consumes
  the recipe marker.
- `witness C @T` is parsed, typed as a method-record dictionary, lowers to TLC
  dictionary resolution, and reports `WitnessReflectNotInScope` for unresolved
  dictionaries while accepting conditional witnesses such as `Eq @(List A)` for
  `witness Eq @(List Int)`.
- Type-value reflection now includes `variants` alongside `fields` and `schema`,
  returning union variant names and payload-field metadata with recursive
  `Type` back-references preserved as runtime type values.
- Built-in structural equality remains the default derive path. Constraint
  recipes synthesize specialized Show and lexicographic Ord witnesses for records
  and unions, including same-variant payload ordering and derived-dictionary
  reflection.
- Verification: `cargo fmt --check`; `cargo test --workspace` (1423 passed);
  `cargo clippy --workspace --all-targets`; `cargo llvm-cov nextest --workspace`
  (function coverage 87.89%; line coverage 81.11%).

### Phase 27: Host capabilities beyond ambient `io.print` ✅

- Standard host capability type names are seeded in the root scope:
  `FsRead`, `FsWrite`, `Env`, `Clock`, `Rng`, and explicit `IoPrint`. `Path`
  and `Instant` are accepted as standard text-shaped host boundary types.
- THIR effect rows recognize standard operations `fs.read`, `fs.write`,
  `env.get`, `clock.now`, and `rng.next`; capability values remain ordinary
  parameters and authority is advisory only.
- TLC keeps residual host effects explicit and rejects ungranted operations by
  default before TLC→DC lowering. CLI `run`, `dataflow`, and native/LLVM compile
  boundaries grant the standard host set and lower granted residual effects.
- Dataflow adds `HostOp`, ANF/SSA/codegen preserve it, and the runtime/evaluator
  dispatch filesystem read/write, environment lookup, clock, and deterministic
  RNG helpers. Ambient `io.print` remains source-compatible, and source handlers
  can still intercept host operations before the boundary.
- Verification: `cargo fmt --check`; `cargo test --workspace`; `cargo clippy
  --workspace --all-targets`; `cargo llvm-cov nextest --workspace` (function
  coverage 88.17%; line coverage 81.29%).

### Phase 26: Higher-rank polymorphism ✅

- Type syntax extended with nested quantifiers in annotation positions:
  `(<A> A -> A) -> R` and constrained `(<A: Show> A -> Text)`. Parser, HIR,
  THIR, and TLC all carry the `ForAll` node.
- Bidirectional checking pushes written higher-rank annotations into lambda and
  function arguments; inference remains predicative and rank-1.
- ForAll in structural non-argument positions (record fields, union variants,
  list element types, tuple items) rejects with an
  `UnsupportedFeature("impredicative type")` diagnostic.
- TLC elaboration adds explicit `TyLam`/`TyApp` at quantification points and
  `App` for constraint dictionaries at each higher-rank call site.
- `applyId` and constrained `showBoth` examples type-check and run through THIR
  and TLC.

### Phase 25: Recursive type aliases and equirecursive equality ✅

- Guarded recursive and mutually recursive aliases now check through THIR/TLC:
  recursive occurrences under records/unions carry alias identity instead of
  eager expansion, while bare/non-productive cycles still report alias-cycle or
  fuel diagnostics.
- Generic recursive aliases such as `Tree A` pre-register constructor arity,
  preserve universe levels through recursive alias applications, and compare via
  scoped equirecursive type matching without stale fixpoint state or variance
  shortcuts.
- Dataflow Core instantiates generic recursive aliases into finite cyclic
  `DfTyId` graphs; validation remains enabled in debug builds, LLVM descriptor
  emission gets finite back-references, and `check`, `run`, `dataflow`, and
  `compile --emit llvm` cover recursive `Tree`, mutual `Expr`/`Args`, generic
  `Tree A`, and structural equality examples.

### Phase 24: Universe-level foundation ✅

- Internal universe levels now flow through THIR kind checking and TLC kind
  lowering. Surface syntax exposed only `Type` at the time; explicit level
  annotations (`$ℓ`, `<$l>`) landed later in milestone V2-A.
- Type constructors and higher-kinded constraints are level-polymorphic with
  cumulativity and lowest-consistent defaulting, so ordinary v1 type-level/HKT
  programs remain accepted while `Pair Int Type` checks at a higher inferred
  universe.
- Type-level fuel still bounds normalization only; universe-circular definitions
  produce a dedicated kind diagnostic. Runtime erasure and backend output for
  ordinary value programs remain unchanged.

### Phase 23: Effect CPS lowering and runtime dispatch ✅

- General source effects now lower through handler-passing CPS before Dataflow
  Core. `perform`/`handle`/`resume` and explicit sequence markers are eliminated
  into ordinary TLC `Lam`/`App`/`Case`/`Variant`/`Record`/`GetField`/`Let`/
  `Letrec` structure when the supported handled-effect subset is fully covered.
- The CPS pass supports forwarding unmatched operations to enclosing handlers,
  multiple operations per row, nested handler scopes, direct-return clauses, and
  source handlers for `io.print`. Unsupported residual effects and open or
  unsupported effect rows still fail at the TLC→DC safety gate.
- Ambient `io.print` is no longer evaluated at compile time. Direct,
  higher-order, function-valued, branch-dependent, and sequence-dependent print
  uses lower through the runtime `HostPrint` path across Dataflow Core, ANF, SSA,
  LLVM, and the native runtime ABI.
- The old AOT effect fold and host-print capture plumbing were removed:
  `fold_aot_effects`, `fold_effect_value_to_source`,
  `eval_tlc_analysis_capture_io`, the effect duty of `value_to_source`, and
  `emit_llvm_with_host_prints` / `host_prints` replay. Reflection over effectful
  code remains rejected until reflection folding moves behind runtime effect
  lowering.
- Phase 23 closed with a CLI differential harness that compares every compiled
  effect fixture's stdout (final value render plus ordered print sequence)
  against the `eval_tlc` oracle. The language/runtime docs now distinguish the
  implemented handler-passing CPS form from the deferred free-monad data encoding
  that needs recursive or nominal Dataflow Core types.

### Phase 22: Reflection AOT lowering ✅

- CLI `compile` and `dataflow` remove the reflection-gate exit for renderable
  reflection programs by evaluating `fields`/`schema` at compile time and
  re-lowering the folded backend literal before Dataflow Core.
- `schema` on closed records, payload unions, plain enums, and empty records
  compiles to LLVM/native output and renders the serializable reflection shape.
  Typed empty-list bindings preserve the schema shape when folded values contain
  empty `fields` / `variants` lists.
- Raw `fields` outputs that would render embedded `Type` values reject with the
  existing Type-result compile diagnostic; reflection combined with effectful
  code remains refused rather than dropping host effects.

### Native codegen hardening: PIE-safe executable output ✅

- Linux object emission now uses `llc -filetype=obj -relocation-model=pic`, and
  Linux binary linking requests `clang -pie` instead of `-no-pie`.
- Codegen no longer emits `ptrtoint (ptr @...)` constant expressions for static
  descriptor, text, atom, closure, or `@main` addresses. Static globals use
  pointer-typed LLVM fields, and functions materialize static addresses with
  instruction-form `ptrtoint ptr @... to i64`.
- CLI native binary coverage includes primitive, record, tuple, union, text,
  atom, and posit entry values; the Linux PIE matrix passed in a Linux aarch64
  container with `llc`/`clang`.

### Phase 21: Config-overlay AOT lowering ✅

- `overlay` and `overlayDeep` now use the spec's patch-first order, so
  `defaults |> overlay patch` type-checks and evaluates through both reference
  evaluators.
- THIR→TLC lowers supported full applications with record-literal patch values to
  ordinary `RecordUpdate` expressions before Dataflow Core. `overlayDeep`
  recursively lowers required nested-record patches; unsupported residual overlay
  forms and optional nested-record deep overlays remain backend-gated rather than
  producing partial native semantics.
- CLI tests cover `check`/`run`, LLVM/dataflow record-update lowering, native
  shallow and deep overlay binaries, and static unknown-field/type-mismatch
  patch diagnostics.

### Phase 20: Effects AOT lowering (superseded by Phase 23) ✅

- This milestone was the old closed-entry bridge before Phase 23. CLI
  `compile`/`dataflow` attempted a pre-DC fold for closed executable programs
  with fully handled effects by running the TLC semantics oracle on a 256 MiB
  worker stack, serializing the forced backend value to pure source, and lowering
  that pure TLC to Dataflow Core.
- Captured `io.print` host output was replayed in generated `@main` through the
  existing `zutai.text_from_global` / `zutai.print_text` runtime ABI; native
  binaries for `print "hello"` printed `hello` before rendering the final
  `"hello"` value.
- Residual/unfoldable effects, effectful function entries, and non-backend
  values remained rejected before Dataflow Core. Direct
  `zutai_dataflow::try_lower_tlc` still gated raw residual
  `TlcExpr::{Perform,Handle,Resume}` and non-empty function rows.
- Phase 23 superseded this approach: the AOT fold and host-print capture path are
  deleted, and supported effects now compile through runtime CPS/`HostPrint`
  lowering.

### Phase 18: Runtime and ABI ✅

- Runtime/codegen now use dense per-union variant tags in construction and type
  descriptors, with `Optional`/`Maybe` fixed at absent/none = 0 and
  present/some = 1.
- Codegen emits static descriptors from `DfTy`, including field/tag name
  strings, and `@main` calls `zutai.show` plus a trailing newline. Function and
  `Type` entry values are rejected with precise compile diagnostics.
- `compile --emit=llvm|obj|bin` writes LLVM text, assembles objects with `llc`,
  and links native binaries with `clang` against `libzutai_rt`. Missing host
  tools produce actionable diagnostics; object/binary tests skip when the
  toolchain is absent.
- v0 keeps the leak arena. Object headers still reserve high bits for future
  precise/generational GC layout IDs, and runtime descriptors provide the
  pointer-shape bridge for that later collector.

### Phase 19: Effect AOT boundary ✅

- Host authority beyond `io.print` is explicit capability passing only:
  filesystem, network, environment, time, and randomness stay out of the
  ambient prelude and must be represented by capability values plus effect rows.
- This boundary was later refined by Phase 23. General source effects still do
  not enter DC as `Perform`/`Handle`/`Resume`, but ambient `io.print` now has a
  narrow runtime `HostPrint` path across DC/ANF/SSA/LLVM. Unsupported residual
  effects and open/unsupported effect rows still reject before Dataflow Core;
  `try_lower_tlc` keeps the same no-silent-erasure safety role for direct
  library callers.

### Phase 18 D-0004: Slot-indexed records ✅

- TLC→DC resolves canonical record slots by lexicographically sorted field names;
  witnessed-method dictionary access records its slot during THIR→TLC lowering.
- DC/ANF/SSA pass integer slots to LLVM. `Select`, `RecordUpdate`, record/tuple
  patterns, and variant payload binding no longer use field-name hashes.
- Runtime support uses the existing slot-keyed `zutai-rt` helpers. D-0004 is
  verified by IR-text slot assertions plus `zutai-rt` record round-trip tests;
  native binary parity remains Phase 18 D-0010 work.

### Phase 17: Reflection builtins (`fields` / `schema`) ✅

- `fields T` and `schema T` parse as ordinary applications to compiler-known
  builtins.
- Record and union reflection produce deterministic runtime type values / schema
  output through the THIR type-value evaluator.
- Open rows are handled explicitly at the reflection boundary.
- Compile/dataflow reject reflection builtins until their outputs are lowered to
  ordinary backend values.

### Phase 16: Effect evaluation and ordering model ✅

- `docs/spec/v1/05-effects.md` now specifies sequencing for `perform`, `handle`,
  operation clauses, `resume`, and sequence expressions.
- TLC evaluation supports handled effects with delimited continuations.
- `print` was re-pointed to `io.print`; host `run` handles residual `io.print`.
- Backend support now includes Phase 20 closed-entry effect folding; residual
  effectful functions and unfoldable effect values remain backend rejections.

### Phase 15: Effect typing ✅

- Function effect rows are represented in THIR and TLC.
- Effect rows are kinded and unified.
- `perform` is checked against the ambient or locally handled effect row.
- `handle` removes handled operations and forwards unhandled operations.
- `resume` result types and one-shot usage are checked.
- `run`/`compile` originally rejected all effectful programs; Phase 16 later
  enabled TLC `run` while keeping backend rejection.

### Phase 14: Method-level type params and higher-kinded constraints ✅

- Method-level type parameters are preserved in HIR/THIR and elaborated to TLC
  `TyLam`/`TyApp`.
- Dictionary passing handles polymorphic methods.
- Constraint targets of kind `Type -> Type` are kind-checked.
- Partial type application in witness targets works, e.g. `Functor @(Result E)`.

### Phase 13: Conditional witnesses ✅

- Parametric witness targets such as `Eq @(List A) :: <A: Eq>` resolve
  recursively through type arguments.
- Witness search normalizes aliases, handles nested parametric aliases, and
  reports recursive or ambiguous search.

### Phase 12: `derive` synthesis ✅

- `derive` synthesizes structural equality-family witnesses for records, tuples,
  and unions.
- Non-derivable constraints and unsupported required methods are rejected.
- Synthesized witness dictionaries feed the existing TLC dictionary-passing path.

### Phase 11: `select` semantics and compile support ✅

- Value-level `select` lowers to record projection plus record construction.
- Type-level `select` lowers to closed record type construction after
  normalization.
- Unknown selected fields are rejected with source-located diagnostics.
- Concrete value-level `select` compiles through Dataflow Core, ANF, SSA, and
  LLVM IR text.

### Phase 10: THIR→TLC row elaboration ✅

- THIR open records/unions lower to TLC rows.
- Named row tails lower to TLC `RVar`.
- Zonking/substitution covers row variables.
- Closed-type positions contain no unresolved row variables after elaboration.

### Phase 9: Row-polymorphic THIR ✅

- THIR records and unions carry row tails.
- Row-variable kinding and first-order row unification support closed rows,
  anonymous open rows, and named row tails.
- Field access through open record/view types is checked.
- Non-principal row-polymorphic inference requires explicit annotations.

### Phase 8: v1 HIR lowering ✅

- HIR represents record/union row tails, value/type `select`, function effect
  rows, `perform`, `handle`, and `resume`.
- Row variables resolve from type-parameter scopes and are distinguished from row
  spread aliases.
- Syntax-context diagnostics catch duplicate selected fields, duplicate explicit
  row fields, invalid row-tail placement, and `resume` outside operation handler
  clauses.

### Phase 7: v1 parser frontend ✅

- Parser covers ellipsis row tails, value/type `select`, algebraic-effect
  surface syntax, and reflection builtin applications.
- Existing v1-adjacent constructs from the v0 cycle include constraints,
  witnesses, `derive`, bounded/kinded type params, and operator method names.

### Phase 6: CLI compilation ✅

- CLI subcommands: `parse`, `check`, `run`, `repl`, `compile`, and `dataflow`.
- `compile` runs semantic → TLC → DC → ANF → SSA → LLVM IR text.
- Diagnostics remain source-located through the semantic facade.

### Phase 5: SSA and LLVM IR ✅

- `crates/general/ssa/` and `crates/general/codegen/` exist.
- ANF lowers to basic-block SSA with phi nodes.
- Codegen emits LLVM IR text using an `i64` universal value representation for
  v0 and external posit helper declarations.

### Phase 4: ANF lowering ✅

- `docs/anf.md` and `crates/general/anf/` exist.
- Dataflow Core lowers through SCC analysis, topological sorting, and `let` /
  `letrec` introduction.

### Phase 3: Dataflow Core ✅

- `crates/general/dataflow/` exists.
- TLC lowers to a graph where locals are shared, globals are `GlobalRef`s, and
  recursion is explicit.
- Validation checks graph invariants in debug builds.

### Phase 2: TLC ✅

- `crates/general/tlc/` exists.
- TLC IR covers `TyLam`/`TyApp`, rows (`RVar`), singletons, variants, kinds,
  effect rows, NbE normalization, dictionary-passing elaboration, and witnessed
  comparison-operator lowering.
- `zutai-semantic` exposes `TlcModule` for complete analyses.

### Phase 1: THIR / LSP foundation ✅

- Parser, HIR, and THIR cover v0 syntax and source-located diagnostics.
- Implemented forms include optional access/defaulting, tuple/record patterns,
  match exhaustiveness, lambda lowering, no-signature function inference,
  predicative polymorphism, imports, constraints, witnesses, and operator
  witness dispatch.

