//! Runtime library for compiled Zutai general-mode programs.
//!
//! This crate implements the `@zutai.*` symbols that `zutai-codegen` references
//! and defines the binary representation of runtime values. The full contract
//! is specified in `docs/compiler/runtime-abi.md`; this module is the v0 skeleton of
//! Phase 18 (Runtime & ABI).
//!
//! Every Zutai value is an `i64` (D-0002, untagged, statically dispatched):
//! immediates (`Int`, `Bool`, `Float` bits, dense atom/variant tags) are stored
//! inline; heap values (records, tuples, lists, variants, text) are pointers
//! cast to `i64`, each beginning with a one-word header (D-0009).
//!
//! Memory is a per-thread bump arena (D-0008): each OS thread owns a
//! `thread_local!` arena whose owned chunks are never leaked, bounded by
//! `ZUTAI_HEAP_MAX` (default 2 GiB) so a runaway allocation aborts with a clear
//! diagnostic instead of OOM-killing the host. `zutai.free` is a no-op in v0.
//!
//! Closures (D-0003) are built and applied inline by codegen, so no closure
//! symbol lives here. `TAG_CLOSURE` is reserved for the header layout only.
/// Version of the native runtime ABI consumed by generated code.
///
/// Increment this when a symbol signature, value representation, descriptor
/// layout, or host boundary changes incompatibly.
pub const ABI_VERSION: u32 = 1;

use fast_posit::{Posit, RoundInto};
use zutai_eval::{EvalError, Value as EvalValue};

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::slice;
use std::str;
use std::sync::{
    Mutex, Once, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Object headers (D-0009, Role B reserves the high bits) ──────────────────────

const TAG_RECORD: i64 = 1;
const TAG_TUPLE: i64 = 2;
const TAG_CONS: i64 = 3;
const TAG_NIL: i64 = 4;
const TAG_VARIANT: i64 = 5;
const TAG_TEXT: i64 = 6;
/// Reserved: closures are emitted inline by codegen, not allocated here.
#[allow(dead_code)]
const TAG_CLOSURE: i64 = 7;

/// Pack a header word: low byte = kind tag, next bits = length/arity.
/// High bits stay zero in v0; the current conservative GC does not use layout ids.
const fn header(tag: i64, count: u64) -> i64 {
    ((count << 8) as i64) | tag
}

fn header_tag(h: i64) -> i64 {
    h & 0xff
}

fn header_count(h: i64) -> u64 {
    ((h as u64) >> 8) & 0x00ff_ffff_ffff_ffff
}

// ── Type descriptors (D-0009, Role A — what `show` walks) ───────────────────────

const DESC_INT: i64 = 0;
const DESC_BOOL: i64 = 1;
const DESC_FLOAT: i64 = 2;
const DESC_TEXT: i64 = 3;
const DESC_ATOM: i64 = 4;
const DESC_RECORD: i64 = 5;
const DESC_TUPLE: i64 = 6;
const DESC_LIST: i64 = 7;
const DESC_OPTIONAL: i64 = 8;
const DESC_MAYBE: i64 = 9;
const DESC_VARIANT: i64 = 10;
const DESC_POSIT: i64 = 11;

macro_rules! match_p32_es {
    ($es:expr, $func:ident $(, $arg:expr)* $(,)?) => {
        match $es {
            0 => $func::<0>($($arg),*),
            1 => $func::<1>($($arg),*),
            2 => $func::<2>($($arg),*),
            3 => $func::<3>($($arg),*),
            4 => $func::<4>($($arg),*),
            5 => $func::<5>($($arg),*),
            6 => $func::<6>($($arg),*),
            7 => $func::<7>($($arg),*),
            8 => $func::<8>($($arg),*),
            9 => $func::<9>($($arg),*),
            10 => $func::<10>($($arg),*),
            11 => $func::<11>($($arg),*),
            12 => $func::<12>($($arg),*),
            13 => $func::<13>($($arg),*),
            14 => $func::<14>($($arg),*),
            15 => $func::<15>($($arg),*),
            16 => $func::<16>($($arg),*),
            17 => $func::<17>($($arg),*),
            18 => $func::<18>($($arg),*),
            19 => $func::<19>($($arg),*),
            20 => $func::<20>($($arg),*),
            21 => $func::<21>($($arg),*),
            22 => $func::<22>($($arg),*),
            23 => $func::<23>($($arg),*),
            24 => $func::<24>($($arg),*),
            25 => $func::<25>($($arg),*),
            26 => $func::<26>($($arg),*),
            27 => $func::<27>($($arg),*),
            28 => $func::<28>($($arg),*),
            29 => $func::<29>($($arg),*),
            30 => $func::<30>($($arg),*),
            31 => $func::<31>($($arg),*),
            _ => unreachable!("invalid p32 exponent size"),
        }
    };
}

macro_rules! match_p64_es {
    ($es:expr, $func:ident $(, $arg:expr)* $(,)?) => {
        match $es {
            0 => $func::<0>($($arg),*), 1 => $func::<1>($($arg),*),
            2 => $func::<2>($($arg),*), 3 => $func::<3>($($arg),*),
            4 => $func::<4>($($arg),*), 5 => $func::<5>($($arg),*),
            6 => $func::<6>($($arg),*), 7 => $func::<7>($($arg),*),
            8 => $func::<8>($($arg),*), 9 => $func::<9>($($arg),*),
            10 => $func::<10>($($arg),*), 11 => $func::<11>($($arg),*),
            12 => $func::<12>($($arg),*), 13 => $func::<13>($($arg),*),
            14 => $func::<14>($($arg),*), 15 => $func::<15>($($arg),*),
            16 => $func::<16>($($arg),*), 17 => $func::<17>($($arg),*),
            18 => $func::<18>($($arg),*), 19 => $func::<19>($($arg),*),
            20 => $func::<20>($($arg),*), 21 => $func::<21>($($arg),*),
            22 => $func::<22>($($arg),*), 23 => $func::<23>($($arg),*),
            24 => $func::<24>($($arg),*), 25 => $func::<25>($($arg),*),
            26 => $func::<26>($($arg),*), 27 => $func::<27>($($arg),*),
            28 => $func::<28>($($arg),*), 29 => $func::<29>($($arg),*),
            30 => $func::<30>($($arg),*), 31 => $func::<31>($($arg),*),
            32 => $func::<32>($($arg),*), 33 => $func::<33>($($arg),*),
            34 => $func::<34>($($arg),*), 35 => $func::<35>($($arg),*),
            36 => $func::<36>($($arg),*), 37 => $func::<37>($($arg),*),
            38 => $func::<38>($($arg),*), 39 => $func::<39>($($arg),*),
            40 => $func::<40>($($arg),*), 41 => $func::<41>($($arg),*),
            42 => $func::<42>($($arg),*), 43 => $func::<43>($($arg),*),
            44 => $func::<44>($($arg),*), 45 => $func::<45>($($arg),*),
            46 => $func::<46>($($arg),*), 47 => $func::<47>($($arg),*),
            48 => $func::<48>($($arg),*), 49 => $func::<49>($($arg),*),
            50 => $func::<50>($($arg),*), 51 => $func::<51>($($arg),*),
            52 => $func::<52>($($arg),*), 53 => $func::<53>($($arg),*),
            54 => $func::<54>($($arg),*), 55 => $func::<55>($($arg),*),
            56 => $func::<56>($($arg),*), 57 => $func::<57>($($arg),*),
            58 => $func::<58>($($arg),*), 59 => $func::<59>($($arg),*),
            60 => $func::<60>($($arg),*), 61 => $func::<61>($($arg),*),
            62 => $func::<62>($($arg),*), 63 => $func::<63>($($arg),*),
            _ => unreachable!("invalid p64 exponent size"),
        }
    };
}

// ── Bump arena (D-0008) ─────────────────────────────────────────────────────────

const CHUNK_BYTES: usize = 1 << 20;

/// Default heap ceiling (2 GiB) when `ZUTAI_HEAP_MAX` is unset: generous enough
/// for any v0 fixture/spec program, low enough to abort a runaway leak cleanly
/// before the OS OOM-killer steps in.
const DEFAULT_HEAP_MAX: usize = 2 << 30;

/// A chunked bump allocator. Chunks are owned by the per-thread `ARENA` (process
/// lifetime on the single mutator thread), so returned pointers stay valid;
/// `Box<[u128]>` payloads are 16-byte aligned and keep a stable address across
/// `chunks` growth.
///
/// When `gc` is `Some` (on by default where supported; opt out with `ZUTAI_GC=0`),
/// the arena additionally tracks every live allocation in a side table and
/// reclaims unreachable objects with a conservative non-moving mark-sweep
/// collector (Phase 34). When `None` — an explicit opt-out, or a platform with no
/// stack-bounds path — the arena is pure bump with no per-alloc tracking
/// (leak-by-default).
struct Arena {
    chunks: Vec<Box<[u128]>>,
    /// Byte offset into the last chunk.
    off: usize,
    /// Total bytes committed across all chunks; never exceeds `cap`.
    committed: usize,
    /// Maximum committed bytes; `usize::MAX` means unlimited.
    cap: usize,
    /// Collector state; `None` only when opted out (`ZUTAI_GC=0`) or unsupported.
    gc: Option<Gc>,
}

impl Arena {
    fn with_cap(cap: usize) -> Self {
        let mode = gc_mode();
        Arena {
            chunks: Vec::new(),
            off: 0,
            committed: 0,
            cap,
            gc: mode.enabled.then(|| Gc::new(mode.stress)),
        }
    }

    /// A leak-by-default arena (collector disabled) for white-box cap/accounting
    /// tests that exercise the pure-bump substrate deterministically, independent
    /// of the process-wide default-on collector (`gc_mode()`).
    #[cfg(test)]
    fn with_cap_leak(cap: usize) -> Self {
        Arena {
            chunks: Vec::new(),
            off: 0,
            committed: 0,
            cap,
            gc: None,
        }
    }

    /// Allocate `bytes` (16-byte aligned). Returns `None` only when, after a
    /// collection if the collector is enabled, growing the arena would push
    /// committed memory past `cap`; the caller turns that into a runtime
    /// diagnostic.
    ///
    /// With the collector enabled, allocation prefers reuse: stress-collect (if
    /// requested), then satisfy from the free list, then bump the current chunk,
    /// then collect-and-retry under memory pressure, and only grow a new chunk as
    /// a last resort. Every handed-out span is recorded in the object table so a
    /// later collection can reclaim it.
    fn try_alloc(&mut self, bytes: usize) -> Option<*mut u8> {
        let bytes = (bytes + 15) & !15; // 16-byte alignment
        let gc_on = self.gc.is_some();

        if gc_on {
            if self.gc.as_ref().unwrap().stress {
                self.collect_garbage();
            }
            if let Some(p) = self.freelist_alloc(bytes) {
                return Some(p);
            }
        }
        if let Some(p) = self.bump(bytes) {
            self.register(p, bytes);
            return Some(p);
        }
        if gc_on {
            // Memory pressure: collect, then retry reuse and bump before growing.
            self.collect_garbage();
            if let Some(p) = self.freelist_alloc(bytes) {
                return Some(p);
            }
            if let Some(p) = self.bump(bytes) {
                self.register(p, bytes);
                return Some(p);
            }
        }
        let p = self.grow_and_bump(bytes)?;
        self.register(p, bytes);
        Some(p)
    }

    /// Bump within the current chunk; `None` if there is no chunk or it is full.
    fn bump(&mut self, bytes: usize) -> Option<*mut u8> {
        let (base, len) = {
            let chunk = self.chunks.last()?;
            (chunk.as_ptr() as usize, chunk.len() * 16)
        };
        if self.off + bytes > len {
            return None;
        }
        let ptr = (base + self.off) as *mut u8;
        self.off += bytes;
        Some(ptr)
    }

    /// Commit a fresh chunk (clamped to the cap's remaining budget) and bump from
    /// it; `None` if the cap leaves no room.
    fn grow_and_bump(&mut self, bytes: usize) -> Option<*mut u8> {
        let remaining = self.cap.saturating_sub(self.committed);
        if bytes > remaining {
            return None; // cap reached
        }
        // `bytes` is 16-aligned and <= remaining, so masking keeps size >= bytes.
        let size = (bytes.max(CHUNK_BYTES).min(remaining)) & !15;
        self.chunks.push(vec![0u128; size / 16].into_boxed_slice());
        self.committed += size;
        self.off = 0;
        self.bump(bytes)
    }

    /// Satisfy `bytes` from the free list (first fit, splitting the remainder),
    /// recording the reused span in the object table. `None` if no fit or no GC.
    fn freelist_alloc(&mut self, bytes: usize) -> Option<*mut u8> {
        let gc = self.gc.as_mut()?;
        let start = freelist_take(&mut gc.free, bytes)?;
        gc.objects.insert(start, bytes);
        Some(start as *mut u8)
    }

    /// Record a freshly bumped/grown span in the object table (no-op without GC).
    fn register(&mut self, p: *mut u8, bytes: usize) {
        if let Some(gc) = &mut self.gc {
            gc.objects.insert(p as usize, bytes);
        }
    }

    /// Address ranges `[lo, hi)` of every committed chunk.
    fn chunk_bounds(&self) -> Vec<(usize, usize)> {
        self.chunks
            .iter()
            .map(|c| {
                let lo = c.as_ptr() as usize;
                (lo, lo + c.len() * 16)
            })
            .collect()
    }

    /// One conservative non-moving mark-sweep cycle. Roots are found by scanning
    /// the machine stack (callee-saved registers flushed via `setjmp`), every
    /// stack word treated as a candidate pointer; reachable objects are traced by
    /// scanning their words the same way. Unreachable objects return to the free
    /// list. Safe to call only at an allocation point (synchronous from the
    /// mutator), which is the only place it runs.
    ///
    /// If the stack bounds cannot be established, the cycle is abandoned *before
    /// sweeping* — the safe direction is to retain (leak) rather than risk
    /// freeing a live object.
    fn collect_garbage(&mut self) {
        if self.gc.is_none() {
            return;
        }
        let bounds = self.chunk_bounds();
        let mut marked: HashSet<usize> = HashSet::new();
        let mut work: Vec<usize> = Vec::new();

        {
            let objects = &self.gc.as_ref().unwrap().objects;
            if !scan_stack_roots(objects, &bounds, &mut marked, &mut work) {
                return; // bounds unknown: do not sweep
            }
            while let Some(start) = work.pop() {
                let Some(&size) = objects.get(&start) else {
                    continue;
                };
                let mut p = start;
                let end = start + size;
                while p + 8 <= end {
                    // SAFETY: `[start, start+size)` is live arena memory backed by
                    // the zero-initialised chunk and only ever written, so the read
                    // is in-bounds and initialised; `read_volatile` keeps the
                    // optimiser from reasoning about the conservative scan.
                    let w = unsafe { std::ptr::read_volatile(p as *const usize) };
                    mark_candidate(w, objects, &bounds, &mut marked, &mut work);
                    p += 8;
                }
            }
        }

        let gc = self.gc.as_mut().unwrap();
        let mut freed: Vec<(usize, usize)> = Vec::new();
        gc.objects.retain(|&start, &mut size| {
            let live = marked.contains(&start);
            if !live {
                freed.push((start, size));
            }
            live
        });
        let reclaimed_objects = freed.len();
        let reclaimed_bytes: usize = freed.iter().map(|&(_, s)| s).sum();
        gc.free.extend(freed);
        coalesce_free(&mut gc.free);

        use Ordering::Relaxed;
        GC_COLLECTIONS.fetch_add(1, Relaxed);
        GC_RECLAIMED_BYTES.fetch_add(reclaimed_bytes, Relaxed);
        GC_RECLAIMED_OBJECTS.fetch_add(reclaimed_objects, Relaxed);
    }
}

// ── Conservative mark-sweep collector (Phase 34, on by default; opt out ZUTAI_GC=0) ─

/// Per-arena collector state (present unless opted out via `ZUTAI_GC=0` or the
/// platform lacks a stack-bounds path).
struct Gc {
    /// Live allocations: object start address -> byte size. Ordered so a
    /// candidate pointer (exact or interior) resolves to its object via a range
    /// query.
    objects: BTreeMap<usize, usize>,
    /// Reclaimed spans `(start, size)` available for reuse (first fit).
    free: Vec<(usize, usize)>,
    /// Collect before every allocation (`ZUTAI_GC_STRESS`) — soundness torture.
    stress: bool,
}

impl Gc {
    fn new(stress: bool) -> Self {
        Gc {
            objects: BTreeMap::new(),
            free: Vec::new(),
            stress,
        }
    }
}

/// Resolved `ZUTAI_GC` configuration.
#[derive(Clone, Copy)]
struct GcMode {
    enabled: bool,
    stress: bool,
}

/// Whether an env var holds a truthy value (`1`/`true`/`yes`/`on`).
fn env_truthy(var: &str) -> bool {
    std::env::var(var)
        .map(|v| {
            let v = v.trim();
            v == "1"
                || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("yes")
                || v.eq_ignore_ascii_case("on")
        })
        .unwrap_or(false)
}

/// Whether an env var is explicitly set to a falsy value (`0`/`false`/`no`/`off`).
/// Distinct from "unset": only an explicit opt-out turns the default-on collector
/// back to leak-by-default.
fn env_falsy(var: &str) -> bool {
    std::env::var(var)
        .map(|v| {
            let v = v.trim();
            v == "0"
                || v.eq_ignore_ascii_case("false")
                || v.eq_ignore_ascii_case("no")
                || v.eq_ignore_ascii_case("off")
        })
        .unwrap_or(false)
}

/// Read the collector mode once per process. The collector is **on by default**
/// wherever the conservative stack scan can establish the stack bounds; an
/// explicit `ZUTAI_GC=0` (or `false`/`no`/`off`), or a platform with no
/// stack-bounds path (`stack_base()` is `None`), returns to leak-by-default.
/// `ZUTAI_GC_STRESS` additionally forces a collection before every allocation and
/// keeps the collector on even past an explicit opt-out.
fn gc_mode() -> GcMode {
    static MODE: OnceLock<GcMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let stress = env_truthy("ZUTAI_GC_STRESS");
        let enabled = (stress || !env_falsy("ZUTAI_GC")) && stack_base().is_some();
        GcMode { enabled, stress }
    })
}

