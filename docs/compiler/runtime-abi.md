# Runtime and ABI

This document specifies the runtime library and the binary ABI of compiled
Zutai general-mode (`.zt`) programs. It is the design contract for the final
pipeline layer: turning `zutai-codegen` LLVM IR into an object, native binary,
or native library that links against `libzutai_rt`.

> **Status: Phase 18 implemented for the stable ABI surface.** The `zutai-rt`
> crate defines the runtime symbols; codegen emits the D-0003 uniform closure
> ABI, dense per-union variant tags, static `DfTy` descriptors, and a
> type-directed `@main`; `compile --emit=llvm|obj|bin|lib` selects LLVM text,
> object, native binary, or native shared-library output. Object mode uses
> `llc -filetype=obj -relocation-model=pic`; Linux binary mode requests
> `clang -pie`; library mode exports `zutai_entry*` host symbols and a
> descriptor-backed `serde_json` bridge.

## Pipeline position

```
Source → HIR → THIR → TLC → DC → ANF → SSA → LLVM IR  (text)
                                                  ↓  llc/clang: assemble
                                              object file
                                                  ↓  link against libzutai_rt
                                     native executable / shared library
```

The runtime/ABI layer is everything below the dotted line: the toolchain driver
(`compile` invoking `llc`/`clang`), the runtime static library
(`libzutai_rt`), and the binary contract between emitted code and that library.

---

## Initial gap (closed by Phase 18)

Before Phase 18, the pipeline produced a `.ll` file and stopped. The closed
gaps were:

- **Runtime symbols were declared but undefined.** `crates/general/runtime/`
  now provides the runtime definitions for allocation, records, tuples, lists,
  variants, text, output, and `zutai.show`.
- **No driver.** `run_compile` now supports `--emit=llvm|obj|bin`; object and
  binary modes invoke `llc`/`clang`, build/link `libzutai_rt`, and diagnose a
  missing host toolchain.
- **No execution path.** CLI coverage includes the native-driver path when the
  host toolchain is present and keeps cheap IR-text shape checks for every
  backend lowering invariant.

Three concrete defects drove the ABI work and are now closed:

1. **Closures are constructed and applied uniformly.** Function values are
   closure objects `{ header, code, caps[] }`; codegen loads the code slot and
   calls it as `i64 fn(i64 self, i64 arg)`, so top-level and capturing closures
   share one call convention.
2. **Record writer/reader key spaces agree.** Construction, selection, update,
   and record/tuple pattern binding all use canonical ordinal slots rather than
   string hashes.
3. **`@main` is type-directed.** Codegen emits a static descriptor for the entry
   `DfTy`, calls `zutai.show`, prints a trailing newline, and rejects function /
   `Type` entry results instead of producing misleading output.

---

## Design principles

- **Correctness over speed.** the native backend is established. Match the
  `zutai-eval` oracle, then optimize. A wrong value is worse than a slow one.
- **Static typing carries the representation.** General mode is fully typed and
  type-erased before Dataflow Core (Decision 0002 in [`tlc.md`](tlc.md)). Every
  value's static type is known at every use site, and rows/effects are closed
  before codegen. The runtime therefore does **not** need runtime type tags for
  dispatch — the compiler picks the right helper. Tags exist only for printing
  and debugging; the current conservative collector traces from the allocation side table.
- **One uniform call convention.** Eliminate the Global-vs-closure split that
  produces defect (1); every function value is applied the same way.
- **Boring memory first.** Ship a bump arena first; the runtime now layers a
  default-on conservative collector over that arena without changing object word layout.

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
matching the operand's known type. This is sound for the stable language because all dispatch is
resolved at compile time (monomorphized, dictionary-passed, rows/effects
closed).

- *Alternatives.* Low-bit pointer tagging (breaks full-range `i64`), NaN-boxing,
  or a tagged `union`. Still rejected: they buy runtime type recovery not needed
  by static dispatch or the conservative collector.

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
- A known-arity direct-call fast path is a **future optimization** (skip the
  closure object and call the lifted function directly when the callee's arity
  is statically known and saturated at the call site), not part of the stable
  ABI contract.

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

Optional record fields use the same physical slot discipline, but their stored
slot value is the presence envelope: `field? : T` stores `Maybe T`, with omitted
fields as `#absent` and present fields as `#present (value)`. The descriptor's
per-field optional flag tells `show` and `value_eq` to skip absent fields and
unwrap present payloads when rendering/comparing source-level records.

