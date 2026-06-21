//! Runtime library for compiled Zutai general-mode programs.
//!
//! This crate implements the `@zutai.*` symbols that `zutai-codegen` references
//! and defines the binary representation of runtime values. The full contract
//! is specified in `docs/runtime-abi.md`; this module is the v0 skeleton of
//! Phase 18 (Runtime & ABI).
//!
//! Every Zutai value is an `i64` (D-0002, untagged, statically dispatched):
//! immediates (`Int`, `Bool`, `Float` bits, dense atom/variant tags) are stored
//! inline; heap values (records, tuples, lists, variants, text) are pointers
//! cast to `i64`, each beginning with a one-word header (D-0009).
//!
//! Memory is a bump arena owned by a process-lifetime `LazyLock` (D-0008); the
//! arena's chunks are owned, never leaked, and `zutai.free` is a no-op in v0.
//!
//! Closures (D-0003) are built and applied inline by codegen, so no closure
//! symbol lives here. `TAG_CLOSURE` is reserved for the header layout only.

use fast_posit::{Posit, RoundInto};

use std::slice;
use std::str;
use std::sync::{LazyLock, Mutex};

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
/// High bits stay zero in v0 (reserved for the future GC's layout id).
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

/// A chunked bump allocator. Chunks are owned by the `ARENA` static (process
/// lifetime), so returned pointers stay valid; `Box<[u8]>` payloads keep a
/// stable address across `chunks` vector growth.
struct Arena {
    chunks: Vec<Box<[u8]>>,
    off: usize,
}

impl Arena {
    const fn new() -> Self {
        Arena {
            chunks: Vec::new(),
            off: 0,
        }
    }

    fn alloc(&mut self, bytes: usize) -> *mut u8 {
        let bytes = (bytes + 15) & !15; // 16-byte alignment
        let need_new = match self.chunks.last() {
            Some(chunk) => self.off + bytes > chunk.len(),
            None => true,
        };
        if need_new {
            let size = bytes.max(CHUNK_BYTES);
            self.chunks.push(vec![0u8; size].into_boxed_slice());
            self.off = 0;
        }
        let chunk = self.chunks.last_mut().expect("chunk present after push");
        // SAFETY: `off + bytes <= chunk.len()` holds by the check above.
        let ptr = unsafe { chunk.as_mut_ptr().add(self.off) };
        self.off += bytes;
        ptr
    }
}

static ARENA: LazyLock<Mutex<Arena>> = LazyLock::new(|| Mutex::new(Arena::new()));

fn arena_alloc(bytes: usize) -> *mut u8 {
    ARENA.lock().expect("arena mutex poisoned").alloc(bytes)
}

fn alloc_words(n: usize) -> *mut i64 {
    arena_alloc(n * 8).cast::<i64>()
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
    // No-op in v0 (leak-by-default arena; OS reclaims at exit).
}

// ── Record ABI (D-0004, ordinal slots) ──────────────────────────────────────────

#[unsafe(export_name = "zutai.record_new")]
pub extern "C" fn record_new(n: i64) -> i64 {
    let p = alloc_words(1 + n as usize);
    unsafe { *p = header(TAG_RECORD, n as u64) };
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
        out
    }
}

// ── Tuple ABI (positional/named slots) ──────────────────────────────────────────

#[unsafe(export_name = "zutai.tuple_new")]
pub extern "C" fn tuple_new(n: i64) -> i64 {
    let p = alloc_words(1 + n as usize);
    unsafe { *p = header(TAG_TUPLE, n as u64) };
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

static NIL: LazyLock<i64> = LazyLock::new(|| {
    let p = alloc_words(1);
    unsafe { *p = header(TAG_NIL, 0) };
    p as i64
});

#[unsafe(export_name = "zutai.list_nil")]
pub extern "C" fn list_nil() -> i64 {
    *NIL
}

#[unsafe(export_name = "zutai.list_cons")]
pub extern "C" fn list_cons(head: i64, tail: i64) -> i64 {
    let p = alloc_words(3);
    unsafe {
        *p = header(TAG_CONS, 0);
        *p.add(1) = head;
        *p.add(2) = tail;
    }
    p as i64
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

/// Unwrap one `Optional`/`Maybe` layer: dense tag 0 (`#none`/`#absent`) selects
/// the fallback, otherwise return the payload.
#[unsafe(export_name = "zutai.coalesce")]
pub extern "C" fn coalesce(v: i64, fallback: i64) -> i64 {
    unsafe {
        if word(v, 1) == 0 {
            fallback
        } else {
            word(v, 2)
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
    p as i64
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

/// Render a record-shaped variant payload with the interpreter's tagged-record
/// spacing (`;`-separated, one space), distinct from a standalone record.
///
/// # Safety
/// `value` is a record object and `desc` its `DESC_RECORD` descriptor.
unsafe fn render_variant_named(out: &mut String, value: i64, desc: *const i64) {
    unsafe {
        let n = *desc.add(1) as usize;
        out.push('{');
        for i in 0..n {
            if i > 0 {
                out.push(';');
            }
            let base = 2 + i * 3;
            out.push(' ');
            out.push_str(name_at(desc, base));
            out.push_str(" = ");
            render(out, word(value, 1 + i), *desc.add(base + 2) as *const i64);
        }
        out.push_str(" }");
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
                if word(value, 1) == 0 {
                    out.push_str("#none");
                } else {
                    out.push_str("#some (");
                    render(out, word(value, 2), *desc.add(1) as *const i64);
                    out.push(')');
                }
            }
            DESC_MAYBE => {
                if word(value, 1) == 0 {
                    out.push_str("#absent");
                } else {
                    out.push_str("#present (");
                    render(out, word(value, 2), *desc.add(1) as *const i64);
                    out.push(')');
                }
            }
            DESC_RECORD => {
                let n = *desc.add(1) as usize;
                out.push('{');
                for i in 0..n {
                    if i > 0 {
                        out.push_str("; ");
                    }
                    let base = 2 + i * 3;
                    out.push(' ');
                    out.push_str(name_at(desc, base));
                    out.push_str(" = ");
                    render(out, word(value, 1 + i), *desc.add(base + 2) as *const i64);
                }
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
                        DESC_RECORD => render_variant_named(out, pval, pdesc),
                        DESC_TUPLE => render(out, pval, pdesc),
                        _ => {
                            out.push('(');
                            render(out, pval, pdesc);
                            out.push(')');
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