/// First-fit allocation from the free list, splitting the chosen span and
/// leaving any remainder. Returns the start of the `bytes`-sized prefix.
fn freelist_take(free: &mut Vec<(usize, usize)>, bytes: usize) -> Option<usize> {
    let idx = free.iter().position(|&(_, size)| size >= bytes)?;
    let (start, size) = free[idx];
    if size == bytes {
        free.swap_remove(idx);
    } else {
        free[idx] = (start + bytes, size - bytes);
    }
    Some(start)
}

/// Merge adjacent free spans in place. Sorting by start then merging means only
/// spans truly contiguous in memory coalesce; spans in different chunks have an
/// address gap and stay separate.
fn coalesce_free(free: &mut Vec<(usize, usize)>) {
    if free.len() < 2 {
        return;
    }
    free.sort_unstable_by_key(|&(start, _)| start);
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(free.len());
    for &(start, size) in free.iter() {
        if let Some(last) = merged.last_mut()
            && last.0 + last.1 == start
        {
            last.1 += size;
            continue;
        }
        merged.push((start, size));
    }
    *free = merged;
}

/// Resolve a candidate pointer to the live object that contains it (exact or
/// interior), or `None` if it points to no live object.
fn find_object(objects: &BTreeMap<usize, usize>, ptr: usize) -> Option<usize> {
    let (&start, &size) = objects.range(..=ptr).next_back()?;
    (ptr < start + size).then_some(start)
}

/// Whether `p` falls inside any committed chunk — a cheap reject before the
/// object-table lookup.
fn ptr_in_chunks(bounds: &[(usize, usize)], p: usize) -> bool {
    bounds.iter().any(|&(lo, hi)| p >= lo && p < hi)
}

/// If `w` points into a live object, mark that object and enqueue it for tracing.
fn mark_candidate(
    w: usize,
    objects: &BTreeMap<usize, usize>,
    bounds: &[(usize, usize)],
    marked: &mut HashSet<usize>,
    work: &mut Vec<usize>,
) {
    if ptr_in_chunks(bounds, w)
        && let Some(start) = find_object(objects, w)
        && marked.insert(start)
    {
        work.push(start);
    }
}

/// `jmp_buf` is far smaller than this on every supported target; oversizing the
/// buffer makes the `setjmp` register spill safe without per-platform sizes.
const JMPBUF_WORDS: usize = 256;

unsafe extern "C" {
    /// Saves callee-saved registers into `env`. We never `longjmp`, so this is
    /// purely a register spill the conservative scan reads back off the stack.
    ///
    /// LOAD-BEARING for soundness: a live arena pointer can sit in a callee-saved
    /// GP register across an allocation call (caller-saved registers cannot hold a
    /// value across a call per the ABI), so the scan can only see such a root if
    /// `setjmp` spills *every* callee-saved GP register into `env`. That holds on
    /// the targets the collector activates on: AArch64 (x19–x28) and x86-64 SysV
    /// (rbx, rbp, r12–r15) — both Apple's and glibc's `setjmp` save the full
    /// callee-saved GP set. One caveat verified empirically: the macOS SDK
    /// `setjmp.h` comment lists "r21-r29" (omitting x19/x20), but that text is
    /// stale — Apple's `setjmp` does save x19/x20 (confirmed by spilling sentinels
    /// through it; the slot count has room for x19–x30). If a future libc narrowed
    /// this, roots in those registers could be missed — re-verify on a new target.
    fn setjmp(env: *mut u8) -> i32;
}

/// Highest address (base) of the current thread's stack, or `None` if it cannot
/// be determined. macOS exposes it directly; Linux reports the low address plus a
/// size, so the base is their sum. Other targets have no wired-up path and keep
/// the collector off (leak-by-default) regardless of `ZUTAI_GC`.
#[cfg(target_os = "macos")]
fn stack_base() -> Option<usize> {
    unsafe extern "C" {
        fn pthread_self() -> *mut u8;
        fn pthread_get_stackaddr_np(thread: *mut u8) -> *mut u8;
    }
    // SAFETY: both are libc calls on the current thread with no preconditions.
    // macOS returns the highest address (stack base) directly.
    let base = unsafe { pthread_get_stackaddr_np(pthread_self()) } as usize;
    (base != 0).then_some(base)
}

