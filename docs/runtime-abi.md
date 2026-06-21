# Runtime and ABI

This document specifies the v0 runtime library and the binary ABI of compiled
Zutai general-mode (`.zt`) programs. It is the design contract for the missing
final layer of the pipeline: turning the LLVM IR text that `zutai-codegen`
emits into a program that links and runs.

> **Status: Phase 18 design accepted; runtime skeleton and closure ABI in tree.**
> The `zutai-rt` crate defines the v0 runtime symbols and codegen now emits the
> D-0003 uniform closure ABI (static empty-capture closures for top-level
> functions, heap closures for capturing lambdas, and one curried application
> shape), but the compiler still emits LLVM IR text only. Codegen does not yet
> emit dense variant indices, static type descriptors, a type-directed `@main`,
> or an object/binary link step.

## Pipeline position

```
Source → HIR → THIR → TLC → DC → ANF → SSA → LLVM IR  (text)
                                                  ↓  llc/clang: assemble
                                              object file
                                                  ↓  link against libzutai_rt
                                            native executable
```

The runtime/ABI layer is everything below the dotted line that does not exist
yet: the toolchain driver (`compile` invoking `clang`/`llc`), the runtime
static library (`libzutai_rt`), and the binary contract between emitted code
and that library.

---

## Initial gap (audited against the tree)

Before Phase 18, the pipeline produced a `.ll` file and stopped. Concretely:

- **Runtime symbols were declared but undefined.** `emit_runtime_decls`
  (`crates/general/codegen/src/lib.rs:154`) `declare`s ~20 `@zutai.*` symbols
  (`zutai.alloc`, `zutai.record_new/set/get/update`, `zutai.tuple_*`,
  `zutai.list_cons/nil`, `zutai.variant_*`, `zutai.text_*`, `zutai.coalesce`,
  `zutai.print_*`). `crates/general/runtime/` now provides the skeleton
  definitions; codegen still has to be changed to meet the ABI below.
- **No driver.** `run_compile` (`crates/cli/src/commands.rs:202`) ends at
  `fs::write(out, &llvm_ir)`. There is no `llc`/`clang`/`lld` invocation, no
  object file, no link step, no execution.
- **No execution tests.** CLI and codegen tests assert substrings of the IR
  text (e.g. `predicate::str::contains("call i64 @zutai.record_get")`). The
  reference interpreter (`zutai-eval`) is the only thing that actually runs a
  program.

Three concrete defects are already baked into codegen and can only surface once
the output runs. The ABI design must resolve all three:

1. **Closures are constructed but never applied.** SSA closure-converts lambdas
   into a record `{ __fn = Global(fn), caps... }`
   (`crates/general/ssa/src/lower.rs:181-211`); the lifted function takes
   `captures ++ [param]` as parameters. But `AnfExpr::Apply` lowers to a
   single-argument `SsaOp::Call` (`lower.rs:171`), and codegen turns an indirect
   call into `inttoptr i64 <v> to i64 (i64)*` followed by `call i64 %fn(arg)`
   (`codegen/src/lib.rs:409-418`). So at runtime it would `inttoptr` the
   **closure-record pointer** and call it as code, passing one argument to a
   function that expects `n+1`, and never reading `__fn` or threading captures.
   Any capturing lambda or any partial application of a curried function is
   miscompiled.
2. **Record writer/reader key spaces disagree.** Construction writes fields by
   **ordinal slot**: `record_set(rec, idx, v)` with `idx = 0,1,2,…`
   (`codegen/src/lib.rs:436-443`). Reads key by **name hash**:
   `Select` → `record_get(base, str_hash(field))` (`lib.rs:519-524`), and
   `RecordUpdate` likewise (`lib.rs:456-466`). A constructed record cannot be
   read back.
3. **`@main` is type-blind.** It always prints the entry result with
   `zutai.print_i64` (`codegen/src/lib.rs:636-642`), regardless of whether the
   program returns a record, text, list, or float.

---

## Design principles

- **Correctness over speed.** v0 is the first runnable backend. Match the
  `zutai-eval` oracle, then optimize. A wrong value is worse than a slow one.
- **Static typing carries the representation.** General mode is fully typed and
  type-erased before Dataflow Core (Decision 0002 in `docs/tlc-core.md`). Every
  value's static type is known at every use site, and rows/effects are closed
  before codegen. The runtime therefore does **not** need runtime type tags for
  dispatch — the compiler picks the right helper. Tags exist only for printing,
  debugging, and a future collector.
