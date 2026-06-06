use std::arch::aarch64::{
    uint8x16_t, vaddv_u8, vandq_u8, vceqq_u8, vdupq_n_u8, vget_high_u8, vget_low_u8, vld1q_u8,
    vorrq_u8,
};

use super::{RawMasks, scalar::classify_chunk_scalar};

#[inline(always)]
pub(super) fn classify_chunk_neon(chunk: &[u8]) -> RawMasks {
    debug_assert!(chunk.len() <= 64);

    let mut masks = RawMasks::default();
    let mut offset = 0;

    while offset + 16 <= chunk.len() {
        let bytes = unsafe { vld1q_u8(chunk.as_ptr().add(offset)) };

        let quote = eq(bytes, b'"');
        let backslash = eq(bytes, b'\\');

        let whitespace = or(
            or(eq(bytes, b' '), eq(bytes, b'\n')),
            or(eq(bytes, b'\r'), eq(bytes, b'\t')),
        );

        let structural = or(
            or(or(eq(bytes, b'{'), eq(bytes, b'}')), eq(bytes, b'[')),
            or(or(eq(bytes, b']'), eq(bytes, b'=')), eq(bytes, b';')),
        );

        masks.quote |= lane_mask(quote) << offset;
        masks.backslash |= lane_mask(backslash) << offset;
        masks.whitespace |= lane_mask(whitespace) << offset;
        masks.structural |= lane_mask(structural) << offset;

        offset += 16;
    }

    if offset < chunk.len() {
        let tail = classify_chunk_scalar(&chunk[offset..]);
        masks.quote |= tail.quote << offset;
        masks.backslash |= tail.backslash << offset;
        masks.whitespace |= tail.whitespace << offset;
        masks.structural |= tail.structural << offset;
    }

    masks
}

#[inline(always)]
fn eq(byte: uint8x16_t, value: u8) -> uint8x16_t {
    unsafe { vceqq_u8(byte, vdupq_n_u8(value)) }
}

#[inline(always)]
fn or(left: uint8x16_t, right: uint8x16_t) -> uint8x16_t {
    unsafe { vorrq_u8(left, right) }
}

#[inline(always)]
fn lane_mask(matches: uint8x16_t) -> u64 {
    const BIT_WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];

    // Comparison lanes are 0xff for matches and 0x00 otherwise. Masking
    // by per-lane powers of two lets horizontal byte sums form movemasks
    // for the low and high 8-byte halves without spilling the lane.
    let weighted = unsafe { vandq_u8(matches, vld1q_u8(BIT_WEIGHTS.as_ptr())) };
    let low = unsafe { vaddv_u8(vget_low_u8(weighted)) };
    let high = unsafe { vaddv_u8(vget_high_u8(weighted)) };

    u64::from(low) | (u64::from(high) << 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neon_classifier_matches_scalar_for_short_chunks() {
        for input in ["", "{", "{ a = 1; }", "\"quote\\\"\"", " \n\r\t{}[]=;#atom"] {
            assert_eq!(
                classify_chunk_neon(input.as_bytes()),
                classify_chunk_scalar(input.as_bytes())
            );
        }
    }

    #[test]
    fn neon_classifier_matches_scalar_for_full_chunks_and_tails() {
        let input = concat!(
            "{ name = \"brace } semi ; quote \\\"\"; tags = [#fast-path; #none;]; }",
            "\nextra = true;"
        );

        for len in 1..=64 {
            let chunk = &input.as_bytes()[..len];
            assert_eq!(classify_chunk_neon(chunk), classify_chunk_scalar(chunk));
        }
    }
}