#[cfg(target_os = "linux")]
fn stack_base() -> Option<usize> {
    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_getattr_np(thread: usize, attr: *mut u8) -> i32;
        fn pthread_attr_getstack(
            attr: *const u8,
            stackaddr: *mut *mut u8,
            stacksize: *mut usize,
        ) -> i32;
        fn pthread_attr_destroy(attr: *mut u8) -> i32;
    }
    // `pthread_attr_t` is opaque; glibc/musl keep it well under 64 bytes on LP64.
    // Over-size and 16-align the buffer so the libc writes land safely.
    #[repr(align(16))]
    struct AttrBuf([u8; 64]);
    let mut attr = AttrBuf([0u8; 64]);
    let mut stackaddr: *mut u8 = std::ptr::null_mut();
    let mut stacksize: usize = 0;
    // SAFETY: libc calls on the current thread; `attr` is sized/aligned for
    // `pthread_attr_t` and is initialised by `pthread_getattr_np` before
    // `pthread_attr_getstack` reads it, then destroyed.
    unsafe {
        if pthread_getattr_np(pthread_self(), attr.0.as_mut_ptr()) != 0 {
            return None;
        }
        let rc = pthread_attr_getstack(attr.0.as_ptr(), &mut stackaddr, &mut stacksize);
        pthread_attr_destroy(attr.0.as_mut_ptr());
        if rc != 0 {
            return None;
        }
    }
    // Linux reports the lowest address + size; the base is the top of the region.
    (!stackaddr.is_null() && stacksize != 0).then(|| stackaddr as usize + stacksize)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn stack_base() -> Option<usize> {
    None
}

/// Flush callee-saved registers, then conservatively scan the active stack
/// `[sp, base)` for candidate pointers. Returns `false` if the stack bounds are
/// unknown, so the caller skips a cycle it could not complete safely.
fn scan_stack_roots(
    objects: &BTreeMap<usize, usize>,
    bounds: &[(usize, usize)],
    marked: &mut HashSet<usize>,
    work: &mut Vec<usize>,
) -> bool {
    let mut jb = [0usize; JMPBUF_WORDS];
    // SAFETY: `jb` is far larger than any jmp_buf; setjmp only writes registers.
    unsafe { setjmp(jb.as_mut_ptr() as *mut u8) };
    std::hint::black_box(&jb);

    let Some(base) = stack_base() else {
        return false;
    };
    // `jb` lives in this (deepest) frame, so scanning from its address up to the
    // stack base covers the saved registers plus every caller frame — every place
    // a live arena pointer can be at an allocation point.
    let sp = jb.as_ptr() as usize;
    if base <= sp {
        return false;
    }
    let mut p = sp;
    while p + 8 <= base {
        // SAFETY: `[sp, base)` is mapped, active stack memory; `read_volatile`
        // tolerates the uninitialised slots a conservative scan inevitably reads.
        let w = unsafe { std::ptr::read_volatile(p as *const usize) };
        mark_candidate(w, objects, bounds, marked, work);
        p += 8;
    }
    true
}

// GC counters mirrored to process statics (the thread-local arena may be gone by
// the time the `atexit` dump runs); reported alongside the heap stats.
static GC_COLLECTIONS: AtomicUsize = AtomicUsize::new(0);
static GC_RECLAIMED_BYTES: AtomicUsize = AtomicUsize::new(0);
static GC_RECLAIMED_OBJECTS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Per-thread bump arena. v0 mutators are single-threaded, so a compiled
    /// program uses one arena on the main thread; isolating per-thread keeps the
    /// unsynchronized `RefCell` sound under parallel `cargo test`.
    static ARENA: RefCell<Arena> = RefCell::new(Arena::with_cap(heap_cap()));
}

/// Resolve the heap ceiling once per process from `ZUTAI_HEAP_MAX`.
fn heap_cap() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("ZUTAI_HEAP_MAX")
            .ok()
            .and_then(|s| parse_cap_bytes(&s))
            .unwrap_or(DEFAULT_HEAP_MAX)
    })
}

/// Parse a `ZUTAI_HEAP_MAX` value: a byte count with an optional binary suffix
/// (`k`/`kib`, `m`/`mib`, `g`/`gib`, case-insensitive). `0`, `unlimited`, and
/// `none` mean no limit. Returns `None` on a malformed value (caller falls back
/// to the default).
fn parse_cap_bytes(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("unlimited") || s.eq_ignore_ascii_case("none") {
        return Some(usize::MAX);
    }
    let lower = s.to_ascii_lowercase();
    let (num, mult) = if let Some(n) = lower
        .strip_suffix("gib")
        .or_else(|| lower.strip_suffix('g'))
    {
        (n, 1usize << 30)
    } else if let Some(n) = lower
        .strip_suffix("mib")
        .or_else(|| lower.strip_suffix('m'))
    {
        (n, 1usize << 20)
    } else if let Some(n) = lower
        .strip_suffix("kib")
        .or_else(|| lower.strip_suffix('k'))
    {
        (n, 1usize << 10)
    } else {
        (lower.as_str(), 1usize)
    };
    let value: usize = num.trim().parse().ok()?;
    if value == 0 {
        return Some(usize::MAX); // 0 = unlimited
    }
    value.checked_mul(mult)
}

/// Abort with an actionable diagnostic when the arena hits its ceiling.
fn heap_limit_exceeded(bytes: usize, cap: usize) -> ! {
    eprintln!(
        "zutai runtime error: heap limit exceeded (cap {cap} bytes; cannot allocate {bytes} more). \
Raise it with ZUTAI_HEAP_MAX (e.g. ZUTAI_HEAP_MAX=4G), or ZUTAI_HEAP_MAX=0 for unlimited."
    );
    std::process::exit(1);
}

fn arena_alloc(bytes: usize) -> *mut u8 {
    let outcome =
        ARENA.with_borrow_mut(|a| a.try_alloc(bytes).map(|p| (p, a.committed)).ok_or(a.cap));
    match outcome {
        Ok((ptr, committed)) => {
            stats_record_alloc((bytes + 15) & !15, committed);
            ptr
        }
        Err(cap) => heap_limit_exceeded(bytes, cap),
    }
}

fn alloc_words(n: usize) -> *mut i64 {
    arena_alloc(n * 8).cast::<i64>()
}

// ── Heap statistics (measurement groundwork for the GC decision) ─────────────────

/// Process-global allocation counters. Updated on every allocation (a few
/// relaxed atomic adds, negligible beside the bump and header writes) so the
/// numbers are always exact; the human-readable dump at process exit is gated on
/// `ZUTAI_HEAP_STATS`. Counters aggregate across every thread's arena.
struct HeapStats {
    /// Cumulative bytes handed out (16-byte-aligned footprint).
    bytes: AtomicUsize,
    /// Cumulative object count.
    objects: AtomicUsize,
    /// High-water committed arena bytes. Commit is monotonic without a
    /// collector, so this is the final footprint — the memory a non-collecting
    /// run actually holds for an O(1)-live program.
    peak_committed: AtomicUsize,
    /// Per-kind object counts, indexed by `TAG_*`; index 0 is unused.
    by_tag: [AtomicUsize; 8],
}

static STATS: HeapStats = HeapStats {
    bytes: AtomicUsize::new(0),
    objects: AtomicUsize::new(0),
    peak_committed: AtomicUsize::new(0),
    by_tag: [const { AtomicUsize::new(0) }; 8],
};

/// Whether to print the exit-time dump; resolved once from `ZUTAI_HEAP_STATS`
/// (`1`/`true`/`yes`/`on`).
fn stats_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("ZUTAI_HEAP_STATS")
            .map(|v| {
                let v = v.trim();
                v == "1"
                    || v.eq_ignore_ascii_case("true")
                    || v.eq_ignore_ascii_case("yes")
                    || v.eq_ignore_ascii_case("on")
            })
            .unwrap_or(false)
    })
}

unsafe extern "C" {
    /// C runtime; runs registered callbacks on normal `main` return or `exit`.
    fn atexit(cb: extern "C" fn()) -> i32;
}

/// Record one allocation (`bytes` = aligned footprint, `committed` = the arena's
/// new committed total). Registers the exit dump on first use when enabled.
fn stats_record_alloc(bytes: usize, committed: usize) {
    use Ordering::Relaxed;
    STATS.bytes.fetch_add(bytes, Relaxed);
    STATS.objects.fetch_add(1, Relaxed);
    STATS.peak_committed.fetch_max(committed, Relaxed);

    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        if stats_enabled() {
            // SAFETY: `atexit` comes from the linked C runtime; the callback only
            // reads atomics and writes stderr.
            unsafe { atexit(dump_heap_stats) };
        }
    });
}

/// Bump the per-kind counter for a freshly allocated object.
fn note_tag(tag: i64) {
    STATS.by_tag[tag as usize].fetch_add(1, Ordering::Relaxed);
}

/// Render the one-line stats report. Pure (testable) given a counter snapshot;
/// `by_tag` is indexed by `TAG_*`. Objects not covered by a tracked tag
/// (closures, raw `Text` byte buffers) fall into `closure/raw`.
fn format_stats_line(
    objects: usize,
    bytes: usize,
    peak: usize,
    by_tag: &[usize; 8],
    cap: usize,
) -> String {
    let avg = bytes.checked_div(objects).unwrap_or(0);
    let typed = by_tag[TAG_RECORD as usize]
        + by_tag[TAG_TUPLE as usize]
        + by_tag[TAG_CONS as usize]
        + by_tag[TAG_VARIANT as usize]
        + by_tag[TAG_TEXT as usize];
    let raw = objects.saturating_sub(typed);
    let cap_s = if cap == usize::MAX {
        "unlimited".to_string()
    } else {
        cap.to_string()
    };
    format!(
        "zutai heap stats: allocated {bytes} bytes in {objects} objects (avg {avg} B); \
peak committed {peak} bytes (cap {cap_s}). by kind: record {}, tuple {}, cons {}, variant {}, text {}, closure/raw {}.",
        by_tag[TAG_RECORD as usize],
        by_tag[TAG_TUPLE as usize],
        by_tag[TAG_CONS as usize],
        by_tag[TAG_VARIANT as usize],
        by_tag[TAG_TEXT as usize],
        raw,
    )
}

/// Exit-time dump to stderr, gated on `ZUTAI_HEAP_STATS`.
extern "C" fn dump_heap_stats() {
    use Ordering::Relaxed;
    if !stats_enabled() {
        return;
    }
    let by_tag = std::array::from_fn(|i| STATS.by_tag[i].load(Relaxed));
    let line = format_stats_line(
        STATS.objects.load(Relaxed),
        STATS.bytes.load(Relaxed),
        STATS.peak_committed.load(Relaxed),
        &by_tag,
        heap_cap(),
    );
    eprintln!("{line}");

    let collections = GC_COLLECTIONS.load(Relaxed);
    if collections > 0 {
        eprintln!(
            "zutai gc stats: {collections} collections, reclaimed {} bytes / {} objects.",
            GC_RECLAIMED_BYTES.load(Relaxed),
            GC_RECLAIMED_OBJECTS.load(Relaxed),
        );
    }
}