- **One uniform call convention.** Eliminate the Global-vs-closure split that
  produces defect (1); every function value is applied the same way.
- **Boring memory first.** Ship a bump arena (leak-by-default); design the
  object header so a real collector can be added later without an ABI break.

---

## Decisions

Numbered for review. Each names the chosen option, the rationale, and the
alternatives deferred.

### D-0001 — Runtime language: Rust `staticlib`

`crates/general/runtime/` (`zutai-rt`), `crate-type = ["staticlib", "rlib"]`,
exporting the `@zutai.*` symbols as `#[unsafe(export_name = "...")] extern "C"`.

- *Rationale.* Matches the workspace and toolchain; `cargo` builds the archive;
  value-rendering logic (`zutai.show`) can share shape with `zutai-eval`'s
  `Display`; memory-safe runtime internals.
- *Alternatives.* Hand-written C runtime (smaller, no Rust ABI coupling, but
  duplicates rendering and adds a C toolchain dependency). Deferred.

### D-0002 — Value representation: untagged `i64`, static dispatch

Every Zutai value is a single `i64`, exactly as codegen already assumes:

- **Immediates** (stored inline): `Int` is the `i64` itself (full range, no
  stolen tag bits); `Bool` is `0`/`1`; `Float` is `bitcast f64 → i64`; dense
  variant discriminants are raw integers in the `ZtVariant.tag` slot.
- **Heap values** (pointer cast to `i64`): records, tuples, lists, variants,
  text, free-standing atoms, closures. 16-byte aligned. Each heap block begins
  with a one-word header (see D-0009).

The representation is **untagged**: an `i64` is ambiguous between an `Int` and a
pointer, and disambiguation is purely static — codegen calls the helper
matching the operand's known type. This is sound for v0 because all dispatch is
resolved at compile time (monomorphized, dictionary-passed, rows/effects
closed).

- *Alternatives.* Low-bit pointer tagging (breaks full-range `i64`), NaN-boxing,
  or a tagged `union`. All deferred; they buy runtime type recovery we don't
  need until reflection-at-runtime or a precise GC lands.

### D-0003 — Closure ABI: uniform single-argument closures

Resolves defect (1). **Every** function value — top-level or lambda — is a heap
closure object, and **every** application uses one convention. Currying is
explicit: an n-ary function is n nested closures.

Closure object layout:

```
ZtClosure {
  i64 header;     // TAG_CLOSURE | (ncaps << 8)
  i64 code;       // fn ptr, signature: i64 (i64 self, i64 arg)
  i64 caps[ncaps];
}
```

- The lifted function's signature is `i64 @fn(i64 %self, i64 %arg)`. It reads
  capture *k* from `%self` at slot `2 + k`; `%arg` is its single parameter.
- **Application** `Apply(f, x)` is always:

  ```llvm
  %c    = inttoptr i64 %f to ptr
  %cp   = getelementptr i64, ptr %c, i64 1      ; code slot
  %code = load i64, ptr %cp
  %fn   = inttoptr i64 %code to ptr             ; i64 (i64, i64)*
  %r    = call i64 %fn(i64 %f, i64 %x)
  ```

- A **top-level** function `f` is a statically-allocated closure constant with
  `ncaps = 0`; `GlobalRef(f)` loads that constant. This removes the
  `Global`-direct vs indirect split entirely — codegen emits one shape.
- A known-arity direct-call fast path is a **v2 optimization** (skip the
  closure object and call the lifted function directly when the callee's arity
  is statically known and saturated at the call site), not part of the v0
  contract.

*Implementation impact (out of scope for this doc, tracked for Phase 18):* SSA
must emit a dedicated `MakeClosure`/closure-apply rather than reusing
`SsaOp::Record` + single-arg `Call`. The `__fn`-record hack
(`ssa/src/lower.rs:204-210`) is replaced by this object.

### D-0004 — Record/tuple ABI: ordinal slots, resolved at compile time

Resolves defect (2). Records and tuples are header + contiguous `i64` slots:

```
ZtRecord { i64 header;  i64 slots[n]; }   // header = TAG_RECORD  | (n << 8)
ZtTuple  { i64 header;  i64 slots[n]; }   // header = TAG_TUPLE   | (n << 8)
```

