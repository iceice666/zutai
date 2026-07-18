use super::*;

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

pub(crate) fn push_quoted(out: &mut String, s: &str) {
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
pub(crate) unsafe fn name_at(desc: *const i64, off: usize) -> &'static str {
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
pub(crate) unsafe fn value_equal(lhs: i64, rhs: i64, desc: *const i64) -> bool {
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
pub(crate) unsafe fn wrapper_is_present(value: i64) -> bool {
    unsafe { header_tag(word(value, 0)) == TAG_VARIANT }
}

/// # Safety
/// `value` must be a present Optional/Maybe storage value.
pub(crate) unsafe fn wrapper_payload(value: i64) -> i64 {
    unsafe { word(word(value, 2), 1) }
}

/// # Safety
/// `lhs` and `rhs` must be Optional/Maybe storage values with payloads described
/// by `inner_desc`.
pub(crate) unsafe fn wrapper_storage_equal(lhs: i64, rhs: i64, inner_desc: *const i64) -> bool {
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
pub(crate) unsafe fn record_equal(lhs: i64, rhs: i64, desc: *const i64) -> bool {
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
pub(crate) unsafe fn json_value(
    value: i64,
    desc: *const i64,
) -> Result<serde_json::Value, &'static str> {
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
pub(crate) unsafe fn text_json_string(value: i64) -> Result<String, &'static str> {
    unsafe {
        str::from_utf8(text_parts(value))
            .map(str::to_string)
            .map_err(|_| "cannot serialize non-UTF-8 text to JSON")
    }
}

/// # Safety
/// `value` must be Optional/Maybe storage for `inner_desc`.
pub(crate) unsafe fn json_wrapper(
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
pub(crate) unsafe fn json_record(
    value: i64,
    desc: *const i64,
) -> Result<serde_json::Value, &'static str> {
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
pub(crate) unsafe fn json_tuple_array(
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
pub(crate) unsafe fn json_tag_payload(
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
pub(crate) unsafe fn render_record_fields(
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
pub(crate) unsafe fn render(out: &mut String, value: i64, desc: *const i64) {
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