// ── Raw word access ─────────────────────────────────────────────────────────────

/// # Safety
/// `p` must be a valid object pointer with at least `i + 1` words.
unsafe fn word(p: i64, i: usize) -> i64 {
    unsafe { *(p as *const i64).add(i) }
}

/// # Safety
/// `p` must be a valid object pointer with at least `i + 1` writable words.
unsafe fn set_word(p: i64, i: usize, v: i64) {
    unsafe { *(p as *mut i64).add(i) = v }
}

/// # Safety
/// `t` must point to a `ZtText` object.
unsafe fn text_parts<'a>(t: i64) -> &'a [u8] {
    unsafe {
        let len = word(t, 1) as usize;
        let ptr = word(t, 2) as *const u8;
        slice::from_raw_parts(ptr, len)
    }
}

// ── Allocation ABI ──────────────────────────────────────────────────────────────

#[unsafe(export_name = "zutai.alloc")]
pub extern "C" fn alloc(nbytes: i64) -> i64 {
    arena_alloc(nbytes as usize) as i64
}

#[unsafe(export_name = "zutai.free")]
pub extern "C" fn free(_p: i64) {
    // No-op in v0; reclamation is collector-owned and arena chunks are released at process exit.
}

// ── Record ABI (D-0004, ordinal slots) ──────────────────────────────────────────

#[unsafe(export_name = "zutai.record_new")]
pub extern "C" fn record_new(n: i64) -> i64 {
    let p = alloc_words(1 + n as usize);
    unsafe { *p = header(TAG_RECORD, n as u64) };
    note_tag(TAG_RECORD);
    p as i64
}

#[unsafe(export_name = "zutai.record_set")]
pub extern "C" fn record_set(r: i64, slot: i64, v: i64) {
    unsafe { set_word(r, 1 + slot as usize, v) };
}

#[unsafe(export_name = "zutai.record_get")]
pub extern "C" fn record_get(r: i64, slot: i64) -> i64 {
    unsafe { word(r, 1 + slot as usize) }
}

#[unsafe(export_name = "zutai.record_update")]
pub extern "C" fn record_update(r: i64, slot: i64, v: i64) -> i64 {
    unsafe {
        let n = header_count(word(r, 0)) as usize;
        let p = alloc_words(1 + n);
        for i in 0..=n {
            *p.add(i) = word(r, i);
        }
        let out = p as i64;
        set_word(out, 1 + slot as usize, v);
        note_tag(TAG_RECORD);
        out
    }
}

// ── Tuple ABI (positional/named slots) ──────────────────────────────────────────

#[unsafe(export_name = "zutai.tuple_new")]
pub extern "C" fn tuple_new(n: i64) -> i64 {
    let p = alloc_words(1 + n as usize);
    unsafe { *p = header(TAG_TUPLE, n as u64) };
    note_tag(TAG_TUPLE);
    p as i64
}

#[unsafe(export_name = "zutai.tuple_set")]
pub extern "C" fn tuple_set(t: i64, slot: i64, v: i64) {
    unsafe { set_word(t, 1 + slot as usize, v) };
}

#[unsafe(export_name = "zutai.tuple_get")]
pub extern "C" fn tuple_get(t: i64, slot: i64) -> i64 {
    unsafe { word(t, 1 + slot as usize) }
}

// ── List ABI (D-0005) ───────────────────────────────────────────────────────────

/// The nil sentinel is a process-static one-word object (just a `TAG_NIL`
/// header). Keeping it out of the per-thread arena means `list_nil()` returns a
/// pointer valid on every thread and for the whole process lifetime.
#[repr(align(16))]
// The header word is read through the raw pointer in `coalesce`/`render`, never
// via the field, so the compiler cannot see the use.
#[allow(dead_code)]
struct NilObj(i64);

static NIL_OBJ: NilObj = NilObj(header(TAG_NIL, 0));

#[unsafe(export_name = "zutai.list_nil")]
pub extern "C" fn list_nil() -> i64 {
    (&raw const NIL_OBJ) as i64
}

#[unsafe(export_name = "zutai.list_cons")]
pub extern "C" fn list_cons(head: i64, tail: i64) -> i64 {
    let p = alloc_words(3);
    unsafe {
        *p = header(TAG_CONS, 0);
        *p.add(1) = head;
        *p.add(2) = tail;
    }
    note_tag(TAG_CONS);
    p as i64
}

#[unsafe(export_name = "zutai.list_append")]
pub extern "C" fn list_append(xs: i64, ys: i64) -> i64 {
    let mut heads = Vec::new();
    let mut cur = xs;
    while list_is_nil(cur) == 0 {
        heads.push(list_head(cur));
        cur = list_tail(cur);
    }
    let mut out = ys;
    for head in heads.into_iter().rev() {
        out = list_cons(head, out);
    }
    out
}

/// `1` (Bool true) for the nil sentinel, `0` for a cons cell. Backs the
/// `listIsNil` bridge primitive; the result is a plain untagged-i64 Bool.
#[unsafe(export_name = "zutai.list_is_nil")]
pub extern "C" fn list_is_nil(v: i64) -> i64 {
    unsafe { (header_tag(word(v, 0)) == TAG_NIL) as i64 }
}

/// First element of a cons cell. Undefined on nil (the `.zt` `fromList` guards
/// this with `listIsNil`). Backs the `listHead` bridge primitive.
#[unsafe(export_name = "zutai.list_head")]
pub extern "C" fn list_head(v: i64) -> i64 {
    unsafe { word(v, 1) }
}

/// Tail of a cons cell. Undefined on nil (guarded by `listIsNil`). Backs the
/// `listTail` bridge primitive.
#[unsafe(export_name = "zutai.list_tail")]
pub extern "C" fn list_tail(v: i64) -> i64 {
    unsafe { word(v, 2) }
}

#[unsafe(export_name = "zutai.list_foldl_strict")]
pub extern "C" fn list_foldl_strict(f: i64, mut acc: i64, mut xs: i64) -> i64 {
    type ClosureCode = extern "C" fn(i64, i64) -> i64;
    while list_is_nil(xs) == 0 {
        let elem = list_head(xs);
        let step = unsafe {
            let code: ClosureCode = std::mem::transmute(word(f, 1));
            code(f, acc)
        };
        acc = unsafe {
            let code: ClosureCode = std::mem::transmute(word(step, 1));
            code(step, elem)
        };
        xs = list_tail(xs);
    }
    acc
}

// ── Numeric bridge ABI ─────────────────────────────────────────────────────────

fn numeric_runtime_error(message: &str) -> ! {
    eprintln!("zutai runtime error: {message}");
    std::process::exit(1);
}

fn int_from_float(value: f64, range_message: &'static str) -> i64 {
    const INT_MIN_INCLUSIVE: f64 = -9_223_372_036_854_775_808.0;
    const INT_MAX_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;
    if !(INT_MIN_INCLUSIVE..INT_MAX_EXCLUSIVE).contains(&value) {
        numeric_runtime_error(range_message);
    }
    value as i64
}

#[unsafe(export_name = "zutai.num_abs")]
pub extern "C" fn num_abs(value: i64) -> i64 {
    value
        .checked_abs()
        .unwrap_or_else(|| numeric_runtime_error("integer overflow in `abs`"))
}

#[unsafe(export_name = "zutai.num_rem")]
pub extern "C" fn num_rem(dividend: i64, divisor: i64) -> i64 {
    if divisor == 0 {
        numeric_runtime_error("integer remainder by zero");
    }
    dividend
        .checked_rem(divisor)
        .unwrap_or_else(|| numeric_runtime_error("integer overflow in `rem`"))
}

#[unsafe(export_name = "zutai.num_pow")]
pub extern "C" fn num_pow(base: i64, exponent: i64) -> i64 {
    if exponent < 0 {
        numeric_runtime_error("invalid numeric argument: pow exponent must be non-negative");
    }
    if exponent > u32::MAX as i64 {
        numeric_runtime_error("invalid numeric argument: pow exponent must fit u32");
    }
    base.checked_pow(exponent as u32)
        .unwrap_or_else(|| numeric_runtime_error("integer overflow in `pow`"))
}

#[unsafe(export_name = "zutai.num_to_float")]
pub extern "C" fn num_to_float(value: i64) -> i64 {
    (value as f64).to_bits() as i64
}

#[unsafe(export_name = "zutai.num_round")]
pub extern "C" fn num_round(value: i64) -> i64 {
    let float = f64::from_bits(value as u64);
    if !float.is_finite() {
        numeric_runtime_error("invalid numeric argument: round requires finite Float");
    }
    int_from_float(
        float.round(),
        "invalid numeric argument: round result outside Int range",
    )
}

#[unsafe(export_name = "zutai.num_truncate")]
pub extern "C" fn num_truncate(value: i64) -> i64 {
    let float = f64::from_bits(value as u64);
    if !float.is_finite() {
        numeric_runtime_error("invalid numeric argument: truncate requires finite Float");
    }
    int_from_float(
        float.trunc(),
        "invalid numeric argument: truncate result outside Int range",
    )
}

fn float_bin(lhs: i64, rhs: i64, op: impl FnOnce(f64, f64) -> f64) -> i64 {
    op(f64::from_bits(lhs as u64), f64::from_bits(rhs as u64)).to_bits() as i64
}

fn float_cmp(lhs: i64, rhs: i64, op: impl FnOnce(f64, f64) -> bool) -> i64 {
    op(f64::from_bits(lhs as u64), f64::from_bits(rhs as u64)) as i64
}

#[unsafe(export_name = "zutai.float_add")]
pub extern "C" fn float_add(lhs: i64, rhs: i64) -> i64 {
    float_bin(lhs, rhs, |a, b| a + b)
}

#[unsafe(export_name = "zutai.float_sub")]
pub extern "C" fn float_sub(lhs: i64, rhs: i64) -> i64 {
    float_bin(lhs, rhs, |a, b| a - b)
}

#[unsafe(export_name = "zutai.float_mul")]
pub extern "C" fn float_mul(lhs: i64, rhs: i64) -> i64 {
    float_bin(lhs, rhs, |a, b| a * b)
}

#[unsafe(export_name = "zutai.float_div")]
pub extern "C" fn float_div(lhs: i64, rhs: i64) -> i64 {
    float_bin(lhs, rhs, |a, b| a / b)
}

#[unsafe(export_name = "zutai.float_lt")]
pub extern "C" fn float_lt(lhs: i64, rhs: i64) -> i64 {
    float_cmp(lhs, rhs, |a, b| a < b)
}

#[unsafe(export_name = "zutai.float_le")]
pub extern "C" fn float_le(lhs: i64, rhs: i64) -> i64 {
    float_cmp(lhs, rhs, |a, b| a <= b)
}

#[unsafe(export_name = "zutai.float_gt")]
pub extern "C" fn float_gt(lhs: i64, rhs: i64) -> i64 {
    float_cmp(lhs, rhs, |a, b| a > b)
}

#[unsafe(export_name = "zutai.float_ge")]
pub extern "C" fn float_ge(lhs: i64, rhs: i64) -> i64 {
    float_cmp(lhs, rhs, |a, b| a >= b)
}

// ── Variant ABI (D-0005 / D-0009, dense indices) ────────────────────────────────