*Implementation impact:* `SsaOp::Select`/`RecordUpdate` must carry the resolved
slot index (computed from the base's `DfTy`), not the field name string.

### D-0005 — List, variant, optional ABI

```
ZtCons    { i64 header; i64 head; i64 tail; }   // TAG_CONS
nil       = a unique sentinel value             // TAG_NIL (immediate or singleton)
ZtVariant { i64 header; i64 tag; i64 payload; } // TAG_VARIANT
```

- `i64 zutai.list_nil()`, `i64 zutai.list_cons(i64 head, i64 tail)`. Read-only
  accessors `i64 zutai.list_is_nil(i64 v)` (`1`/`0` Bool), `i64 zutai.list_head(i64 v)`,
  `i64 zutai.list_tail(i64 v)` back the stream `toList`/`fromList` list-bridge
  primitives (`list_head`/`list_tail` are undefined on nil; the `.zt` source guards
  them with `list_is_nil`).
- `i64 zutai.variant_new(i64 tag, i64 payload)`, `i64 zutai.variant_tag(i64 v)`,
  `i64 zutai.variant_value(i64 v)`.
- **Optional/Maybe** are ordinary variants: `#none`/`#some (v)` and
  `#absent`/`#present (v)`, matching the interpreter split documented in
  the [implementation history](../history/README.md). Source `??` lowers to control flow so fallback expressions stay
  lazy; the strict `i64 zutai.coalesce(i64 v, i64 fallback)` helper remains for
  residual helper-shaped IR and unwraps exactly one layer.

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
  when the repr is finite and has no `.`/`e`/`E` (`value.rs:523-533`)—the old
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
is not recoverable from the closure object at runtime. `compile` **rejects**
such entry types with a precise diagnostic rather than emitting a
parity-breaking `<function>`. (`Type`/witness results are already gated upstream.)

`@main` returns `i32 0` on success; a runtime fault calls `@exit` nonzero.
stdout is flushed before exit so streamed `io.print` and the final line are
never reordered or lost.

### D-0008 — Memory model: thread-local bump arena + default-on conservative GC

> **Update (2026-06-27): GC work is closed for the current runtime.** The conservative
> non-moving mark-sweep collector is the committed endpoint. It runs **by
> default** wherever the conservative stack scan can establish stack bounds
> (macOS, Linux); `ZUTAI_GC=0` (or `false`/`no`/`off`) opts back out to the
> original leak-by-default arena, and platforms without a stack-bounds path stay
> leak-by-default regardless. The earlier precise/moving trajectory is retired,
> not pending: it would require a shadow stack or stack maps plus pointer-layout
> metadata/calling-convention changes, while the untagged-`i64` ABI and
> strict+TCO write-once backend remain fixed. Historical rationale (why
> leak-by-default shipped first) is preserved below.

- `i64 zutai.alloc(i64 nbytes)` — bump a **thread-local**, chunk-growing arena
  (1 MiB `Box<[u128]>` chunks, so every result is 16-byte aligned *by
  construction*, not by the system allocator's luck). Returns the pointer as
  `i64`. Thread-local rather than global: no lock on the hot path, and each host
  thread gets its own arena, which keeps the `rlib` sound when linked into a
  multi-threaded host (e.g. the parallel test harness).
- `void zutai.free(i64 p)` — **no-op in the stable ABI.** Declared for ABI stability.
- **Heap ceiling.** Committed arena bytes are capped — default **2 GiB**,
  overridable via `ZUTAI_HEAP_MAX` (`k`/`m`/`g` suffixes; `0`/`unlimited`/`none`
  disables). An allocation that would grow the arena past the cap aborts with a
  `heap limit exceeded` diagnostic and `exit(1)`, turning the unbounded leak
  into a clean, debuggable failure instead of an OS OOM-kill. Under the default-on
  collector the cap is a backstop that the reclaimed footprint rarely approaches;
  under `ZUTAI_GC=0` nothing is reclaimed below the cap and the OS reclaims
  everything at process exit.
- The `nil` sentinel is a single process-static (16-byte aligned), not an arena
  allocation — a per-thread arena cannot back a process-global pointer.

- *Rationale.* A first runnable, *correct* backend should not block on a
  collector. The pure/lazy core means reachable allocation is bounded by what
  the program forces; for the fixture/spec programs this is fine, and the cap
  bounds the blast radius when it is not.
- *GC urgency — decision (A), 2026-06-22.* The native backend commits to
  **strict semantics plus tail-call optimization** (`musttail`; ANF→SSA return
  sinking + tail marking, Phase 31), and **defers GC**. Measured rationale:
  before TCO the native stack overflowed (~10^5–10^6 frames) long before the
  heap cap (~10^7–10^8 objects), so GC could not help those programs — TCO
  could. After TCO, deep tail recursion runs in O(1) stack and the **heap
  becomes the binding constraint**, making GC a real space optimization for
  bounded-live / unbounded-allocation programs. ~2/3 of accumulator allocation
  is calling-convention overhead (one arg-tuple + one closure per curried call),
  so **uncurrying lands before any collector**. Caveat to "without an ABI break"
  below: that holds for the *object layout*, but **root-finding** does not —
  untagged `i64` roots (D-0002) need a shadow stack or stack maps for a precise
  collector, which is a calling-convention change. The historical record lives
  in the [2026 H1 implementation history](../history/2026-h1.md).
- *Retired precise/moving trajectory (2026-06-27).* The default-on conservative
  collector is the final GC posture for the current backend. The earlier planned
  path — precise non-moving mark-sweep followed by a generational Cheney copying
  young generation — is no longer active work. It needs exact root maps (shadow
  stack or stack maps) and per-object pointer-layout metadata, which are
  calling-convention and codegen contracts outside D-0008/D-0009. Keeping the
  conservative collector preserves D-0002's untagged `i64` ABI, the strict+TCO
  execution model, and the write-once heap invariant.

  Unsupported targets keep the safe fallback: if stack bounds cannot be
  established, collection is disabled or a cycle returns before sweeping. The
  high object-header bits remain reserved for ABI headroom, but they are not an
  active GC milestone.

  **Why not a lazy backend / write barrier.** A memoizing lazy backend would turn
  thunk update into heap mutation and create old→young pointers, forcing the
  remembered-set/write-barrier machinery that the strict backend avoids. Zutai's
  current compiled core is pure, immutable, and write-once; the collector stays
  barrier-free by preserving that invariant.

  **Reference counting is rejected:** `letrec` produces cyclic immutable data,
  which refcounting leaks. Reintroducing runtime thunks or mutable references in
  a future language version would reintroduce a write barrier (GHC-style on thunk
  update); the current collector stays tied to the strict, no-thunk model.

### D-0009 — Object header and type descriptors

The descriptor concept does **two jobs**; separating them clarifies what the runtime needs
for printing versus what a retired precise collector would have needed.

**Role A — static type descriptors (needed by the runtime).** For every type that
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
       | POSIT   nbits es
       | RECORD   n (name_ref, optional?, desc_ref)^n
                    -- field names/order plus optional-slot storage flag
       | TUPLE    n (named?, name_ref, desc_ref)^n
       | LIST     desc_ref
       | OPTIONAL desc_ref                       -- fixed #none / #some rendering
       | MAYBE    desc_ref                       -- fixed #absent / #present rendering
       | VARIANT  n (tag_str_ref, payload_ref?)^n
name_ref / tag_str_ref ::= (ptr, len) into the static string pool
desc_ref               ::= ptr to another descriptor entry
```

Emitted static objects and descriptors use pointer-typed LLVM fields (`ptr @...`)
for relocatable static references while preserving the runtime 8-byte word layout
that `show` reads as `i64` slots. `ptrtoint (ptr @...)` constant expressions are
forbidden in global initializers; static addresses are materialized inside
functions with `ptrtoint ptr @... to i64` instructions.

`OPTIONAL` and `MAYBE` are distinct from generic `VARIANT` because their arms
and rendering are fixed — they are distinct `DfTy` constructors
(`Optional(DfTyId)` / `Maybe(DfTyId)`, `dataflow/src/lib.rs:192-193`) — so
`show` handles them without enumerating members.

**Variant tag identity (implemented: dense indices).** A runtime `ZtVariant`
stores an integer tag; to render `#tag` and select the right payload descriptor,
`show` matches that integer against the descriptor's members. Within a union,
members are assigned dense indices `0..n` (declaration order) as the runtime
tag. Construction (`SsaOp::Variant`), `MatchDiscriminant`, descriptors, and
`show` all agree by construction, the descriptor's member list is indexed
directly by the runtime tag (no string compare, no collisions), and
`Optional`/`Maybe` get the fixed assignment `#none`/`#absent = 0`,
`#some`/`#present = 1`. The global atom hash is retained only for free-standing
`Atom` values, which are not union discriminants.

**Role B — precise pointer layout (retired for the current runtime).** A precise or moving
collector would need each heap object to describe which slots are pointers: a
record `{x: Int, y: Text}` has an immediate in slot 0 and a pointer in slot 1,
which the kind-tag header alone does not capture. The header still reserves high
bits for an optional future layout/shape id, but the active collector does not
use them: it conservatively scans object words and accepts false retention.

This split remains useful for documentation, but **Role B is not active work**.
Reopening precise/moving GC would require both object layout ids and exact root
maps (shadow stack or stack maps) in the calling convention. The current runtime
does neither; the default-on conservative collector is the committed endpoint.

### D-0010 — Toolchain driver

`compile` accepts `--target=host` (default) or one of the four validated native
triples: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin`, and `aarch64-apple-darwin`. The resolved descriptor owns
the LLVM triple and data layout, object target, shared-library suffix, runtime
archive identity, and linker shape; unsupported hosts and triples do not fall
back to a different platform.

The emit selector remains:

- `--emit=llvm` (default; write `.ll` text or stdout),
- `--emit=obj` (invoke `llc -filetype=obj -mtriple=<target>
  -relocation-model=pic` → object file),
- `--emit=bin` (assemble, then link against the matching `libzutai_rt` → native
  executable),
- `--emit=lib` (assemble, then link against the matching `libzutai_rt` → native
  shared library).

The driver discovers `clang`/`llc` from `PATH` (overridable via
`ZUTAI_CLANG`/`CLANG` and `ZUTAI_LLC`/`LLC`) and preflights the required tools
and target runtime before creating native intermediates. Native assembly and
linking happen in a temporary sibling directory; the requested artifact is
renamed into place only after success, so unavailable targets/toolchains do not
leave partial outputs. Binary and library modes resolve `libzutai_rt.a` from
`ZUTAI_RUNTIME_ARCHIVE`, then executable-relative
`../lib/zutai/<target>/libzutai_rt.a`; only the host target may reuse the
workspace development archive. Linux binaries use `-pie -lpthread -ldl -lm`;
shared libraries use the selected platform's `.so`/`.dylib`, shared-library
flag, and archive force-loading form. `--metadata <path>` records the logical
package roots, package/stdlib identities, compiler compatibility, selected
target triple and data layout, PIC relocation model, artifact kind, and runtime
ABI version.

Library-mode LLVM omits `main` and exports:

- `zutai_entry() -> i64` — evaluate the program and return the raw ABI value.
- `zutai_entry_descriptor() -> i64` — return the static descriptor pointer for
  the entry type.
- `zutai_entry_json() -> i64` — evaluate the program, serialize the entry value
  through the runtime `serde_json` bridge, and return a runtime `Text` object.

Host code can read the returned JSON bytes through the C-friendly
`zutai_text_ptr(i64) -> i64` and `zutai_text_len(i64) -> i64` aliases, or call
`zutai_to_json(value, descriptor)` directly after using `zutai_entry()` and
`zutai_entry_descriptor()`.

**Rust host-call pattern.** Load the produced shared library with `libloading`,
`dlopen`, or the platform equivalent, resolve symbols once, and keep the raw
entry value paired with its descriptor. Prefer `zutai_entry_json()` when the host
only needs natural JSON; use `zutai_to_json(value, descriptor)` when it also
needs to inspect or cache the raw value:

```rust
type Entry = unsafe extern "C" fn() -> i64;
type ToJson = unsafe extern "C" fn(i64, i64) -> i64;
type TextAccess = unsafe extern "C" fn(i64) -> i64;

let entry: Entry = load_symbol("zutai_entry");
let entry_descriptor: Entry = load_symbol("zutai_entry_descriptor");
let entry_json: Entry = load_symbol("zutai_entry_json");
let to_json: ToJson = load_symbol("zutai_to_json");
let text_ptr: TextAccess = load_symbol("zutai_text_ptr");
let text_len: TextAccess = load_symbol("zutai_text_len");

let value = unsafe { entry() };
let descriptor = unsafe { entry_descriptor() };

let json_text = unsafe { entry_json() };
let json_text_again = unsafe { to_json(value, descriptor) };

let ptr = unsafe { text_ptr(json_text) } as *const u8;
let len = unsafe { text_len(json_text) } as usize;
let json_bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
```

The JSON bytes are owned by the loaded Zutai runtime. Copy them into host-owned
storage before unloading the library or before handing them to code that outlives
the runtime call boundary.

---

## Runtime symbol table (the contract `libzutai_rt` implements)

All values are `i64` per D-0002. Slots/indices are 0-based.

| Symbol | Signature | Semantics |
| --- | --- | --- |
| `zutai.alloc` | `i64 (i64 nbytes)` | Bump-allocate from a thread-local arena, 16-byte aligned; returns pointer as `i64`. Aborts (`exit 1`) past the `ZUTAI_HEAP_MAX` cap (default 2 GiB). |
| `zutai.free` | `void (i64 p)` | No-op in the stable ABI. |
| `zutai.record_new` | `i64 (i64 n)` | Allocate a record of `n` slots. |
| `zutai.record_set` | `void (i64 r, i64 slot, i64 v)` | Set slot by **ordinal index**. |
| `zutai.record_get` | `i64 (i64 r, i64 slot)` | Get slot by **ordinal index**. |
| `zutai.record_update` | `i64 (i64 r, i64 slot, i64 v)` | Shallow copy with one slot replaced. |
| `zutai.tuple_new` | `i64 (i64 n)` | Allocate a tuple of `n` slots. |
| `zutai.tuple_set` | `void (i64 t, i64 slot, i64 v)` | Set slot by ordinal index. |
| `zutai.tuple_get` | `i64 (i64 t, i64 slot)` | Get slot by ordinal index. |
| `zutai.list_nil` | `i64 ()` | The nil sentinel. |
| `zutai.list_cons` | `i64 (i64 head, i64 tail)` | Allocate a cons cell. |
| `zutai.list_is_nil` | `i64 (i64 v)` | `1`/`0` Bool: is `v` the nil sentinel? (stream list-bridge primitive) |
| `zutai.list_head` | `i64 (i64 v)` | Head of a cons cell; undefined on nil. (stream list-bridge primitive) |
| `zutai.list_tail` | `i64 (i64 v)` | Tail of a cons cell; undefined on nil. (stream list-bridge primitive) |
| `zutai.variant_new` | `i64 (i64 tag, i64 payload)` | Allocate a variant. |
| `zutai.variant_tag` | `i64 (i64 v)` | Variant tag. |
| `zutai.variant_value` | `i64 (i64 v)` | Variant payload. |
| `zutai.coalesce` | `i64 (i64 v, i64 fallback)` | Unwrap one `Optional`/`Maybe` layer or use `fallback`. |
| `zutai.text_from_global` | `i64 (i64 ptr, i64 len)` | Wrap a static UTF-8 constant. |
| `zutai.text_ptr` / `zutai_text_ptr` | `i64 (i64 text)` | Return the UTF-8 byte pointer stored in a runtime `Text`; the underscore alias is host-FFI friendly. |
| `zutai.text_len` / `zutai_text_len` | `i64 (i64 text)` | Return the UTF-8 byte length stored in a runtime `Text`; the underscore alias is host-FFI friendly. |
| `zutai.text_concat` | `i64 (i64 a, i64 b)` | Allocate concatenated text. |
| `zutai.print_i64` | `void (i64 v)` | Print an integer. |
| `zutai.print_bool` | `void (i64 v)` | Print a boolean. |
| `zutai.print_float` | `void (i64 v)` | Print a float (bitcast from `i64`). |
| `zutai.print_text` | `void (i64 v)` | Print text; also the `io.print` handler. |
| `zutai.print_posit` | `void (i64 value, i64 nbits, i64 es)` | Print a posit by static spec. |
| `zutai.show` | `void (i64 value, i64 descriptor)` | Type-directed render (records/tuples/lists/variants/optionals/posits). |
| `zutai.to_json` / `zutai_to_json` | `i64 (i64 value, i64 descriptor)` | Type-directed natural JSON serialization via `serde_json`; returns a runtime `Text`; the underscore alias is host-FFI friendly. |
| `zutai.host.fs_open_read` | `i64 (i64 path)` | Open a UTF-8 path for text reads; returns an opaque `Reader` handle id. |
| `zutai.host.fs_read_line` | `i64 (i64 reader)` | Read one UTF-8 line as `Text?`; strips one trailing LF and optional CR; EOF returns `#none`. |
| `zutai.host.fs_close_read` | `i64 (i64 reader)` | Close a known reader id; idempotent for known handles. |
| `zutai.host.fs_open_write` | `i64 (i64 path)` | Create/truncate a UTF-8 path for text writes; returns an opaque `Writer` handle id. |
| `zutai.host.fs_write_text` | `i64 (i64 request)` | Write request record `{ contents; writer }`; writes bytes exactly, no newline. |
| `zutai.host.fs_flush` | `i64 (i64 writer)` | Flush a known writer id. |
| `zutai.host.fs_close_write` | `i64 (i64 writer)` | Flush and close a known writer id; idempotent for known handles. |
| `exit` | `i64 (i64 code)` | libc; abnormal termination. |

Closure application is emitted inline (D-0003); no `zutai.apply` symbol is
required, though one may be added later to shrink call-site code.

Net changes from the current declarations: `record_get`/`record_update`/`Select`
re-key from name-hash to ordinal slot; `zutai.show` is added; everything else
keeps its signature but gains a real definition.

---

## Effects

`io.print` is the only ambient host operation in the current runtime ABI.
Source handlers are elaborated before Dataflow Core; any residual unsupported or
ungranted operation remains a compile/dataflow error. Residual ambient
`io.print` lowers to `HostPrint` through DC → ANF → SSA and emits:

```llvm
call void @zutai.print_text(i64 %text)
```

using the text value's runtime pointer. `HostPrint` returns the same `Text`
value, so direct `print "x"`, higher-order `apply print`, and explicit
`perform io.print "x"` keep the reference evaluator contract: streamed raw text
appears before the final `zutai.show` rendering in `@main`.

Non-ambient standard host operations lower through explicit `HostOp` helpers
when granted by the CLI boundary: `fs.read`, `fs.write`, `fs.openRead`,
`fs.readLine`, `fs.closeRead`, `fs.openWrite`, `fs.writeText`, `fs.flush`,
`fs.closeWrite`, `env.get`, `clock.now`, `rng.next`, `load.zti`, and `load.zt`.
Dynamic `load.zti` / `load.zt` return the source-level `Data` envelope; source
handlers can intercept either operation before the runtime boundary.

The scoped filesystem helpers store `BufReader<File>` / `BufWriter<File>` in
runtime handle tables and pass opaque integer ids through compiled code. Unknown
ids abort with a host-boundary diagnostic; closing a known already-closed handle
returns `Unit`. `closeWrite` flushes before closing. The runtime ABI remains
text-only and synchronous for this slice.

---

## Verification gate

- **Codegen/CLI shape tests.** IR text asserts dense variant tags, static type
  descriptors, `zutai.show` entry rendering, closure ABI calls, slot-indexed
  records, residual-effect rejection, and PIE-safe static-address materialization
  with no `ptrtoint (ptr @...)` constant-expression form.
- **Native driver tests.** `compile --emit=obj` / `--emit=bin` / `--emit=lib` tests run when
  `llc`/`clang` are available and skip cleanly when the host lacks that
  toolchain. The Linux binary matrix covers primitive, record, tuple, union,
  text, atom, and posit entry values; library coverage links a C host harness
  against a shared library and includes a Rust host integration test that
  compiles `examples/deploy_readiness.zt`, loads the library, calls
  `zutai_entry()`, `zutai_entry_descriptor()`, and `zutai_entry_json()`, and
  compares the parsed JSON with `zutai json examples/deploy_readiness.zt`.
- **ABI unit tests** in `zutai-rt`: record set/get round-trip by slot, record
  update immutability, list build/traverse, variant tag/value, coalesce on each
  optional shape, text concat, closure capture + curried application, posit
  rendering, type-directed `show`, and descriptor-backed JSON serialization.

Gate: `cargo test --workspace` plus `cargo clippy --workspace --all-targets`
and `cargo fmt --check`; native object/binary execution is toolchain-gated.

---

## Non-goals

- Precise/moving garbage collection; the runtime uses the default-on conservative
  collector over the arena, with `ZUTAI_GC=0` preserving leak-by-default opt-out.
- Tagged-pointer / NaN-boxing / runtime type recovery.
- Multithreading, async, FFI, dynamic linking, separate compilation.
- DWARF/debug info and source-level debugging of compiled output.
- Known-arity direct-call optimization and closure inlining.
- Coercion/cast nodes, GADT equalities (see [`tlc.md`](tlc.md) non-goals).

---

## Crate

The runtime lives at `crates/general/runtime/` (`zutai-rt`),
`crate-type = ["staticlib", "rlib"]`. The toolchain driver lives in `zutai-cli`
(`run_compile`). Codegen changes to meet this ABI (uniform closure calls,
slot-indexed record access, type-directed `@main`, descriptor emission) land in
`zutai-ssa` and `zutai-codegen`.
