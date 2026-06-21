//! Crate-private SIMD helper for locating the next string-special byte.
//!
//! [`Parser::parse_string`](crate::parser) copies literal spans between the
//! bytes that actually need handling: the closing quote `"`, an escape `\`, or a
//! raw control byte (`< 0x20`). Everything else — including multi-byte UTF-8 — is
//! literal text. This module finds the offset of the next such byte in bulk so
//! the parser stops walking string contents one byte at a time.
//!
//! Backend selection mirrors `scanner/mod.rs`: AVX2 when the running process
//! supports it, SSE2 as the x86_64 baseline, NEON on aarch64, and a scalar
//! fallback elsewhere. The choice is made once per parser construction and stored
//! as a function pointer, so AVX2 detection never runs inside the scanning loop.

/// Finds the first `"`, `\`, or byte `< 0x20` in `bytes`, returning its index
/// relative to the start of the slice, or `None` when the slice has none.
pub(crate) type StringSpecialFinder = fn(&[u8]) -> Option<usize>;

/// The backend used when AVX2 is not selected: SSE2 on x86_64, NEON on aarch64,
/// scalar elsewhere. AVX2 is chosen at runtime in [`select_string_special_finder`].
#[cfg(target_arch = "x86_64")]
const DEFAULT_FINDER: StringSpecialFinder = find_string_special_sse2;
#[cfg(target_arch = "aarch64")]
const DEFAULT_FINDER: StringSpecialFinder = find_string_special_neon;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const DEFAULT_FINDER: StringSpecialFinder = find_string_special_scalar;

/// Picks the string-special finder for the running process using the same target
/// policy as `scanner/mod.rs`. Call once and reuse the returned pointer.
pub(crate) fn select_string_special_finder() -> StringSpecialFinder {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return find_string_special_avx2_checked;
        }
    }

    DEFAULT_FINDER
}

/// Scalar reference: also used for the sub-lane tail of every SIMD backend.
fn find_string_special_scalar(bytes: &[u8]) -> Option<usize> {
    bytes
        .iter()
        .position(|&byte| byte == b'"' || byte == b'\\' || byte < 0x20)
}

#[cfg(target_arch = "x86_64")]
fn find_string_special_sse2(bytes: &[u8]) -> Option<usize> {
    use std::arch::x86_64::{
        __m128i, _mm_cmpeq_epi8, _mm_cmpgt_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_or_si128,
        _mm_set1_epi8, _mm_xor_si128,
    };

    let mut offset = 0;
    while offset + 16 <= bytes.len() {
        // SAFETY: `offset + 16 <= bytes.len()` guarantees 16 readable bytes for
        // the unaligned load. SSE2 is baseline on x86_64, so every intrinsic
        // here is available for this target.
        let mask = unsafe {
            let chunk = _mm_loadu_si128(bytes.as_ptr().add(offset) as *const __m128i);
            let quote = _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'"' as i8));
            let backslash = _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'\\' as i8));
            // Unsigned `byte < 0x20`: XOR-bias both operands by 0x80 so a signed
            // compare matches the unsigned ordering. UTF-8 bytes >= 0x80 then sort
            // high and are never mistaken for control bytes.
            let biased = _mm_xor_si128(chunk, _mm_set1_epi8(0x80u8 as i8));
            let control = _mm_cmpgt_epi8(_mm_set1_epi8((0x20u8 ^ 0x80u8) as i8), biased);
            let special = _mm_or_si128(_mm_or_si128(quote, backslash), control);
            _mm_movemask_epi8(special) as u16
        };
        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    find_string_special_scalar(&bytes[offset..]).map(|index| offset + index)
}

/// Safe wrapper returned by [`select_string_special_finder`] once AVX2 support is
/// confirmed; it holds the single unsafe call into [`find_string_special_avx2`].
#[cfg(target_arch = "x86_64")]
fn find_string_special_avx2_checked(bytes: &[u8]) -> Option<usize> {
    // SAFETY: `select_string_special_finder` only returns this wrapper after
    // `is_x86_feature_detected!("avx2")` confirmed AVX2 support for the process.
    unsafe { find_string_special_avx2(bytes) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn find_string_special_avx2(bytes: &[u8]) -> Option<usize> {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_cmpgt_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
        _mm256_or_si256, _mm256_set1_epi8, _mm256_xor_si256,
    };

    let mut offset = 0;
    while offset + 32 <= bytes.len() {
        // SAFETY: `offset + 32 <= bytes.len()` guarantees 32 readable bytes for
        // the unaligned load. The `#[target_feature(enable = "avx2")]` attribute
        // makes the remaining AVX2 intrinsics safe to call here; the caller
        // guarantees AVX2 support for the process.
        let chunk = unsafe { _mm256_loadu_si256(bytes.as_ptr().add(offset) as *const __m256i) };
        let quote = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8));
        let backslash = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\\' as i8));
        // Unsigned `byte < 0x20` via XOR bias, as in the SSE2 path.
        let biased = _mm256_xor_si256(chunk, _mm256_set1_epi8(0x80u8 as i8));
        let control = _mm256_cmpgt_epi8(_mm256_set1_epi8((0x20u8 ^ 0x80u8) as i8), biased);
        let special = _mm256_or_si256(_mm256_or_si256(quote, backslash), control);
        let mask = _mm256_movemask_epi8(special) as u32;
        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 32;
    }

    find_string_special_scalar(&bytes[offset..]).map(|index| offset + index)
}

