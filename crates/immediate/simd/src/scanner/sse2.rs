use std::arch::x86_64::{
    __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_or_si128, _mm_set1_epi8,
};

use super::{RawMasks, scalar::classify_chunk_scalar};

#[inline(always)]
pub(super) fn classify_chunk_sse2(chunk: &[u8]) -> RawMasks {
    debug_assert!(chunk.len() <= 64);

    let mut masks = RawMasks::default();
    let mut offset = 0;

    while offset + 16 <= chunk.len() {
        // SAFETY: `offset + 16 <= chunk.len()` guarantees 16 readable bytes;
        // `_mm_loadu_si128` is unaligned. SSE2 is baseline on x86_64.
        let bytes = unsafe { _mm_loadu_si128(chunk.as_ptr().add(offset) as *const __m128i) };

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
fn eq(bytes: __m128i, value: u8) -> __m128i {
    // SAFETY: SSE2 baseline on x86_64. `value` is ASCII (< 128); `as i8` is a
    // lossless bit reinterpretation for byte-equality compare.
    unsafe { _mm_cmpeq_epi8(bytes, _mm_set1_epi8(value as i8)) }
}

#[inline(always)]
fn or(left: __m128i, right: __m128i) -> __m128i {
    // SAFETY: SSE2 baseline on x86_64.
    unsafe { _mm_or_si128(left, right) }
}

#[inline(always)]
fn lane_mask(matches: __m128i) -> u64 {
    // `_mm_movemask_epi8` packs the MSB of each of the 16 lanes (0xff on match,
    // 0x00 otherwise) into bit i for lane i — same bit order as the scalar
    // `<< bit` convention. Mask to the low 16 bits and zero-extend.
    // SAFETY: SSE2 baseline on x86_64.
    (unsafe { _mm_movemask_epi8(matches) } as u16) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse2_classifier_matches_scalar_for_short_chunks() {
        for input in ["", "{", "{ a = 1; }", "\"quote\\\"\"", " \n\r\t{}[]=;#atom"] {
            assert_eq!(
                classify_chunk_sse2(input.as_bytes()),
                classify_chunk_scalar(input.as_bytes())
            );
        }
    }

    #[test]
    fn sse2_classifier_matches_scalar_for_full_chunks_and_tails() {
        let input = concat!(
            "{ name = \"brace } semi ; quote \\\"\"; tags = [#fast-path; #none;]; }",
            "\nextra = true;"
        );

        for len in 1..=64 {
            let chunk = &input.as_bytes()[..len];
            assert_eq!(classify_chunk_sse2(chunk), classify_chunk_scalar(chunk));
        }
    }
}
