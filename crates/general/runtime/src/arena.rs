use super::*;

// ── Bump arena (D-0008) ─────────────────────────────────────────────────────────

pub(crate) const CHUNK_BYTES: usize = 1 << 20;

/// Default heap ceiling (2 GiB) when `ZUTAI_HEAP_MAX` is unset: generous enough
/// for any v0 fixture/spec program, low enough to abort a runaway leak cleanly
/// before the OS OOM-killer steps in.
pub(crate) const DEFAULT_HEAP_MAX: usize = 2 << 30;

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
pub(crate) struct Arena {
    pub(crate) chunks: Vec<Box<[u128]>>,
    /// Byte offset into the last chunk.
    pub(crate) off: usize,
    /// Total bytes committed across all chunks; never exceeds `cap`.
    pub(crate) committed: usize,
    /// Maximum committed bytes; `usize::MAX` means unlimited.
    pub(crate) cap: usize,
    /// Collector state; `None` only when opted out (`ZUTAI_GC=0`) or unsupported.
    pub(crate) gc: Option<Gc>,
}

impl Arena {
    pub(crate) fn with_cap(cap: usize) -> Self {
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
    pub(crate) fn with_cap_leak(cap: usize) -> Self {
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
    pub(crate) fn try_alloc(&mut self, bytes: usize) -> Option<*mut u8> {
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
pub(crate) struct Gc {
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
pub(crate) struct GcMode {
    enabled: bool,
    stress: bool,
}

/// Whether an env var holds a truthy value (`1`/`true`/`yes`/`on`).
pub(crate) fn env_truthy(var: &str) -> bool {
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
pub(crate) fn env_falsy(var: &str) -> bool {
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
pub(crate) fn gc_mode() -> GcMode {
    static MODE: OnceLock<GcMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let stress = env_truthy("ZUTAI_GC_STRESS");
        let enabled = (stress || !env_falsy("ZUTAI_GC")) && stack_base().is_some();
        GcMode { enabled, stress }
    })
}

/// First-fit allocation from the free list, splitting the chosen span and
/// leaving any remainder. Returns the start of the `bytes`-sized prefix.
pub(crate) fn freelist_take(free: &mut Vec<(usize, usize)>, bytes: usize) -> Option<usize> {
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
pub(crate) fn coalesce_free(free: &mut Vec<(usize, usize)>) {
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
pub(crate) fn find_object(objects: &BTreeMap<usize, usize>, ptr: usize) -> Option<usize> {
    let (&start, &size) = objects.range(..=ptr).next_back()?;
    (ptr < start + size).then_some(start)
}

/// Whether `p` falls inside any committed chunk — a cheap reject before the
/// object-table lookup.
pub(crate) fn ptr_in_chunks(bounds: &[(usize, usize)], p: usize) -> bool {
    bounds.iter().any(|&(lo, hi)| p >= lo && p < hi)
}

/// If `w` points into a live object, mark that object and enqueue it for tracing.
pub(crate) fn mark_candidate(
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
pub(crate) const JMPBUF_WORDS: usize = 256;

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
pub(crate) fn stack_base() -> Option<usize> {
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
pub(crate) fn stack_base() -> Option<usize> {
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
pub(crate) fn stack_base() -> Option<usize> {
    None
}

/// Flush callee-saved registers, then conservatively scan the active stack
/// `[sp, base)` for candidate pointers. Returns `false` if the stack bounds are
/// unknown, so the caller skips a cycle it could not complete safely.
pub(crate) fn scan_stack_roots(
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
pub(crate) static GC_COLLECTIONS: AtomicUsize = AtomicUsize::new(0);
pub(crate) static GC_RECLAIMED_BYTES: AtomicUsize = AtomicUsize::new(0);
pub(crate) static GC_RECLAIMED_OBJECTS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Per-thread bump arena. v0 mutators are single-threaded, so a compiled
    /// program uses one arena on the main thread; isolating per-thread keeps the
    /// unsynchronized `RefCell` sound under parallel `cargo test`.
    static ARENA: RefCell<Arena> = RefCell::new(Arena::with_cap(heap_cap()));
}

/// Resolve the heap ceiling once per process from `ZUTAI_HEAP_MAX`.
pub(crate) fn heap_cap() -> usize {
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
pub(crate) fn parse_cap_bytes(s: &str) -> Option<usize> {
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
pub(crate) fn heap_limit_exceeded(bytes: usize, cap: usize) -> ! {
    eprintln!(
        "zutai runtime error: heap limit exceeded (cap {cap} bytes; cannot allocate {bytes} more). \
Raise it with ZUTAI_HEAP_MAX (e.g. ZUTAI_HEAP_MAX=4G), or ZUTAI_HEAP_MAX=0 for unlimited."
    );
    std::process::exit(1);
}

pub(crate) fn arena_alloc(bytes: usize) -> *mut u8 {
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

pub(crate) fn alloc_words(n: usize) -> *mut i64 {
    arena_alloc(n * 8).cast::<i64>()
}

// ── Heap statistics (measurement groundwork for the GC decision) ─────────────────

/// Process-global allocation counters. Updated on every allocation (a few
/// relaxed atomic adds, negligible beside the bump and header writes) so the
/// numbers are always exact; the human-readable dump at process exit is gated on
/// `ZUTAI_HEAP_STATS`. Counters aggregate across every thread's arena.
pub(crate) struct HeapStats {
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

pub(crate) static STATS: HeapStats = HeapStats {
    bytes: AtomicUsize::new(0),
    objects: AtomicUsize::new(0),
    peak_committed: AtomicUsize::new(0),
    by_tag: [const { AtomicUsize::new(0) }; 8],
};

/// Whether to print the exit-time dump; resolved once from `ZUTAI_HEAP_STATS`
/// (`1`/`true`/`yes`/`on`).
pub(crate) fn stats_enabled() -> bool {
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
pub(crate) fn stats_record_alloc(bytes: usize, committed: usize) {
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
pub(crate) fn note_tag(tag: i64) {
    STATS.by_tag[tag as usize].fetch_add(1, Ordering::Relaxed);
}

/// Render the one-line stats report. Pure (testable) given a counter snapshot;
/// `by_tag` is indexed by `TAG_*`. Objects not covered by a tracked tag
/// (closures, raw `Text` byte buffers) fall into `closure/raw`.
pub(crate) fn format_stats_line(
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
pub(crate) extern "C" fn dump_heap_stats() {
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
