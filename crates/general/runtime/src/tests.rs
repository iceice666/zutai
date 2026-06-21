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
    assert_eq!(render_str(variant_new(0, 0), &opt_d), "#none");
    assert_eq!(render_str(variant_new(1, 7), &opt_d), "#some (7)");

    let maybe_d = [DESC_MAYBE, int_d.as_ptr() as i64];
    assert_eq!(render_str(variant_new(0, 0), &maybe_d), "#absent");
    assert_eq!(render_str(variant_new(1, 9), &maybe_d), "#present (9)");
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
    assert_eq!(render_str(variant_new(0, 0), &union_d), "#red");
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
    assert_eq!(coalesce(variant_new(0, 0), 99), 99);
    assert_eq!(coalesce(variant_new(1, 7), 99), 7);
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
