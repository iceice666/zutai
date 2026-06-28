//! ABI round-trip and `show`/render parity tests for the v0 runtime skeleton.
//!
//! Descriptors are built as local `i64` arrays in the exact wire layout codegen
//! will emit (D-0009). Rendered strings are asserted against the interpreter's
//! `Display` output, byte for byte.

use super::*;

/// Render `value` against descriptor array `desc` into a fresh string.
fn render_str(value: i64, desc: &[i64]) -> String {
    let mut out = String::new();
    // SAFETY: each test pairs a value with a descriptor built for its type.
    unsafe { render(&mut out, value, desc.as_ptr()) };
    out
}

fn text(s: &str) -> i64 {
    text_from_global(s.as_ptr() as i64, s.len() as i64)
}

#[test]
fn num_bridge_helpers_return_values() {
    assert_eq!(num_abs(-5), 5);
    assert_eq!(num_rem(17, 5), 2);
    assert_eq!(num_pow(2, 10), 1024);
    assert_eq!(f64::from_bits(num_to_float(42) as u64), 42.0);
    assert_eq!(num_round(2.6f64.to_bits() as i64), 3);
    assert_eq!(num_round((-2.5f64).to_bits() as i64), -3);
    assert_eq!(num_truncate((-2.9f64).to_bits() as i64), -2);
}

#[test]
fn text_bridge_helpers_return_values() {
    assert_eq!(text_length(text("hé")), 2);
    assert_eq!(
        render_str(text_trim(text("  hi  ")), &[DESC_TEXT]),
        "\"hi\""
    );
    assert_eq!(
        render_str(text_to_upper(text("ab")), &[DESC_TEXT]),
        "\"AB\""
    );
    assert_eq!(
        render_str(text_to_lower(text("AB")), &[DESC_TEXT]),
        "\"ab\""
    );
    assert_eq!(text_contains(text("b"), text("abc")), 1);
    assert_eq!(
        render_str(
            text_replace(text("a"), text("o"), text("cat")),
            &[DESC_TEXT]
        ),
        "\"cot\""
    );
    assert_eq!(
        render_str(text_show(text("x")), &[DESC_TEXT]),
        "\"\\\"x\\\"\""
    );

    let parts = text_split(text(","), text("a,b"));
    assert_eq!(
        render_str(text_join(text("-"), parts), &[DESC_TEXT]),
        "\"a-b\""
    );

    let parsed_int = text_parse_int(text("42"));
    let int_desc = [DESC_INT];
    let opt_int_desc = [DESC_OPTIONAL, int_desc.as_ptr() as i64];
    assert_eq!(render_str(parsed_int, &opt_int_desc), "#some (42)");

    let parsed_float = text_parse_float(text("2.5"));
    let float_desc = [DESC_FLOAT];
    let opt_float_desc = [DESC_OPTIONAL, float_desc.as_ptr() as i64];
    assert_eq!(render_str(parsed_float, &opt_float_desc), "#some (2.5)");
    assert_eq!(
        render_str(text_parse_int(text("nope")), &opt_int_desc),
        "#none"
    );
}
#[test]
fn record_round_trip_and_update() {
    let r = record_new(2);
    record_set(r, 0, 1);
    record_set(r, 1, 2);
    assert_eq!(record_get(r, 0), 1);
    assert_eq!(record_get(r, 1), 2);

    let r2 = record_update(r, 1, 9);
    assert_eq!(record_get(r2, 0), 1);
    assert_eq!(record_get(r2, 1), 9);
    // Records are immutable: the update is a fresh copy.
    assert_eq!(record_get(r, 1), 2);
}

#[test]
fn record_render_matches_display_spacing() {
    let int_d = [DESC_INT];
    let text_d = [DESC_TEXT];
    let rec_d = [
        DESC_RECORD,
        2,
        b"x".as_ptr() as i64,
        1,
        int_d.as_ptr() as i64,
        b"y".as_ptr() as i64,
        1,
        text_d.as_ptr() as i64,
    ];
    let r = record_new(2);
    record_set(r, 0, 5);
    record_set(r, 1, text("hi"));
    // Two spaces after the `;` — exactly the interpreter's Display output.
    assert_eq!(render_str(r, &rec_d), "{ x = 5;  y = \"hi\" }");
}

#[test]
fn list_render() {
    let int_d = [DESC_INT];
    let list_d = [DESC_LIST, int_d.as_ptr() as i64];
    let l = list_cons(1, list_cons(2, list_cons(3, list_nil())));
    assert_eq!(render_str(l, &list_d), "[1; 2; 3]");

    let empty = list_nil();
    assert_eq!(render_str(empty, &list_d), "[]");
}