Field access is **slot-indexed**, never name-hashed. The field set of every
record is statically known at codegen time (row variables are closed by
THIR→TLC before DC), so the compiler assigns each field a stable slot from its
declared order and emits the integer index. `record_new/set/get/update` and
`tuple_*` all key by slot:

- `i64 zutai.record_new(i64 n)` / `void zutai.record_set(i64 r, i64 slot, i64 v)`
- `i64 zutai.record_get(i64 r, i64 slot)`
- `i64 zutai.record_update(i64 r, i64 slot, i64 v)` — allocates a shallow copy
  with one slot replaced (records are immutable).

`str_hash`-keyed access is removed. Field *names* survive only in the type
descriptor (D-0009) for rendering and debugging.

*Implementation impact:* `SsaOp::Select`/`RecordUpdate` must carry the resolved
slot index (computed from the base's `DfTy`), not the field name string.

### D-0005 — List, variant, optional ABI

```
ZtCons    { i64 header; i64 head; i64 tail; }   // TAG_CONS
nil       = a unique sentinel value             // TAG_NIL (immediate or singleton)
ZtVariant { i64 header; i64 tag; i64 payload; } // TAG_VARIANT
```

- `i64 zutai.list_nil()`, `i64 zutai.list_cons(i64 head, i64 tail)`.
- `i64 zutai.variant_new(i64 tag, i64 payload)`, `i64 zutai.variant_tag(i64 v)`,
  `i64 zutai.variant_value(i64 v)`.
- **Optional/Maybe** are ordinary variants: `#none`/`#some (v)` and
  `#absent`/`#present (v)`, matching the interpreter split documented in
  `ARCHIVED.md`. `i64 zutai.coalesce(i64 v, i64 fallback)` inspects the tag and
  returns the payload for `#some`/`#present`, else `fallback` — unwrapping
  exactly one layer (parity with `eval_tlc.rs`).

### D-0006 — Text and atom ABI

```
ZtText { i64 header; i64 len; i64 bytes; }  // TAG_TEXT; bytes -> UTF-8, immutable
```

The `bytes` word points to the UTF-8 payload (a static constant or arena-owned),
keeping `ZtText` fixed-size so accessors need no variable-length layout.

- `i64 zutai.text_from_global(i64 ptr, i64 len)` — wrap a statically-emitted
  byte constant: store `ptr`/`len` directly, no copy (the static lives for the
  program's lifetime).
- `i64 zutai.text_concat(i64 a, i64 b)` — allocate fresh bytes in the arena and a
  `ZtText` pointing at them.
- `i64 zutai.atom_from_global(i64 ptr, i64 len)` — build a free-standing atom
  value that carries the atom's source spelling for `show`; it uses the same
  fixed-size name-reference layout as `ZtText`.

Variant tags are not atom values. Union discriminants are dense per-union
indices (D-0009), so `show` recovers tag names from the static descriptor, not
from a global FNV hash.

### D-0007 — Entry point and type-directed printing

Resolves defect (3), and pins the exact contract the differential gate checks.

**Parity target.** Compiled stdout must equal what the interpreter `run` path
emits for the same program. `run` produces, in order:
1. any `io.print` output streamed *during* evaluation (raw text, no quotes), then
2. the final value rendered by `Value`'s `Display`
   (`crates/general/eval/src/value.rs:518`), followed by a single trailing
   newline (`commands.rs:26` prints `println!("{rendered}")` where `rendered`
   is `value.to_string()`, `commands.rs:93`).

A compiled program reproduces (1) by calling `zutai.print_text` from the
`io.print` handler as it runs, and (2) from `@main` after the entry returns —
same calls, same order, same bytes.

**`@main` shape.** No per-scalar special-casing. `@main` always:

```llvm
%r = call i64 @<entry>(...)          ; or load the entry closure + apply
call void @zutai.show(i64 %r, i64 <entry_desc>)
; trailing '\n' via a one-byte zutai.print_text — no new symbol needed
ret i32 0
```

`<entry_desc>` is the static descriptor for the entry's `DfTy` (D-0009).
Routing *everything* through `zutai.show` (not `print_i64` etc.) is what removes
the type-blind bug: `show` becomes the single source of rendering truth and
matches `Display` exactly. The `zutai.print_*` family stays, but only as (a) the
`io.print` handler (`print_text`, raw) and (b) internal leaves of `show`.

`io.print`'s `print_text` is **raw** (the text's bytes); `show` of a `Text`
value is **quoted and escaped** (`"a\nb"`). These are deliberately different and
must not be merged.

**Rendering rules `show` must reproduce** (all from the `Display` impl):
- `Int` decimal; `Bool` → `true`/`false`.
- `Float`: Rust shortest round-trip; bare `inf`/`-inf`/`NaN`; append `.0` only
  when the repr is finite and has no `.`/`e`/`E` (`value.rs:523-533`) — the v0
  `inf.0`/`NaN.0` bug must not regress.
- `Text`: `"`-wrapped, escaping `\" \\ \n \r \t` (`value.rs:537-545`).
- `Atom`: `#name`.
- `List`: `[a; b; c]` — `; `-separated, square brackets.
- `Tuple`: `(a, b)`, named fields as `(name = v, …)` — `, `-separated.
- `Record`: `{ name = v; name2 = v2 }` — leading space, `; `-separated, ` }`.
- `Variant`: `#tag`; positional payload → `#tag (a, b)`; named → `#tag { n = v; … }`.
- `Optional`/`Maybe`: `#none` / `#some (v)`, `#absent` / `#present (v)`.

**Restriction.** A program whose final result type is a function cannot be
meaningfully printed — `Display` renders `<function/N>`, but the residual arity
is not recoverable from the closure object at runtime. v0 `compile` **rejects**
such entry types with a precise diagnostic rather than emitting a
parity-breaking `<function>`. (`Type`/witness results are already gated upstream.)

`@main` returns `i32 0` on success; a runtime fault calls `@exit` nonzero.
stdout is flushed before exit so streamed `io.print` and the final line are
never reordered or lost.

### D-0008 — Memory model: bump arena, leak-by-default

- `i64 zutai.alloc(i64 nbytes)` — bump a global, chunk-growing arena (mmap /
  `realloc` of large chunks), 16-byte aligned.
- `void zutai.free(i64 p)` — **no-op in v0.** Declared for ABI stability.
- The OS reclaims everything at process exit.

- *Rationale.* A first runnable, *correct* backend should not block on a
  collector. The pure/lazy core means reachable allocation is bounded by what
  the program forces; for the v0 fixture/spec programs this is fine.
- *Deferred GC trajectory.* The per-object header (D-0009) and type descriptors
  (exact pointer-vs-immediate slot maps) make a **precise**, and even **moving**,
  collector safe, so a collector can be added **without an ABI break**. The
  planned path:
  1. **v1 — precise, non-moving, stop-the-world mark-sweep.** Simplest correct
     precise collector: no pointer fixup, reuses the descriptors, handles
     `letrec` cycles natively. The bridge off the arena.
  2. **v2 — generational, copying young generation** (Cheney semispace),
     GHC-shaped. Minor GC is pointer-bump allocation plus a cheap survivor copy;
     no fragmentation.

  **Why not Go's collector.** Go's GC is concurrent, non-generational,
  non-moving, with a hybrid (Dijkstra + Yuasa) write barrier — choices tuned for
  a mutable, heavily-concurrent, cgo-interop language. Zutai's compiled core is
  pure, immutable, single-threaded (v0), and write-once (no runtime thunks), so:
  - a **write barrier is unnecessary** — an object can only reference objects
    that existed when it was allocated, so the old generation never points into
    the nursery (the invariant generational GC normally spends a barrier and a
    remembered set to maintain). The one exception, co-allocated `letrec` SCCs,
    is handled by both mark-sweep and copying;
  - **moving is cheap and safe** because layout is precise and there is no
    `cgo`-style raw-pointer interop to pin. This is why a generational copying
    collector (à la GHC, the nearest pure/lazy precedent) is the endgame rather
    than Go's non-moving design.

  **Reference counting is rejected:** `letrec` produces cyclic immutable data,
  which refcounting leaks. Reintroducing runtime thunks or mutable references in
  a future language version would reintroduce a write barrier (GHC-style on thunk
  update); the trajectory above assumes the current no-thunk model holds.

### D-0009 — Object header and type descriptors

The descriptor concept does **two jobs**; separating them clarifies what v0
needs versus what the GC needs later.

**Role A — static type descriptors (needed in v0).** For every type that
reaches `zutai.show` (the entry type and, transitively, its components) codegen
emits a static, read-only descriptor and passes it to `show`. `show` walks the
**value and descriptor in lockstep**, top-down: the descriptor supplies the
field names, element types, and variant tag strings that the untagged value
representation (D-0002) cannot recover on its own. Because the descriptor is the
authority, `show` needs nothing per-object except the header's kind tag to
discriminate sum shapes (nil vs cons; which optional arm; which variant member).

Descriptors are a **flat table of entries referenced by index**, not an infinite
tree — recursive types (`List` of self, recursive records/unions) and shared
sub-types reference existing entries by index, mirroring how `DfTy` already uses
arena `DfTyId`s (`dataflow/src/lib.rs:191-197`). This keeps cyclic/recursive
descriptors finite and lets equal sub-descriptors be emitted once.

Entry grammar (each entry is a small static `i64`/`i8` array):

```
desc ::= INT | BOOL | FLOAT | TEXT | ATOM
       | RECORD   n (name_ref, desc_ref)^n      -- field names + order preserved
       | TUPLE    n (named?, name_ref, desc_ref)^n
       | LIST     desc_ref
       | OPTIONAL desc_ref                       -- fixed #none / #some rendering
       | MAYBE    desc_ref                       -- fixed #absent / #present rendering
       | VARIANT  n (tag_str_ref, payload_ref?)^n
name_ref / tag_str_ref ::= (ptr, len) into the static string pool
desc_ref               ::= index into the descriptor table
```

`OPTIONAL` and `MAYBE` are distinct from generic `VARIANT` because their arms
and rendering are fixed — they are distinct `DfTy` constructors
(`Optional(DfTyId)` / `Maybe(DfTyId)`, `dataflow/src/lib.rs:192-193`) — so
`show` handles them without enumerating members.

**Variant tag identity (decided: dense indices).** A runtime `ZtVariant` stores
an integer tag; to render `#tag` and select the right payload descriptor, `show`
matches that integer against the descriptor's members. Codegen currently derives
variant tags from a **global** FNV hash (`atom_tag`, `codegen/src/lib.rs:528`)
shared by construction and matching — correct only while no two distinct tags
collide. **Decision:** within a union, members are assigned **dense indices
`0..n`** (declaration order) as the runtime tag, replacing the global hash.
Construction (`SsaOp::Variant`), `MatchDiscriminant`, and `show` all agree by
construction, the descriptor's member list is indexed directly by the runtime
tag (no string compare, no collisions), and `Optional`/`Maybe` get the fixed
assignment `#none`/`#absent = 0`, `#some`/`#present = 1`. This requires threading
the statically-known union type into `Variant` construction and
`MatchDiscriminant` lowering so each member resolves to its index; tracked as
Phase 18 work. The global `atom_tag` hash is retained only for free-standing
`Atom` values, which are not union discriminants.

**Role B — per-object layout for the future GC (not in v0).** A collector traces
from roots with no static type at each pointer, so each heap object must be
self-describing about *which slots are pointers*: a record `{x: Int, y: Text}`
has an immediate in slot 0 and a pointer in slot 1, which the kind-tag header
alone does not capture. The header word is therefore laid out as: low byte =
kind tag (`TAG_RECORD/TUPLE/CONS/NIL/VARIANT/TEXT/CLOSURE`), next bits =
length/arity/`ncaps`, **high bits reserved for a layout/shape id** the v1 GC
will use to find the object's pointer-map. v0 leaves the reserved bits zero;
populating them is additive and does not change the v0 ABI.

This split is the key point: **v0 ships only Role A** (static descriptors for
`show`); the header reserves the bits that make Role B (precise GC tracing)
landable later without an ABI break.

### D-0010 — Toolchain driver

`compile` gains an emit selector:

- `--emit=llvm` (default; current behavior — write `.ll`),
- `--emit=obj` (invoke `llc`/`clang -c` → object file),
- `--emit=bin` (assemble, then link against `libzutai_rt` → native executable).

The driver discovers `clang`/`llc` from `PATH` (overridable via env), and emits
a precise, actionable diagnostic when the toolchain is absent rather than
failing opaquely. The Rust runtime archive is built by `cargo` and located via
the build, not hand-pathed.

---

## Runtime symbol table (the contract `libzutai_rt` implements)

All values are `i64` per D-0002. Slots/indices are 0-based.

| Symbol | Signature | Semantics |
| --- | --- | --- |
| `zutai.alloc` | `i64 (i64 nbytes)` | Bump-allocate, 16-byte aligned; returns pointer as `i64`. |
| `zutai.free` | `void (i64 p)` | No-op in v0. |
| `zutai.record_new` | `i64 (i64 n)` | Allocate a record of `n` slots. |
| `zutai.record_set` | `void (i64 r, i64 slot, i64 v)` | Set slot by **ordinal index**. |
| `zutai.record_get` | `i64 (i64 r, i64 slot)` | Get slot by **ordinal index**. |
| `zutai.record_update` | `i64 (i64 r, i64 slot, i64 v)` | Shallow copy with one slot replaced. |
| `zutai.tuple_new` | `i64 (i64 n)` | Allocate a tuple of `n` slots. |
| `zutai.tuple_set` | `void (i64 t, i64 slot, i64 v)` | Set slot by ordinal index. |
| `zutai.tuple_get` | `i64 (i64 t, i64 slot)` | Get slot by ordinal index. |
| `zutai.list_nil` | `i64 ()` | The nil sentinel. |
| `zutai.list_cons` | `i64 (i64 head, i64 tail)` | Allocate a cons cell. |
| `zutai.variant_new` | `i64 (i64 tag, i64 payload)` | Allocate a variant. |
| `zutai.variant_tag` | `i64 (i64 v)` | Variant tag. |
| `zutai.variant_value` | `i64 (i64 v)` | Variant payload. |
| `zutai.coalesce` | `i64 (i64 v, i64 fallback)` | Unwrap one `Optional`/`Maybe` layer or use `fallback`. |
| `zutai.text_from_global` | `i64 (i64 ptr, i64 len)` | Wrap a static UTF-8 constant. |
| `zutai.text_concat` | `i64 (i64 a, i64 b)` | Allocate concatenated text. |
| `zutai.print_i64` | `void (i64 v)` | Print an integer. |
| `zutai.print_bool` | `void (i64 v)` | Print a boolean. |
| `zutai.print_float` | `void (i64 v)` | Print a float (bitcast from `i64`). |
| `zutai.print_text` | `void (i64 v)` | Print text; also the v0 `io.print` handler. |
| `zutai.show` | `void (i64 value, i64 descriptor)` | Type-directed render (records/tuples/lists/variants/optionals). |
| `exit` | `i64 (i64 code)` | libc; abnormal termination. |

Closure application is emitted inline (D-0003); no `zutai.apply` symbol is
required, though one may be added later to shrink call-site code.

Net changes from the current declarations: `record_get`/`record_update`/`Select`
re-key from name-hash to ordinal slot; `zutai.show` is added; everything else
keeps its signature but gains a real definition.

---

## Effects

Effect typing and the residual-effect gate already exist: `compile`/`dataflow`
reject non-empty function effect rows after TLC lowering, and `print` lowers to
an `io.print` effect handled at the host `run` boundary
(`ARCHIVED.md` Phase 16). The compiled host boundary remains gated until the
Phase 19 pre-DC free-monad/CPS lowering and host-boundary interpreter exist; no
additional effect machinery enters the runtime ABI before then.

---

## Verification gate

This is the check that has been missing — IR-text assertions do not prove
execution.

- **End-to-end differential.** Compile each runnable program in the v0 fixture
  and spec-conformance set (`crates/general/eval/tests/v0_spec.rs`,
  `crates/general/fixtures/valid/`) to a native binary, run it, and assert its
  stdout matches the `zutai-eval` oracle's rendering of the same program.
- **ABI unit tests** in `zutai-rt`: record set/get round-trip by slot, record
  update immutability, list build/traverse, variant tag/value, coalesce on each
  optional shape, text concat, closure capture + curried application.
- The existing IR-text tests remain as cheap structural checks.

Gate: `cargo test --workspace` green, **plus** the differential battery green on
a machine with `clang`/`llc` available; the differential is skipped with a clear
notice when the toolchain is absent.

---

## Non-goals (v0)

- Garbage collection (arena + leak; header/descriptors keep the door open).
- Tagged-pointer / NaN-boxing / runtime type recovery.
- Multithreading, async, FFI, dynamic linking, separate compilation.
- DWARF/debug info and source-level debugging of compiled output.
- Known-arity direct-call optimization and closure inlining.
- Coercion/cast nodes, GADT equalities (see `docs/tlc-core.md` non-goals).

---

## Crate

The runtime lives at `crates/general/runtime/` (`zutai-rt`),
`crate-type = ["staticlib", "rlib"]`. The toolchain driver lives in `zutai-cli`
(`run_compile`). Codegen changes to meet this ABI (uniform closure calls,
slot-indexed record access, type-directed `@main`, descriptor emission) land in
`zutai-ssa` and `zutai-codegen`.