#[unsafe(export_name = "zutai.variant_new")]
pub extern "C" fn variant_new(tag: i64, payload: i64) -> i64 {
    let p = alloc_words(3);
    unsafe {
        *p = header(TAG_VARIANT, 0);
        *p.add(1) = tag;
        *p.add(2) = payload;
    }
    note_tag(TAG_VARIANT);
    p as i64
}

#[unsafe(export_name = "zutai.variant_tag")]
pub extern "C" fn variant_tag(v: i64) -> i64 {
    unsafe { word(v, 1) }
}

#[unsafe(export_name = "zutai.variant_value")]
pub extern "C" fn variant_value(v: i64) -> i64 {
    unsafe { word(v, 2) }
}

/// Unwrap one `Optional`/`Maybe` layer.
#[unsafe(export_name = "zutai.coalesce")]
pub extern "C" fn coalesce(v: i64, fallback: i64) -> i64 {
    unsafe {
        // #some (x) / #present (x) are variant_new(1, <1-tuple>) — return the
        // inner value (tuple slot 0). #none / #absent are atom text objects.
        if header_tag(word(v, 0)) == TAG_VARIANT {
            word(word(v, 2), 1)
        } else {
            fallback
        }
    }
}

// ── Text ABI (D-0006, pointer to UTF-8 bytes) ───────────────────────────────────

#[unsafe(export_name = "zutai.text_from_global")]
pub extern "C" fn text_from_global(ptr: i64, len: i64) -> i64 {
    let p = alloc_words(3);
    unsafe {
        *p = header(TAG_TEXT, 0);
        *p.add(1) = len;
        *p.add(2) = ptr;
    }
    note_tag(TAG_TEXT);
    p as i64
}

#[unsafe(export_name = "zutai.text_len")]
pub extern "C" fn text_len(value: i64) -> i64 {
    unsafe { word(value, 1) }
}

#[unsafe(no_mangle)]
pub extern "C" fn zutai_text_len(value: i64) -> i64 {
    text_len(value)
}

#[unsafe(export_name = "zutai.text_ptr")]
pub extern "C" fn text_ptr(value: i64) -> i64 {
    unsafe { word(value, 2) }
}

#[unsafe(no_mangle)]
pub extern "C" fn zutai_text_ptr(value: i64) -> i64 {
    text_ptr(value)
}

/// Build a free-standing atom value. Atoms carry their source spelling for
/// `show`; dense variant tags remain raw integer indices.
#[unsafe(export_name = "zutai.atom_from_global")]
pub extern "C" fn atom_from_global(ptr: i64, len: i64) -> i64 {
    text_from_global(ptr, len)
}

#[unsafe(export_name = "zutai.text_concat")]
pub extern "C" fn text_concat(a: i64, b: i64) -> i64 {
    unsafe {
        let sa = text_parts(a);
        let sb = text_parts(b);
        let total = sa.len() + sb.len();
        let dst = arena_alloc(total);
        std::ptr::copy_nonoverlapping(sa.as_ptr(), dst, sa.len());
        std::ptr::copy_nonoverlapping(sb.as_ptr(), dst.add(sa.len()), sb.len());
        text_from_global(dst as i64, total as i64)
    }
}

#[unsafe(export_name = "zutai.text_eq")]
pub extern "C" fn text_eq(a: i64, b: i64) -> i64 {
    unsafe { (text_parts(a) == text_parts(b)) as i64 }
}

#[unsafe(export_name = "zutai.text_ne")]
pub extern "C" fn text_ne(a: i64, b: i64) -> i64 {
    unsafe { (text_parts(a) != text_parts(b)) as i64 }
}

#[unsafe(export_name = "zutai.text_lt")]
pub extern "C" fn text_lt(a: i64, b: i64) -> i64 {
    unsafe { (text_parts(a) < text_parts(b)) as i64 }
}

#[unsafe(export_name = "zutai.text_le")]
pub extern "C" fn text_le(a: i64, b: i64) -> i64 {
    unsafe { (text_parts(a) <= text_parts(b)) as i64 }
}

#[unsafe(export_name = "zutai.text_gt")]
pub extern "C" fn text_gt(a: i64, b: i64) -> i64 {
    unsafe { (text_parts(a) > text_parts(b)) as i64 }
}

#[unsafe(export_name = "zutai.text_ge")]
pub extern "C" fn text_ge(a: i64, b: i64) -> i64 {
    unsafe { (text_parts(a) >= text_parts(b)) as i64 }
}

#[unsafe(export_name = "zutai.text_length")]
pub extern "C" fn text_length(value: i64) -> i64 {
    unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.length input is not UTF-8"))
            .chars()
            .count() as i64
    }
}

#[unsafe(export_name = "zutai.text_split")]
pub extern "C" fn text_split(separator: i64, value: i64) -> i64 {
    let separator = unsafe {
        str::from_utf8(text_parts(separator))
            .unwrap_or_else(|_| runtime_error("text.split separator is not UTF-8"))
    };
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.split input is not UTF-8"))
    };
    let parts: Vec<&str> = value.split(separator).collect();
    let mut out = list_nil();
    for part in parts.into_iter().rev() {
        out = list_cons(text_from_bytes(part.as_bytes()), out);
    }
    out
}

#[unsafe(export_name = "zutai.text_join")]
pub extern "C" fn text_join(separator: i64, values: i64) -> i64 {
    let separator = unsafe {
        str::from_utf8(text_parts(separator))
            .unwrap_or_else(|_| runtime_error("text.join separator is not UTF-8"))
    };
    let mut parts = Vec::new();
    let mut xs = values;
    while list_is_nil(xs) == 0 {
        let item = list_head(xs);
        let part = unsafe {
            str::from_utf8(text_parts(item))
                .unwrap_or_else(|_| runtime_error("text.join item is not UTF-8"))
        };
        parts.push(part.to_string());
        xs = list_tail(xs);
    }
    text_from_string(parts.join(separator))
}

#[unsafe(export_name = "zutai.text_trim")]
pub extern "C" fn text_trim(value: i64) -> i64 {
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.trim input is not UTF-8"))
    };
    text_from_bytes(value.trim().as_bytes())
}

#[unsafe(export_name = "zutai.text_to_upper")]
pub extern "C" fn text_to_upper(value: i64) -> i64 {
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.toUpper input is not UTF-8"))
    };
    text_from_string(value.to_uppercase())
}

#[unsafe(export_name = "zutai.text_to_lower")]
pub extern "C" fn text_to_lower(value: i64) -> i64 {
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.toLower input is not UTF-8"))
    };
    text_from_string(value.to_lowercase())
}

#[unsafe(export_name = "zutai.text_contains")]
pub extern "C" fn text_contains(needle: i64, value: i64) -> i64 {
    let needle = unsafe {
        str::from_utf8(text_parts(needle))
            .unwrap_or_else(|_| runtime_error("text.contains needle is not UTF-8"))
    };
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.contains input is not UTF-8"))
    };
    value.contains(needle) as i64
}

#[unsafe(export_name = "zutai.text_replace")]
pub extern "C" fn text_replace(from: i64, to: i64, value: i64) -> i64 {
    let from = unsafe {
        str::from_utf8(text_parts(from))
            .unwrap_or_else(|_| runtime_error("text.replace from is not UTF-8"))
    };
    let to = unsafe {
        str::from_utf8(text_parts(to))
            .unwrap_or_else(|_| runtime_error("text.replace to is not UTF-8"))
    };
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.replace input is not UTF-8"))
    };
    text_from_string(value.replace(from, to))
}

fn quoted_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[unsafe(export_name = "zutai.text_show")]
pub extern "C" fn text_show(value: i64) -> i64 {
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.show input is not UTF-8"))
    };
    text_from_string(quoted_text(value))
}

fn optional_runtime_value(value: Option<i64>) -> i64 {
    match value {
        Some(value) => {
            let tuple = tuple_new(1);
            tuple_set(tuple, 0, value);
            variant_new(1, tuple)
        }
        None => text_from_bytes(b"none"),
    }
}

#[unsafe(export_name = "zutai.text_parse_int")]
pub extern "C" fn text_parse_int(value: i64) -> i64 {
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.parseInt input is not UTF-8"))
    };
    optional_runtime_value(value.trim().parse::<i64>().ok())
}

#[unsafe(export_name = "zutai.text_parse_float")]
pub extern "C" fn text_parse_float(value: i64) -> i64 {
    let value = unsafe {
        str::from_utf8(text_parts(value))
            .unwrap_or_else(|_| runtime_error("text.parseFloat input is not UTF-8"))
    };
    let parsed = value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value.to_bits() as i64);
    optional_runtime_value(parsed)
}

fn text_from_bytes(bytes: &[u8]) -> i64 {
    let dst = arena_alloc(bytes.len());
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
    }
    text_from_global(dst as i64, bytes.len() as i64)
}

fn text_from_string(s: String) -> i64 {
    text_from_bytes(s.as_bytes())
}

fn runtime_error(message: &str) -> ! {
    eprintln!("zutai host boundary error: {message}");
    std::process::exit(1);
}

fn optional_text(value: Option<String>) -> i64 {
    match value {
        Some(text) => {
            let tuple = tuple_new(1);
            tuple_set(tuple, 0, text_from_string(text));
            variant_new(1, tuple)
        }
        None => text_from_bytes(b"none"),
    }
}

// ── Host capability operations ─────────────────────────────────────────────────

#[unsafe(export_name = "zutai.host.io_print")]
pub extern "C" fn host_io_print(v: i64) -> i64 {
    print_text(v);
    let newline = text_from_bytes(b"\n");
    print_text(newline);
    v
}

#[unsafe(export_name = "zutai.host.fs_read")]
pub extern "C" fn host_fs_read(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("fs.read path is not UTF-8"))
    };
    match std::fs::read_to_string(path) {
        Ok(contents) => text_from_string(contents),
        Err(err) => runtime_error(&format!("fs.read failed for {path:?}: {err}")),
    }
}

#[unsafe(export_name = "zutai.host.load_zti")]
pub extern "C" fn host_load_zti(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("load.zti path is not UTF-8"))
    };
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|err| runtime_error(&format!("load.zti failed for {path:?}: {err}")));
    let block = zutai_im::parse(&source)
        .unwrap_or_else(|err| runtime_error(&format!("load.zti parse failed for {path:?}: {err}")));
    data_from_zti_block(&block)
}

#[unsafe(export_name = "zutai.host.load_zt")]
pub extern "C" fn host_load_zt(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("load.zt path is not UTF-8"))
    };
    match zutai_eval::eval_tlc_path(std::path::Path::new(path))
        .and_then(|value| data_from_eval_value(&value))
    {
        Ok(value) => value,
        Err(err) => runtime_error(&format!("load.zt failed for {path:?}: {err}")),
    }
}

fn data_from_zti_block(block: &zutai_im::Block) -> i64 {
    let mut fields = list_nil();
    for pair in block.iter().rev() {
        let field = data_field(&pair.field_name, data_from_zti_value(&pair.value));
        fields = list_cons(field, fields);
    }
    data_variant(6, data_record(&[("fields", fields)]))
}

