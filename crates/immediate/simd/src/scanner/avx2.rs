use std::arch::x86_64::{
    __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256,
    _mm256_set1_epi8,
};

use super::{RawMasks, scalar::classify_chunk_scalar};

#[target_feature(enable = "avx2")]
pub(super) unsafe fn classify_chunk_avx2(chunk: &[u8]) -> RawMasks {
    debug_assert!(chunk.len() <= 64);

    let mut masks = RawMasks::default();
    let mut offset = 0;

    while offset + 32 <= chunk.len() {
        // SAFETY: `offset + 32 <= chunk.len()` guarantees 32 readable bytes;
        // `_mm256_loadu_si256` is unaligned. The caller guarantees AVX2.
        let bytes = unsafe { _mm256_loadu_si256(chunk.as_ptr().add(offset) as *const __m256i) };

        let quote = _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'"' as i8));
        let backslash = _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'\\' as i8));

        let whitespace = _mm256_or_si256(
            _mm256_or_si256(
                _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b' ' as i8)),
                _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'\n' as i8)),
            ),
            _mm256_or_si256(
                _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'\r' as i8)),
                _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'\t' as i8)),
            ),
        );

        let structural = _mm256_or_si256(
            _mm256_or_si256(
                _mm256_or_si256(
                    _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'{' as i8)),
                    _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'}' as i8)),
                ),
                _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'[' as i8)),
            ),
            _mm256_or_si256(
                _mm256_or_si256(
                    _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b']' as i8)),
                    _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'=' as i8)),
                ),
                _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b';' as i8)),
            ),
        );

        // `_mm256_movemask_epi8` packs the MSB of each of the 32 lanes into bit i
        // for lane i. Cast through `u32` to preserve exactly the low 32 mask bits
        // before widening to the scanner's `u64` masks.
        masks.quote |= (_mm256_movemask_epi8(quote) as u32 as u64) << offset;
        masks.backslash |= (_mm256_movemask_epi8(backslash) as u32 as u64) << offset;
        masks.whitespace |= (_mm256_movemask_epi8(whitespace) as u32 as u64) << offset;
        masks.structural |= (_mm256_movemask_epi8(structural) as u32 as u64) << offset;
        offset += 32;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avx2_classifier_matches_scalar_for_short_chunks() {
        if std::is_x86_feature_detected!("avx2") {
            for input in ["", "{", "{ a = 1; }", "\"quote\\\"\"", " \n\r\t{}[]=;#atom"] {
                // SAFETY: The test branch checks AVX2 support immediately
                // before calling the AVX2-only classifier.
                assert_eq!(
                    unsafe { classify_chunk_avx2(input.as_bytes()) },
                    classify_chunk_scalar(input.as_bytes())
                );
            }
        }
    }

    #[test]
    fn avx2_classifier_matches_scalar_for_full_chunks_and_tails() {
        if std::is_x86_feature_detected!("avx2") {
            let input = concat!(
                "{ name = \"brace } semi ; quote \\\"\"; tags = [#fast-path; #none;]; }",
                "\nextra = true;"
            );

            for len in 1..=64 {
                let chunk = &input.as_bytes()[..len];
                // SAFETY: The test branch checks AVX2 support immediately
                // before calling the AVX2-only classifier.
                assert_eq!(
                    unsafe { classify_chunk_avx2(chunk) },
                    classify_chunk_scalar(chunk)
                );
            }
        }
    }
}
