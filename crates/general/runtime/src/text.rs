use super::*;

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

pub(crate) fn quoted_text(value: &str) -> String {
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

pub(crate) fn optional_runtime_value(value: Option<i64>) -> i64 {
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

pub(crate) fn text_from_bytes(bytes: &[u8]) -> i64 {
    let dst = arena_alloc(bytes.len());
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
    }
    text_from_global(dst as i64, bytes.len() as i64)
}

pub(crate) fn text_from_string(s: String) -> i64 {
    text_from_bytes(s.as_bytes())
}

pub(crate) fn runtime_error(message: &str) -> ! {
    eprintln!("zutai host boundary error: {message}");
    std::process::exit(1);
}

pub(crate) fn optional_text(value: Option<String>) -> i64 {
    match value {
        Some(text) => {
            let tuple = tuple_new(1);
            tuple_set(tuple, 0, text_from_string(text));
            variant_new(1, tuple)
        }
        None => text_from_bytes(b"none"),
    }
}