fn data_from_zti_value(value: &zutai_im::Value) -> i64 {
    match value {
        zutai_im::Value::True => data_variant(0, data_record(&[("value", 1)])),
        zutai_im::Value::False => data_variant(0, data_record(&[("value", 0)])),
        zutai_im::Value::Atom(atom) => data_variant(
            4,
            data_record(&[("value", text_from_bytes(atom.as_bytes()))]),
        ),
        zutai_im::Value::String(text) => data_variant(
            3,
            data_record(&[("value", text_from_bytes(text.as_bytes()))]),
        ),
        zutai_im::Value::Float(value) => {
            data_variant(2, data_record(&[("value", value.to_bits() as i64)]))
        }
        zutai_im::Value::Integer(value) => data_variant(1, data_record(&[("value", *value)])),
        zutai_im::Value::Array(items) => {
            let mut out = list_nil();
            for item in items.iter().rev() {
                out = list_cons(data_from_zti_value(item), out);
            }
            data_variant(5, data_record(&[("items", out)]))
        }
        zutai_im::Value::Block(block) => data_from_zti_block(block),
    }
}

fn data_from_eval_value(value: &EvalValue) -> Result<i64, EvalError> {
    match value {
        EvalValue::Bool(value) => Ok(data_variant(
            0,
            data_record(&[("value", i64::from(*value))]),
        )),
        EvalValue::Int(value) => Ok(data_variant(1, data_record(&[("value", *value)]))),
        EvalValue::Float(value) => Ok(data_variant(
            2,
            data_record(&[("value", value.to_bits() as i64)]),
        )),
        EvalValue::Posit(literal) => Ok(data_variant(
            2,
            data_record(&[("value", (literal.bits as f64).to_bits() as i64)]),
        )),
        EvalValue::Text(value) => Ok(data_variant(
            3,
            data_record(&[("value", text_from_bytes(value.as_bytes()))]),
        )),
        EvalValue::Atom(value) => Ok(data_variant(
            4,
            data_record(&[("value", text_from_bytes(value.as_bytes()))]),
        )),
        EvalValue::List(items) => {
            let mut out = list_nil();
            for item in items.iter().rev() {
                let Some(value) = item.peek() else {
                    return Err(EvalError::Internal(
                        "load.zt result contains an unforced list item",
                    ));
                };
                out = list_cons(data_from_eval_value(&value)?, out);
            }
            Ok(data_variant(5, data_record(&[("items", out)])))
        }
        EvalValue::Tuple(items) => {
            let mut fields = list_nil();
            for (index, item) in items.iter().enumerate().rev() {
                let Some(value) = item.value.peek() else {
                    return Err(EvalError::Internal(
                        "load.zt result contains an unforced tuple item",
                    ));
                };
                let name = item
                    .name
                    .as_ref()
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| index.to_string());
                fields = list_cons(data_field(&name, data_from_eval_value(&value)?), fields);
            }
            Ok(data_variant(6, data_record(&[("fields", fields)])))
        }
        EvalValue::Record(source_fields) => {
            let mut fields = list_nil();
            for (name, thunk) in source_fields.iter().rev() {
                let Some(value) = thunk.peek() else {
                    return Err(EvalError::Internal(
                        "load.zt result contains an unforced record field",
                    ));
                };
                fields = list_cons(data_field(name, data_from_eval_value(&value)?), fields);
            }
            Ok(data_variant(6, data_record(&[("fields", fields)])))
        }
        EvalValue::TaggedValue { tag, payload } => {
            let payload = data_from_eval_value(&EvalValue::Record(payload.clone()))?;
            Ok(data_variant(
                7,
                data_record(&[
                    ("payload", payload),
                    ("tag", text_from_bytes(tag.as_bytes())),
                ]),
            ))
        }
        EvalValue::Nothing => Ok(data_variant(
            4,
            data_record(&[("value", text_from_bytes(b"absent"))]),
        )),
        EvalValue::Closure(_)
        | EvalValue::TypeValue(_)
        | EvalValue::WitnessDict(_)
        | EvalValue::TlcClosure(_)
        | EvalValue::HostHandle(_)
        | EvalValue::Builtin(_)
        | EvalValue::BuiltinPartial { .. } => Err(EvalError::EffectfulNotExecutable(
            "load.zt final value is not first-order serializable data".to_string(),
        )),
    }
}

fn data_field(name: &str, value: i64) -> i64 {
    data_record(&[("name", text_from_bytes(name.as_bytes())), ("value", value)])
}

fn data_record(fields: &[(&str, i64)]) -> i64 {
    let record = record_new(fields.len() as i64);
    for (index, (_, value)) in fields.iter().enumerate() {
        record_set(record, index as i64, *value);
    }
    record
}

fn data_variant(tag_index: i64, payload: i64) -> i64 {
    variant_new(tag_index, payload)
}

#[unsafe(export_name = "zutai.host.fs_write")]
pub extern "C" fn host_fs_write(request: i64) -> i64 {
    unsafe {
        // Dataflow/Core and SSA sort record fields by name, so the standard
        // `{ contents : Text; path : Path; }` payload stores contents at slot 0
        // and path at slot 1.
        let contents = str::from_utf8(text_parts(word(request, 1)))
            .unwrap_or_else(|_| runtime_error("fs.write contents are not UTF-8"));
        let path = str::from_utf8(text_parts(word(request, 2)))
            .unwrap_or_else(|_| runtime_error("fs.write path is not UTF-8"));
        if let Err(err) = std::fs::write(path, contents) {
            runtime_error(&format!("fs.write failed for {path:?}: {err}"));
        }
    }
    tuple_new(0)
}

// ── Scoped filesystem text handles ─────────────────────────────────────────────

static FS_READERS: Mutex<Vec<Option<BufReader<File>>>> = Mutex::new(Vec::new());
static FS_WRITERS: Mutex<Vec<Option<BufWriter<File>>>> = Mutex::new(Vec::new());
static FS_NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn fs_alloc_id() -> u64 {
    FS_NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[unsafe(export_name = "zutai.host.fs_open_read")]
pub extern "C" fn host_fs_open_read(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("fs.openRead path is not UTF-8"))
    };
    let file = File::open(path).unwrap_or_else(|err| runtime_error(&format!("fs.openRead: {err}")));
    let id = fs_alloc_id();
    let mut readers = FS_READERS.lock().unwrap();
    while readers.len() <= id as usize {
        readers.push(None);
    }
    readers[id as usize] = Some(BufReader::new(file));
    id as i64
}

#[unsafe(export_name = "zutai.host.fs_read_line")]
pub extern "C" fn host_fs_read_line(reader_id: i64) -> i64 {
    let mut readers = FS_READERS.lock().unwrap();
    let reader = readers
        .get_mut(reader_id as usize)
        .and_then(|slot| slot.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("fs.readLine: reader {reader_id} not found")));
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .unwrap_or_else(|err| runtime_error(&format!("fs.readLine: {err}")));
    if bytes == 0 {
        optional_text(None)
    } else {
        optional_text(Some(strip_read_line_ending(&line).to_string()))
    }
}

fn strip_read_line_ending(line: &str) -> &str {
    let Some(stripped) = line.strip_suffix('\n') else {
        return line;
    };
    stripped.strip_suffix('\r').unwrap_or(stripped)
}