#[cfg(target_arch = "aarch64")]
fn find_string_special_neon(bytes: &[u8]) -> Option<usize> {
    use std::arch::aarch64::{vceqq_u8, vcltq_u8, vdupq_n_u8, vld1q_u8, vorrq_u8};

    let mut offset = 0;
    while offset + 16 <= bytes.len() {
        // SAFETY: `offset + 16 <= bytes.len()` guarantees 16 readable bytes for
        // the unaligned load. NEON is baseline on aarch64.
        let mask = unsafe {
            let chunk = vld1q_u8(bytes.as_ptr().add(offset));
            let quote = vceqq_u8(chunk, vdupq_n_u8(b'"'));
            let backslash = vceqq_u8(chunk, vdupq_n_u8(b'\\'));
            // `vcltq_u8` is an unsigned compare, so UTF-8 bytes >= 0x80 never
            // count as control bytes.
            let control = vcltq_u8(chunk, vdupq_n_u8(0x20));
            neon_lane_mask(vorrq_u8(vorrq_u8(quote, backslash), control))
        };
        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    find_string_special_scalar(&bytes[offset..]).map(|index| offset + index)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn neon_lane_mask(matches: std::arch::aarch64::uint8x16_t) -> u64 {
    use std::arch::aarch64::{vaddv_u8, vandq_u8, vget_high_u8, vget_low_u8, vld1q_u8};

    const BIT_WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];

    // Comparison lanes are 0xff on match and 0x00 otherwise. Weighting by per-lane
    // powers of two then horizontally summing each 8-byte half forms a movemask
    // in the same bit order as the scalar `<< bit` convention.
    let weighted = unsafe { vandq_u8(matches, vld1q_u8(BIT_WEIGHTS.as_ptr())) };
    let low = unsafe { vaddv_u8(vget_low_u8(weighted)) };
    let high = unsafe { vaddv_u8(vget_high_u8(weighted)) };

    u64::from(low) | (u64::from(high) << 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent corpus exercising every special byte at the lane boundaries
    /// that SIMD tail handling is most likely to get wrong.
    fn cases() -> Vec<Vec<u8>> {
        const SPECIALS: [u8; 8] = [b'"', b'\\', 0x00, b'\n', b'\r', b'\t', 0x1F, 0x20];

        let mut cases = Vec::new();
        for len in 0..=128usize {
            let plain: Vec<u8> = (0..len).map(|i| b'a' + (i % 26) as u8).collect();
            cases.push(plain.clone());

            if len == 0 {
                continue;
            }
            for &special in &SPECIALS {
                for &pos in &[0usize, len / 2, len - 1] {
                    let mut variant = plain.clone();
                    variant[pos] = special;
                    cases.push(variant);
                }
            }
        }

        // Multi-byte UTF-8: every byte >= 0x80 must stay literal, never control.
        cases.push("café".as_bytes().to_vec());
        cases.push("a café au lait costs 5€ — naïve façade".as_bytes().to_vec());
        cases.push("café\"".as_bytes().to_vec());
        cases.push("façade\\escape".as_bytes().to_vec());

        cases
    }

    fn assert_matches_scalar(finder: StringSpecialFinder) {
        for case in cases() {
            assert_eq!(
                finder(&case),
                find_string_special_scalar(&case),
                "finder disagreed with scalar for {case:?}"
            );
        }
    }

    /// Pins the scalar reference itself to hand-checked truth so the SIMD/scalar
    /// parity tests below inherit a correct oracle.
    #[test]
    fn scalar_reference_known_values() {
        assert_eq!(find_string_special_scalar(b""), None);
        assert_eq!(find_string_special_scalar(b"plain text 123"), None);
        assert_eq!(find_string_special_scalar(b"ab\"cd"), Some(2));
        assert_eq!(find_string_special_scalar(b"ab\\cd"), Some(2));
        assert_eq!(find_string_special_scalar(b"ab\ncd"), Some(2));
        assert_eq!(find_string_special_scalar(b"ab\rcd"), Some(2));
        assert_eq!(find_string_special_scalar(b"ab\tcd"), Some(2));
        assert_eq!(find_string_special_scalar(b"ab\0cd"), Some(2));
        assert_eq!(find_string_special_scalar(b"\x1fxy"), Some(0)); // 0x1F is control
        assert_eq!(find_string_special_scalar(b" xy"), None); // 0x20 is not control
        assert_eq!(find_string_special_scalar("café".as_bytes()), None);
    }

    #[test]
    fn selected_finder_matches_scalar() {
        assert_matches_scalar(select_string_special_finder());
    }

    #[test]
    fn utf8_high_bytes_are_never_control() {
        let high_bytes: Vec<u8> = (0x80u8..=0xFF).collect();
        assert_eq!(find_string_special_scalar(&high_bytes), None);
        assert_eq!(select_string_special_finder()(&high_bytes), None);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn sse2_matches_scalar() {
        assert_matches_scalar(find_string_special_sse2);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_matches_scalar() {
        if std::is_x86_feature_detected!("avx2") {
            assert_matches_scalar(find_string_special_avx2_checked);
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn neon_matches_scalar() {
        assert_matches_scalar(find_string_special_neon);
    }
}