#[test]
fn optional_and_maybe_render() {
    let int_d = [DESC_INT];
    let opt_d = [DESC_OPTIONAL, int_d.as_ptr() as i64];
    assert_eq!(render_str(text("none"), &opt_d), "#none");
    let some = {
        let t = tuple_new(1);
        tuple_set(t, 0, 7);
        variant_new(1, t)
    };
    assert_eq!(render_str(some, &opt_d), "#some (7)");

    let maybe_d = [DESC_MAYBE, int_d.as_ptr() as i64];
    assert_eq!(render_str(text("absent"), &maybe_d), "#absent");
    let present = {
        let t = tuple_new(1);
        tuple_set(t, 0, 9);
        variant_new(1, t)
    };
    assert_eq!(render_str(present, &maybe_d), "#present (9)");
}

#[test]
fn variant_render_all_payload_shapes() {
    let int_d = [DESC_INT];

    // Nullary member + tuple-payload member in one union.
    let tup_d = [
        DESC_TUPLE,
        2,
        0,
        0,
        0,
        int_d.as_ptr() as i64,
        0,
        0,
        0,
        int_d.as_ptr() as i64,
    ];
    let union_d = [
        DESC_VARIANT,
        2,
        b"red".as_ptr() as i64,
        3,
        0, // nullary
        b"rgb".as_ptr() as i64,
        3,
        tup_d.as_ptr() as i64,
    ];
    assert_eq!(render_str(text("red"), &union_d), "#red");
    let t = tuple_new(2);
    tuple_set(t, 0, 1);
    tuple_set(t, 1, 2);
    assert_eq!(render_str(variant_new(1, t), &union_d), "#rgb (1, 2)");

    // Single-value payload → wrapped in parens.
    let just_d = [
        DESC_VARIANT,
        1,
        b"just".as_ptr() as i64,
        4,
        int_d.as_ptr() as i64,
    ];
    assert_eq!(render_str(variant_new(0, 5), &just_d), "#just (5)");

    // Record payload → tagged-record spacing (single space after `;`).
    let rec_payload = [
        DESC_RECORD,
        1,
        b"radius".as_ptr() as i64,
        6,
        int_d.as_ptr() as i64,
    ];
    let circle_d = [
        DESC_VARIANT,
        1,
        b"circle".as_ptr() as i64,
        6,
        rec_payload.as_ptr() as i64,
    ];
    let rec = record_new(1);
    record_set(rec, 0, 5);
    assert_eq!(
        render_str(variant_new(0, rec), &circle_d),
        "#circle { radius = 5 }"
    );
}

#[test]
fn tuple_named_render() {
    let int_d = [DESC_INT];
    let tup_d = [
        DESC_TUPLE,
        2,
        1,
        b"a".as_ptr() as i64,
        1,
        int_d.as_ptr() as i64,
        1,
        b"b".as_ptr() as i64,
        1,
        int_d.as_ptr() as i64,
    ];
    let t = tuple_new(2);
    tuple_set(t, 0, 3);
    tuple_set(t, 1, 4);
    assert_eq!(render_str(t, &tup_d), "(a = 3, b = 4)");
}

#[test]
fn float_render_matches_display() {
    let float_d = [DESC_FLOAT];
    assert_eq!(render_str(2.5f64.to_bits() as i64, &float_d), "2.5");
    assert_eq!(render_str(3.0f64.to_bits() as i64, &float_d), "3.0");
    assert_eq!(render_str(f64::INFINITY.to_bits() as i64, &float_d), "inf");
    assert_eq!(render_str(f64::NAN.to_bits() as i64, &float_d), "NaN");
}

#[test]
fn posit_render_matches_display_suffix() {
    let posit_d = [DESC_POSIT, 32, 3];
    assert_eq!(render_str(0x4000_0000, &posit_d), "1p32e3");
    let posit_default_d = [DESC_POSIT, 64, 2];
    assert_eq!(render_str(0x4000_0000_0000_0000, &posit_default_d), "1p64");
}

#[test]
fn text_render_escapes() {
    let text_d = [DESC_TEXT];
    let v = text("a\"b\nc");
    assert_eq!(render_str(v, &text_d), "\"a\\\"b\\nc\"");
}

#[test]
fn atom_render() {
    let atom_d = [DESC_ATOM];
    assert_eq!(
        render_str(atom_from_global(b"foo".as_ptr() as i64, 3), &atom_d),
        "#foo"
    );
}