#[unsafe(export_name = "zutai.host.fs_close_read")]
pub extern "C" fn host_fs_close_read(reader_id: i64) -> i64 {
    let mut readers = FS_READERS.lock().unwrap();
    let slot = readers
        .get_mut(reader_id as usize)
        .unwrap_or_else(|| runtime_error(&format!("fs.closeRead: reader {reader_id} not found")));
    *slot = None;
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.fs_open_write")]
pub extern "C" fn host_fs_open_write(path: i64) -> i64 {
    let path = unsafe {
        str::from_utf8(text_parts(path))
            .unwrap_or_else(|_| runtime_error("fs.openWrite path is not UTF-8"))
    };
    let file =
        File::create(path).unwrap_or_else(|err| runtime_error(&format!("fs.openWrite: {err}")));
    let id = fs_alloc_id();
    let mut writers = FS_WRITERS.lock().unwrap();
    while writers.len() <= id as usize {
        writers.push(None);
    }
    writers[id as usize] = Some(BufWriter::new(file));
    id as i64
}

#[unsafe(export_name = "zutai.host.fs_write_text")]
pub extern "C" fn host_fs_write_text(request: i64) -> i64 {
    unsafe {
        // `{ contents : Text; writer : Writer; }` is slot-sorted by name.
        let contents = str::from_utf8(text_parts(word(request, 1)))
            .unwrap_or_else(|_| runtime_error("fs.writeText contents are not UTF-8"));
        let writer_id = word(request, 2);
        let mut writers = FS_WRITERS.lock().unwrap();
        let writer = writers
            .get_mut(writer_id as usize)
            .and_then(|slot| slot.as_mut())
            .unwrap_or_else(|| {
                runtime_error(&format!("fs.writeText: writer {writer_id} not found"))
            });
        writer
            .write_all(contents.as_bytes())
            .unwrap_or_else(|err| runtime_error(&format!("fs.writeText: {err}")));
    }
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.fs_flush")]
pub extern "C" fn host_fs_flush(writer_id: i64) -> i64 {
    let mut writers = FS_WRITERS.lock().unwrap();
    let writer = writers
        .get_mut(writer_id as usize)
        .and_then(|slot| slot.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("fs.flush: writer {writer_id} not found")));
    writer
        .flush()
        .unwrap_or_else(|err| runtime_error(&format!("fs.flush: {err}")));
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.fs_close_write")]
pub extern "C" fn host_fs_close_write(writer_id: i64) -> i64 {
    let mut writers = FS_WRITERS.lock().unwrap();
    let slot = writers
        .get_mut(writer_id as usize)
        .unwrap_or_else(|| runtime_error(&format!("fs.closeWrite: writer {writer_id} not found")));
    if let Some(mut writer) = slot.take() {
        writer
            .flush()
            .unwrap_or_else(|err| runtime_error(&format!("fs.closeWrite: {err}")));
    }
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.env_get")]
pub extern "C" fn host_env_get(name: i64) -> i64 {
    let name = unsafe {
        str::from_utf8(text_parts(name))
            .unwrap_or_else(|_| runtime_error("env.get name is not UTF-8"))
    };
    optional_text(std::env::var(name).ok())
}

#[unsafe(export_name = "zutai.host.clock_now")]
pub extern "C" fn host_clock_now(_unit: i64) -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    text_from_string(millis.to_string())
}

static RNG_STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

#[unsafe(export_name = "zutai.host.rng_next")]
pub extern "C" fn host_rng_next(_unit: i64) -> i64 {
    let mut state = RNG_STATE.load(Ordering::Relaxed);
    loop {
        let next = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        match RNG_STATE.compare_exchange_weak(state, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return (next >> 1) as i64,
            Err(found) => state = found,
        }
    }
}

// ── Network capability operations ────────────────────────────────────────────────

static NET_LISTENERS: Mutex<Vec<Option<TcpListener>>> = Mutex::new(Vec::new());
static NET_CONNECTIONS: Mutex<Vec<Option<TcpStream>>> = Mutex::new(Vec::new());
static NET_CURRENT_CONN: AtomicU64 = AtomicU64::new(0);
static NET_NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn net_alloc_id(next: &AtomicU64) -> u64 {
    next.fetch_add(1, Ordering::Relaxed)
}

#[unsafe(export_name = "zutai.host.net_listen")]
pub extern "C" fn host_net_listen(port: i64) -> i64 {
    let addr = format!("127.0.0.1:{port}");
    let listener =
        TcpListener::bind(&addr).unwrap_or_else(|err| runtime_error(&format!("net.listen: {err}")));
    let id = net_alloc_id(&NET_NEXT_ID);
    let mut listeners = NET_LISTENERS.lock().unwrap();
    while listeners.len() <= id as usize {
        listeners.push(None);
    }
    listeners[id as usize] = Some(listener);
    id as i64
}

#[unsafe(export_name = "zutai.host.net_accept")]
pub extern "C" fn host_net_accept(listener_id: i64) -> i64 {
    let mut listeners = NET_LISTENERS.lock().unwrap();
    let listener = listeners
        .get_mut(listener_id as usize)
        .and_then(|opt| opt.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("net.accept: listener {listener_id} not found")));
    let (stream, _addr) = listener
        .accept()
        .unwrap_or_else(|err| runtime_error(&format!("net.accept: {err}")));
    let conn_id = net_alloc_id(&NET_NEXT_ID);
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    while conns.len() <= conn_id as usize {
        conns.push(None);
    }
    conns[conn_id as usize] = Some(stream);
    NET_CURRENT_CONN.store(conn_id, Ordering::Relaxed);
    conn_id as i64
}

#[unsafe(export_name = "zutai.host.net_read")]
pub extern "C" fn host_net_read(conn_id: i64) -> i64 {
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    let stream = conns
        .get_mut(conn_id as usize)
        .and_then(|opt| opt.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("net.read: connection {conn_id} not found")));
    let mut reader = BufReader::new(&mut *stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .unwrap_or_else(|err| runtime_error(&format!("net.read: {err}")));
    let trimmed = line.trim_end_matches(['\r', '\n']);
    text_from_string(trimmed.to_string())
}

#[unsafe(export_name = "zutai.host.net_write")]
pub extern "C" fn host_net_write(text: i64) -> i64 {
    let conn_id = NET_CURRENT_CONN.load(Ordering::Relaxed);
    if conn_id == 0 {
        runtime_error("net.write: no current connection");
    }
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    let stream = conns
        .get_mut(conn_id as usize)
        .and_then(|opt| opt.as_mut())
        .unwrap_or_else(|| runtime_error(&format!("net.write: connection {conn_id} not found")));
    let data = unsafe { text_parts(text) };
    use std::io::Write;
    stream
        .write_all(data)
        .and_then(|_| stream.flush())
        .unwrap_or_else(|err| runtime_error(&format!("net.write: {err}")));
    tuple_new(0)
}

#[unsafe(export_name = "zutai.host.net_close")]
pub extern "C" fn host_net_close(conn_id: i64) -> i64 {
    let mut conns = NET_CONNECTIONS.lock().unwrap();
    if (conn_id as usize) < conns.len() {
        conns[conn_id as usize] = None;
    }
    if NET_CURRENT_CONN.load(Ordering::Relaxed) == conn_id as u64 {
        NET_CURRENT_CONN.store(0, Ordering::Relaxed);
    }
    tuple_new(0)
}

// ── Output ──────────────────────────────────────────────────────────────────────

fn out_bytes(bytes: &[u8]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(bytes);
    let _ = lock.flush();
}

/// Rust's shortest round-trip float repr, matching the interpreter's `Display`
/// (`crates/general/eval/src/value.rs`): bare `inf`/`-inf`/`NaN`, and `.0`
/// appended only for finite integral values.
fn fmt_float(x: f64) -> String {
    let s = format!("{x:?}");
    if !x.is_finite() || s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn to_f64_p32<const ES: u32>(bits: u64) -> f64 {
    Posit::<32, ES, i64>::from_bits(bits as u32 as i32 as i64).round_into()
}

fn to_f64_p64<const ES: u32>(bits: u64) -> f64 {
    Posit::<64, ES, i128>::from_bits(bits as i64 as i128).round_into()
}

type P32<const ES: u32> = Posit<32, ES, i64>;

fn p32_from_abi<const ES: u32>(bits: i32) -> P32<ES> {
    P32::<ES>::from_bits(bits as i64)
}

fn p32_to_abi<const ES: u32>(value: P32<ES>) -> i32 {
    value.to_bits() as i32
}

fn p32_add<const ES: u32>(lhs: i32, rhs: i32) -> i32 {
    p32_to_abi(p32_from_abi::<ES>(lhs) + p32_from_abi::<ES>(rhs))
}

#[unsafe(export_name = "zutai.posit32e2.add")]
pub extern "C" fn posit32e2_add(lhs: i32, rhs: i32) -> i32 {
    p32_add::<2>(lhs, rhs)
}

fn fmt_posit(bits: i64, nbits: i64, es: i64) -> String {
    let value = match nbits {
        32 => match_p32_es!(es, to_f64_p32, bits as u64),
        64 => match_p64_es!(es, to_f64_p64, bits as u64),
        _ => unreachable!("invalid posit width"),
    };
    let mut out = format!("{value:?}");
    if value.is_finite() && out.ends_with(".0") {
        out.truncate(out.len() - 2);
    }
    if es == 2 {
        out.push_str(&format!("p{nbits}"));
    } else {
        out.push_str(&format!("p{nbits}e{es}"));
    }
    out
}

#[unsafe(export_name = "zutai.print_i64")]
pub extern "C" fn print_i64(v: i64) {
    out_bytes(v.to_string().as_bytes());
}

#[unsafe(export_name = "zutai.print_bool")]
pub extern "C" fn print_bool(v: i64) {
    out_bytes(if v != 0 { b"true" } else { b"false" });
}

#[unsafe(export_name = "zutai.print_float")]
pub extern "C" fn print_float(v: i64) {
    out_bytes(fmt_float(f64::from_bits(v as u64)).as_bytes());
}

/// Raw text output — the `io.print` handler. Writes the UTF-8 bytes verbatim
/// (no quotes); contrast with `show`, which quotes and escapes `Text` values.
#[unsafe(export_name = "zutai.print_text")]
pub extern "C" fn print_text(v: i64) {
    out_bytes(unsafe { text_parts(v) });
}

#[unsafe(export_name = "zutai.print_posit")]
pub extern "C" fn print_posit(v: i64, nbits: i64, es: i64) {
    out_bytes(fmt_posit(v, nbits, es).as_bytes());
}
// ── show (D-0007 / D-0009) ──────────────────────────────────────────────────────

/// Type-directed render of a fully-forced value, parity with the interpreter's
/// `Display`. Walks the value and its static descriptor in lockstep.
#[unsafe(export_name = "zutai.show")]
pub extern "C" fn show(value: i64, descriptor: i64) {
    let mut out = String::new();
    unsafe { render(&mut out, value, descriptor as *const i64) };
    out_bytes(out.as_bytes());
}

/// Type-directed natural JSON serialization. Returns a runtime `Text` object
/// containing compact JSON bytes produced by `serde_json`.
#[unsafe(export_name = "zutai.to_json")]
pub extern "C" fn to_json(value: i64, descriptor: i64) -> i64 {
    let json = unsafe { json_value(value, descriptor as *const i64) }
        .unwrap_or_else(|message| runtime_error(message));
    let rendered = serde_json::to_string(&json)
        .unwrap_or_else(|_| runtime_error("failed to serialize runtime value to JSON"));
    text_from_string(rendered)
}

#[unsafe(no_mangle)]
pub extern "C" fn zutai_to_json(value: i64, descriptor: i64) -> i64 {
    to_json(value, descriptor)
}

#[unsafe(export_name = "zutai.value_eq")]
pub extern "C" fn value_eq(lhs: i64, rhs: i64, descriptor: i64) -> i64 {
    unsafe { value_equal(lhs, rhs, descriptor as *const i64) as i64 }
}

fn push_quoted(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// # Safety
/// `desc` must be a valid name-ref pair `(ptr, len)` at `desc[off]`, `desc[off+1]`.
unsafe fn name_at(desc: *const i64, off: usize) -> &'static str {
    unsafe {
        let ptr = *desc.add(off) as *const u8;
        let len = *desc.add(off + 1) as usize;
        str::from_utf8(slice::from_raw_parts(ptr, len)).unwrap_or("?")
    }
}

/// Type-directed equality, sharing the static descriptor format used by `show`.
///
/// # Safety
/// `lhs` and `rhs` must match the type described by `desc`.
unsafe fn value_equal(lhs: i64, rhs: i64, desc: *const i64) -> bool {
    unsafe {
        match *desc {
            DESC_INT | DESC_BOOL | DESC_POSIT => lhs == rhs,
            DESC_FLOAT => f64::from_bits(lhs as u64) == f64::from_bits(rhs as u64),
            DESC_TEXT | DESC_ATOM => text_parts(lhs) == text_parts(rhs),
            DESC_LIST => {
                let elem = *desc.add(1) as *const i64;
                let mut left = lhs;
                let mut right = rhs;
                loop {
                    match (header_tag(word(left, 0)), header_tag(word(right, 0))) {
                        (TAG_CONS, TAG_CONS) => {
                            if !value_equal(word(left, 1), word(right, 1), elem) {
                                return false;
                            }
                            left = word(left, 2);
                            right = word(right, 2);
                        }
                        (TAG_NIL, TAG_NIL) => return true,
                        _ => return false,
                    }
                }
            }
            DESC_OPTIONAL | DESC_MAYBE => {
                let inner = *desc.add(1) as *const i64;
                match (
                    header_tag(word(lhs, 0)) == TAG_VARIANT,
                    header_tag(word(rhs, 0)) == TAG_VARIANT,
                ) {
                    (true, true) => {
                        value_equal(word(word(lhs, 2), 1), word(word(rhs, 2), 1), inner)
                    }
                    (false, false) => text_parts(lhs) == text_parts(rhs),
                    _ => false,
                }
            }
            DESC_RECORD => record_equal(lhs, rhs, desc),
            DESC_TUPLE => {
                let n = *desc.add(1) as usize;
                (0..n).all(|i| {
                    let base = 2 + i * 4;
                    value_equal(
                        word(lhs, 1 + i),
                        word(rhs, 1 + i),
                        *desc.add(base + 3) as *const i64,
                    )
                })
            }
            DESC_VARIANT => {
                let lhs_text = header_tag(word(lhs, 0)) == TAG_TEXT;
                let rhs_text = header_tag(word(rhs, 0)) == TAG_TEXT;
                if lhs_text || rhs_text {
                    return lhs_text && rhs_text && text_parts(lhs) == text_parts(rhs);
                }
                let lhs_tag = word(lhs, 1);
                if lhs_tag != word(rhs, 1) {
                    return false;
                }
                let base = 2 + lhs_tag as usize * 3;
                let payload_desc = *desc.add(base + 2);
                payload_desc == 0
                    || value_equal(word(lhs, 2), word(rhs, 2), payload_desc as *const i64)
            }
            _ => lhs == rhs,
        }
    }
}

/// # Safety
/// `value` must be an Optional/Maybe storage value.
unsafe fn wrapper_is_present(value: i64) -> bool {
    unsafe { header_tag(word(value, 0)) == TAG_VARIANT }
}

/// # Safety
/// `value` must be a present Optional/Maybe storage value.
unsafe fn wrapper_payload(value: i64) -> i64 {
    unsafe { word(word(value, 2), 1) }
}

/// # Safety
/// `lhs` and `rhs` must be Optional/Maybe storage values with payloads described
/// by `inner_desc`.
unsafe fn wrapper_storage_equal(lhs: i64, rhs: i64, inner_desc: *const i64) -> bool {
    unsafe {
        match (wrapper_is_present(lhs), wrapper_is_present(rhs)) {
            (true, true) => value_equal(wrapper_payload(lhs), wrapper_payload(rhs), inner_desc),
            (false, false) => text_parts(lhs) == text_parts(rhs),
            _ => false,
        }
    }
}

/// # Safety
/// `lhs` and `rhs` must be record objects matching `desc`.
unsafe fn record_equal(lhs: i64, rhs: i64, desc: *const i64) -> bool {
    unsafe {
        let n = *desc.add(1) as usize;
        (0..n).all(|i| {
            let base = 2 + i * 4;
            let inner_desc = *desc.add(base + 3) as *const i64;
            let left = word(lhs, 1 + i);
            let right = word(rhs, 1 + i);
            if *desc.add(base + 2) != 0 {
                wrapper_storage_equal(left, right, inner_desc)
            } else {
                value_equal(left, right, inner_desc)
            }
        })
    }
}

/// # Safety
/// `value` must match the type described by `desc`.
unsafe fn json_value(value: i64, desc: *const i64) -> Result<serde_json::Value, &'static str> {
    use serde_json::{Map, Number, Value as J};

    unsafe {
        match *desc {
            DESC_INT => Ok(J::Number(Number::from(value))),
            DESC_BOOL => Ok(J::Bool(value != 0)),
            DESC_FLOAT => {
                let value = f64::from_bits(value as u64);
                Number::from_f64(value)
                    .map(J::Number)
                    .ok_or("cannot serialize non-finite float to JSON")
            }
            DESC_POSIT => Ok(J::String(fmt_posit(value, *desc.add(1), *desc.add(2)))),
            DESC_TEXT => Ok(J::String(text_json_string(value)?)),
            DESC_ATOM => Ok(J::String(format!("#{}", text_json_string(value)?))),
            DESC_LIST => {
                let elem = *desc.add(1) as *const i64;
                let mut items = Vec::new();
                let mut node = value;
                while header_tag(word(node, 0)) == TAG_CONS {
                    items.push(json_value(word(node, 1), elem)?);
                    node = word(node, 2);
                }
                Ok(J::Array(items))
            }
            DESC_OPTIONAL => json_wrapper(value, *desc.add(1) as *const i64, "some", "none"),
            DESC_MAYBE => json_wrapper(value, *desc.add(1) as *const i64, "present", "absent"),
            DESC_RECORD => json_record(value, desc),
            DESC_TUPLE => json_tuple_array(value, desc),
            DESC_VARIANT => {
                if header_tag(word(value, 0)) == TAG_TEXT {
                    return Ok(J::String(format!("#{}", text_json_string(value)?)));
                }
                if header_tag(word(value, 0)) != TAG_VARIANT {
                    return Err("cannot serialize malformed tagged value to JSON");
                }
                let tag = word(value, 1) as usize;
                let n = *desc.add(1) as usize;
                if tag >= n {
                    return Err("cannot serialize tagged value with out-of-range tag");
                }
                let base = 2 + tag * 3;
                let tag_name = name_at(desc, base).to_string();
                let payload_desc = *desc.add(base + 2);
                let payload = if payload_desc == 0 {
                    J::Null
                } else {
                    json_tag_payload(word(value, 2), payload_desc as *const i64)?
                };
                let mut map = Map::with_capacity(2);
                map.insert("tag".to_string(), J::String(tag_name));
                map.insert("payload".to_string(), payload);
                Ok(J::Object(map))
            }
            _ => Err("cannot serialize non-data runtime value to JSON"),
        }
    }
}

/// # Safety
/// `value` must be a runtime text object.
unsafe fn text_json_string(value: i64) -> Result<String, &'static str> {
    unsafe {
        str::from_utf8(text_parts(value))
            .map(str::to_string)
            .map_err(|_| "cannot serialize non-UTF-8 text to JSON")
    }
}

