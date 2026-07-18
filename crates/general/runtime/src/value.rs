use super::*;

// ── Raw word access ─────────────────────────────────────────────────────────────

/// # Safety
/// `p` must be a valid object pointer with at least `i + 1` words.
pub(crate) unsafe fn word(p: i64, i: usize) -> i64 {
    unsafe { *(p as *const i64).add(i) }
}

/// # Safety
/// `p` must be a valid object pointer with at least `i + 1` writable words.
pub(crate) unsafe fn set_word(p: i64, i: usize, v: i64) {
    unsafe { *(p as *mut i64).add(i) = v }
}

/// # Safety
/// `t` must point to a `ZtText` object.
pub(crate) unsafe fn text_parts<'a>(t: i64) -> &'a [u8] {
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
pub(crate) struct NilObj(i64);

pub(crate) static NIL_OBJ: NilObj = NilObj(header(TAG_NIL, 0));

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