#[test]
fn coalesce_unwraps_one_layer() {
    assert_eq!(coalesce(text("none"), 99), 99);
    let some = {
        let t = tuple_new(1);
        tuple_set(t, 0, 7);
        variant_new(1, t)
    };
    assert_eq!(coalesce(some, 99), 7);
}

#[test]
fn text_concat_joins_bytes() {
    let c = text_concat(text("foo"), text("bar"));
    let text_d = [DESC_TEXT];
    assert_eq!(render_str(c, &text_d), "\"foobar\"");
}

#[test]
fn bool_render() {
    let bool_d = [DESC_BOOL];
    assert_eq!(render_str(1, &bool_d), "true");
    assert_eq!(render_str(0, &bool_d), "false");
}

// ── Arena cap + accounting (D-0008 release valve) ───────────────────────────────

#[test]
fn arena_allocates_below_cap_aligned_and_distinct() {
    let mut a = Arena::with_cap_leak(CHUNK_BYTES);
    let p1 = a.try_alloc(8).expect("first alloc fits");
    let p2 = a.try_alloc(8).expect("second alloc fits");
    assert_ne!(p1, p2, "distinct allocations");
    assert_eq!(p1 as usize % 16, 0, "p1 is 16-byte aligned");
    assert_eq!(p2 as usize % 16, 0, "p2 is 16-byte aligned");
    // An 8-byte request rounds up to 16, so the two are exactly 16 bytes apart.
    assert_eq!(p2 as usize - p1 as usize, 16);
}

#[test]
fn arena_try_alloc_returns_none_past_cap() {
    // Cap below one chunk: the first chunk is clamped to the cap, then exhausts.
    let mut a = Arena::with_cap_leak(64);
    assert!(a.try_alloc(32).is_some(), "32 fits in a 64-byte cap");
    assert!(
        a.try_alloc(32).is_some(),
        "another 32 fills the cap exactly"
    );
    assert!(a.try_alloc(16).is_none(), "cap exhausted -> None");
    assert!(
        a.committed <= 64,
        "committed never exceeds cap: {}",
        a.committed
    );
}

#[test]
fn arena_oversized_request_past_cap_is_rejected() {
    let mut a = Arena::with_cap_leak(64);
    assert!(
        a.try_alloc(128).is_none(),
        "a single request bigger than the cap -> None"
    );
    assert_eq!(a.committed, 0, "nothing is committed on rejection");
}

#[test]
fn arena_committed_tracks_chunk_growth() {
    let mut a = Arena::with_cap_leak(usize::MAX);
    assert_eq!(a.committed, 0);
    a.try_alloc(8).expect("alloc");
    assert_eq!(a.committed, CHUNK_BYTES, "first chunk is one CHUNK_BYTES");
    // Fill the rest of the first chunk without growing.
    a.try_alloc(CHUNK_BYTES - 16).expect("alloc");
    assert_eq!(a.committed, CHUNK_BYTES, "still one chunk");
    // The next allocation forces a second chunk.
    a.try_alloc(8).expect("alloc");
    assert_eq!(a.committed, 2 * CHUNK_BYTES, "second chunk committed");
}

#[test]
fn parse_cap_bytes_handles_suffixes_and_unlimited() {
    assert_eq!(parse_cap_bytes("1024"), Some(1024));
    assert_eq!(parse_cap_bytes("2k"), Some(2 * 1024));
    assert_eq!(parse_cap_bytes("2KiB"), Some(2 * 1024));
    assert_eq!(parse_cap_bytes("4m"), Some(4 * 1024 * 1024));
    assert_eq!(parse_cap_bytes("1G"), Some(1024 * 1024 * 1024));
    assert_eq!(parse_cap_bytes("  8M  "), Some(8 * 1024 * 1024));
    assert_eq!(parse_cap_bytes("unlimited"), Some(usize::MAX));
    assert_eq!(parse_cap_bytes("none"), Some(usize::MAX));
    assert_eq!(parse_cap_bytes("0"), Some(usize::MAX));
    assert_eq!(parse_cap_bytes("garbage"), None);
    assert_eq!(parse_cap_bytes(""), None);
}

// ── Heap-stats reporting ────────────────────────────────────────────────────────

#[test]
fn format_stats_line_reports_totals_and_kinds() {
    let mut by_tag = [0usize; 8];
    by_tag[TAG_RECORD as usize] = 4000;
    by_tag[TAG_CONS as usize] = 10;
    by_tag[TAG_TEXT as usize] = 1;
    // 4013 objects total => 2 not covered by a tracked tag (closures / raw bytes).
    let line = format_stats_line(4013, 96_312, 1 << 20, &by_tag, 2 << 30);
    assert!(
        line.contains("allocated 96312 bytes in 4013 objects"),
        "{line}"
    );
    assert!(line.contains("avg 24 B"), "{line}");
    assert!(
        line.contains("peak committed 1048576 bytes (cap 2147483648)"),
        "{line}"
    );
    assert!(line.contains("record 4000"), "{line}");
    assert!(line.contains("cons 10"), "{line}");
    assert!(line.contains("text 1"), "{line}");
    assert!(line.contains("closure/raw 2"), "{line}");
}