/// # Safety
/// `value` must be Optional/Maybe storage for `inner_desc`.
unsafe fn json_wrapper(
    value: i64,
    inner_desc: *const i64,
    present_tag: &str,
    absent_tag: &str,
) -> Result<serde_json::Value, &'static str> {
    use serde_json::{Map, Value as J};

    unsafe {
        if wrapper_is_present(value) {
            let payload = J::Array(vec![json_value(wrapper_payload(value), inner_desc)?]);
            let mut map = Map::with_capacity(2);
            map.insert("tag".to_string(), J::String(present_tag.to_string()));
            map.insert("payload".to_string(), payload);
            Ok(J::Object(map))
        } else {
            Ok(J::String(format!("#{absent_tag}")))
        }
    }
}

/// # Safety
/// `value` must be a record object matching `desc`.
unsafe fn json_record(value: i64, desc: *const i64) -> Result<serde_json::Value, &'static str> {
    let mut map = serde_json::Map::new();
    unsafe {
        let n = *desc.add(1) as usize;
        for i in 0..n {
            let base = 2 + i * 4;
            let optional = *desc.add(base + 2) != 0;
            let slot = word(value, 1 + i);
            if optional && !wrapper_is_present(slot) {
                continue;
            }
            let field_value = if optional {
                wrapper_payload(slot)
            } else {
                slot
            };
            map.insert(
                name_at(desc, base).to_string(),
                json_value(field_value, *desc.add(base + 3) as *const i64)?,
            );
        }
    }
    Ok(serde_json::Value::Object(map))
}

/// # Safety
/// `value` must be a tuple object matching `desc`.
unsafe fn json_tuple_array(
    value: i64,
    desc: *const i64,
) -> Result<serde_json::Value, &'static str> {
    unsafe {
        let n = *desc.add(1) as usize;
        let mut items = Vec::with_capacity(n);
        for i in 0..n {
            let base = 2 + i * 4;
            items.push(json_value(
                word(value, 1 + i),
                *desc.add(base + 3) as *const i64,
            )?);
        }
        Ok(serde_json::Value::Array(items))
    }
}

/// # Safety
/// `payload` must match `desc`, which is a union member payload descriptor.
unsafe fn json_tag_payload(
    payload: i64,
    desc: *const i64,
) -> Result<serde_json::Value, &'static str> {
    unsafe {
        match *desc {
            DESC_RECORD => json_record(payload, desc),
            DESC_TUPLE => json_tuple_array(payload, desc),
            _ => json_value(payload, desc),
        }
    }
}

/// Render record fields. Optional slots use native storage `Maybe`: `#absent`
/// is omitted, while `#present (x)` renders the payload as the field value.
///
/// # Safety
/// `value` is a record object and `desc` its `DESC_RECORD` descriptor.
unsafe fn render_record_fields(
    out: &mut String,
    value: i64,
    desc: *const i64,
    tagged_payload_spacing: bool,
) {
    unsafe {
        let n = *desc.add(1) as usize;
        let mut rendered = 0usize;
        for i in 0..n {
            let base = 2 + i * 4;
            let optional = *desc.add(base + 2) != 0;
            let slot = word(value, 1 + i);
            if optional && !wrapper_is_present(slot) {
                continue;
            }
            if rendered > 0 {
                out.push_str(if tagged_payload_spacing { ";" } else { "; " });
            }
            rendered += 1;
            out.push(' ');
            out.push_str(name_at(desc, base));
            out.push_str(" = ");
            let field_value = if optional {
                wrapper_payload(slot)
            } else {
                slot
            };
            render(out, field_value, *desc.add(base + 3) as *const i64);
        }
    }
}

/// # Safety
/// `value` must match the type described by `desc` (both produced by codegen for
/// the same `DfTy`).
unsafe fn render(out: &mut String, value: i64, desc: *const i64) {
    unsafe {
        match *desc {
            DESC_INT => out.push_str(&value.to_string()),
            DESC_BOOL => out.push_str(if value != 0 { "true" } else { "false" }),
            DESC_FLOAT => out.push_str(&fmt_float(f64::from_bits(value as u64))),
            DESC_TEXT => {
                let s = str::from_utf8(text_parts(value)).unwrap_or("");
                push_quoted(out, s);
            }
            DESC_ATOM => {
                let s = str::from_utf8(text_parts(value)).unwrap_or("");
                out.push('#');
                out.push_str(s);
            }
            DESC_POSIT => out.push_str(&fmt_posit(value, *desc.add(1), *desc.add(2))),
            DESC_LIST => {
                let elem = *desc.add(1) as *const i64;
                out.push('[');
                let mut node = value;
                let mut first = true;
                while header_tag(word(node, 0)) == TAG_CONS {
                    if !first {
                        out.push_str("; ");
                    }
                    first = false;
                    render(out, word(node, 1), elem);
                    node = word(node, 2);
                }
                out.push(']');
            }
            DESC_OPTIONAL => {
                // #some is variant_new(1, <1-tuple>); #none is an atom text object.
                if header_tag(word(value, 0)) == TAG_VARIANT {
                    out.push_str("#some (");
                    render(out, word(word(value, 2), 1), *desc.add(1) as *const i64);
                    out.push(')');
                } else {
                    out.push_str("#none");
                }
            }
            DESC_MAYBE => {
                if header_tag(word(value, 0)) == TAG_VARIANT {
                    out.push_str("#present (");
                    render(out, word(word(value, 2), 1), *desc.add(1) as *const i64);
                    out.push(')');
                } else {
                    out.push_str("#absent");
                }
            }
            DESC_RECORD => {
                out.push('{');
                render_record_fields(out, value, desc, false);
                out.push_str(" }");
            }
            DESC_TUPLE => {
                let n = *desc.add(1) as usize;
                out.push('(');
                for i in 0..n {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let base = 2 + i * 4;
                    if *desc.add(base) != 0 {
                        out.push_str(name_at(desc, base + 1));
                        out.push_str(" = ");
                    }
                    render(out, word(value, 1 + i), *desc.add(base + 3) as *const i64);
                }
                out.push(')');
            }
            DESC_VARIANT => {
                if header_tag(word(value, 0)) == TAG_TEXT {
                    // Nullary member: emitted as an atom text object, not variant_new.
                    let s = str::from_utf8(text_parts(value)).unwrap_or("");
                    out.push('#');
                    out.push_str(s);
                } else {
                    let tag = word(value, 1) as usize;
                    let base = 2 + tag * 3;
                    out.push('#');
                    out.push_str(name_at(desc, base));
                    let payload_desc = *desc.add(base + 2);
                    if payload_desc != 0 {
                        let pdesc = payload_desc as *const i64;
                        let pval = word(value, 2);
                        out.push(' ');
                        match *pdesc {
                            DESC_RECORD => {
                                out.push('{');
                                render_record_fields(out, pval, pdesc, true);
                                out.push_str(" }");
                            }
                            DESC_TUPLE => render(out, pval, pdesc),
                            _ => {
                                out.push('(');
                                render(out, pval, pdesc);
                                out.push(')');
                            }
                        }
                    }
                }
            }
            _ => out.push_str("<?>"),
        }
    }
}

#[cfg(test)]
mod tests;
