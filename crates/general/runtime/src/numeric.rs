use super::*;

// ── Numeric bridge ABI ─────────────────────────────────────────────────────────

pub(crate) fn numeric_runtime_error(message: &str) -> ! {
    eprintln!("zutai runtime error: {message}");
    std::process::exit(1);
}

pub(crate) fn int_from_float(value: f64, range_message: &'static str) -> i64 {
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

pub(crate) fn float_bin(lhs: i64, rhs: i64, op: impl FnOnce(f64, f64) -> f64) -> i64 {
    op(f64::from_bits(lhs as u64), f64::from_bits(rhs as u64)).to_bits() as i64
}

pub(crate) fn float_cmp(lhs: i64, rhs: i64, op: impl FnOnce(f64, f64) -> bool) -> i64 {
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

/// Rust's shortest round-trip float repr, matching the interpreter's `Display`
/// (`crates/general/eval/src/value.rs`): bare `inf`/`-inf`/`NaN`, and `.0`
/// appended only for finite integral values.
pub(crate) fn fmt_float(x: f64) -> String {
    let s = format!("{x:?}");
    if !x.is_finite() || s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

pub(crate) fn to_f64_p32<const ES: u32>(bits: u64) -> f64 {
    Posit::<32, ES, i64>::from_bits(bits as u32 as i32 as i64).round_into()
}

pub(crate) fn to_f64_p64<const ES: u32>(bits: u64) -> f64 {
    Posit::<64, ES, i128>::from_bits(bits as i64 as i128).round_into()
}

pub(crate) type P32<const ES: u32> = Posit<32, ES, i64>;

pub(crate) fn p32_from_abi<const ES: u32>(bits: i32) -> P32<ES> {
    P32::<ES>::from_bits(bits as i64)
}

pub(crate) fn p32_to_abi<const ES: u32>(value: P32<ES>) -> i32 {
    value.to_bits() as i32
}

pub(crate) fn p32_add<const ES: u32>(lhs: i32, rhs: i32) -> i32 {
    p32_to_abi(p32_from_abi::<ES>(lhs) + p32_from_abi::<ES>(rhs))
}

#[unsafe(export_name = "zutai.posit32e2.add")]
pub extern "C" fn posit32e2_add(lhs: i32, rhs: i32) -> i32 {
    p32_add::<2>(lhs, rhs)
}

pub(crate) fn fmt_posit(bits: i64, nbits: i64, es: i64) -> String {
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