#[test]
fn format_stats_line_unlimited_cap_and_empty() {
    let by_tag = [0usize; 8];
    let line = format_stats_line(0, 0, 0, &by_tag, usize::MAX);
    assert!(line.contains("in 0 objects (avg 0 B)"), "{line}");
    assert!(line.contains("(cap unlimited)"), "{line}");
    assert!(line.contains("closure/raw 0"), "{line}");
}

// ── Collector internals (Phase 34) ───────────────────────────────────────────────

#[test]
fn freelist_take_first_fit_splits_remainder() {
    // First span large enough wins; the unused tail stays on the free list.
    let mut free = vec![(0x1000usize, 16usize), (0x2000usize, 64usize)];
    let got = freelist_take(&mut free, 32);
    assert_eq!(
        got,
        Some(0x2000),
        "first fit should skip the too-small 16 B span"
    );
    assert_eq!(
        free,
        vec![(0x1000, 16), (0x2020, 32)],
        "remainder of the 64 B span (0x2000+32, 64-32) stays on the list"
    );
    // The 16 B span still cannot satisfy 32 B.
    assert_eq!(freelist_take(&mut free, 48), None);
}

#[test]
fn freelist_take_exact_fit_removes_span() {
    let mut free = vec![(0x4000usize, 32usize)];
    assert_eq!(freelist_take(&mut free, 32), Some(0x4000));
    assert!(free.is_empty(), "an exact fit consumes the whole span");
}

#[test]
fn coalesce_free_merges_only_contiguous_spans() {
    // Two adjacent spans in one region merge; a span in another region (gap) does
    // not, even after sorting.
    let mut free = vec![
        (0x2000usize, 16usize),
        (0x1000usize, 16usize),
        (0x1010usize, 32usize),
    ];
    coalesce_free(&mut free);
    assert_eq!(
        free,
        vec![(0x1000, 48), (0x2000, 16)],
        "0x1000+16 == 0x1010 coalesces; 0x2000 has a gap and stays separate"
    );
}

#[test]
fn find_object_resolves_exact_and_interior_pointers() {
    let mut objects = BTreeMap::new();
    objects.insert(0x1000usize, 24usize);
    objects.insert(0x2000usize, 16usize);
    assert_eq!(find_object(&objects, 0x1000), Some(0x1000), "exact start");
    assert_eq!(
        find_object(&objects, 0x1008),
        Some(0x1000),
        "interior pointer pins object"
    );
    assert_eq!(
        find_object(&objects, 0x1017),
        Some(0x1000),
        "last byte is interior"
    );
    assert_eq!(
        find_object(&objects, 0x1018),
        None,
        "one past the end is not interior"
    );
    assert_eq!(
        find_object(&objects, 0x0fff),
        None,
        "below the lowest object"
    );
}

#[test]
fn ptr_in_chunks_range_check() {
    let bounds = [(0x1000usize, 0x2000usize), (0x5000usize, 0x6000usize)];
    assert!(ptr_in_chunks(&bounds, 0x1000), "inclusive low bound");
    assert!(ptr_in_chunks(&bounds, 0x1fff));
    assert!(!ptr_in_chunks(&bounds, 0x2000), "exclusive high bound");
    assert!(!ptr_in_chunks(&bounds, 0x3000), "between chunks");
    assert!(ptr_in_chunks(&bounds, 0x5500));
}

#[test]
fn mark_candidate_marks_live_object_once() {
    let mut objects = BTreeMap::new();
    objects.insert(0x1000usize, 24usize);
    let bounds = [(0x1000usize, 0x2000usize)];
    let mut marked = HashSet::new();
    let mut work = Vec::new();

    // A non-pointer (outside chunks) does nothing.
    mark_candidate(0x9999, &objects, &bounds, &mut marked, &mut work);
    assert!(marked.is_empty());

    // An interior pointer marks the object and enqueues it exactly once.
    mark_candidate(0x1008, &objects, &bounds, &mut marked, &mut work);
    mark_candidate(0x1000, &objects, &bounds, &mut marked, &mut work);
    assert_eq!(marked.len(), 1);
    assert_eq!(work, vec![0x1000]);
}
