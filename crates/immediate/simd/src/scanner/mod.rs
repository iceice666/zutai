mod scalar;

#[cfg(not(target_arch = "aarch64"))]
use scalar::classify_chunk_scalar as classify_chunk;

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(target_arch = "aarch64")]
use neon::classify_chunk_neon as classify_chunk;

use crate::error::{ParseError, ParseErrorKind, error};

// A run of k backslashes starting at bit i (even position) with odd length k
// ends at bit i+k-1; the escaped byte is at i+k (an odd position → ODD_BITS).
// Vice versa for odd-start runs.
const EVEN_BITS: u64 = 0x5555_5555_5555_5555;
const ODD_BITS: u64 = !EVEN_BITS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralIndex {
    pub structural: Vec<usize>,
    pub pseudo_structural: Vec<usize>,
    pub significant: Vec<usize>,
    pub chunks: Vec<ChunkScan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkScan {
    pub base: usize,

    /// Structural characters outside strings.
    pub structural_mask: u64,

    /// Unescaped quote delimiters only, not escaped quote bytes inside strings.
    pub quote_mask: u64,

    /// All backslash bytes, including inside strings.
    pub backslash_mask: u64,

    /// All whitespace bytes, including inside strings.
    pub whitespace_mask: u64,

    /// Token starts plus opening string quotes.
    pub pseudo_structural_mask: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RawMasks {
    pub(super) quote: u64,
    pub(super) backslash: u64,
    pub(super) whitespace: u64,
    pub(super) structural: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScanState {
    in_string: bool,
    // Bit 0: did an odd-length backslash run end exactly at the last byte of
    // the previous chunk? (i.e., is the first byte of the next chunk escaped?)
    escape_carry: u64,
    previous_was_boundary: bool,
}

impl Default for ScanState {
    fn default() -> Self {
        Self {
            in_string: false,
            escape_carry: 0,
            previous_was_boundary: true,
        }
    }
}

pub(crate) struct Scanner<'a> {
    input: &'a str,
    bytes: &'a [u8],
}

impl<'a> Scanner<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
        }
    }

    pub(crate) fn scan(self) -> Result<StructuralIndex, ParseError> {
        let mut structural = Vec::new();
        let mut pseudo_structural = Vec::new();
        let mut significant = Vec::new();
        let mut chunks = Vec::with_capacity(self.bytes.len().div_ceil(64));

        let mut state = ScanState::default();

        for (chunk_index, chunk) in self.bytes.chunks(64).enumerate() {
            let base = chunk_index * 64;
            let len = chunk.len();

            // Stage 1: raw per-byte classification. Architecture-specific
            // SIMD can accelerate this without changing later scanner stages.
            let raw = classify_chunk(chunk);

            // Stage 2a: quotes that actually toggle string state.
            let quote_mask =
                unescaped_quote_mask(raw.quote, raw.backslash, len, &mut state.escape_carry);

            // Stage 2b: bytes occupied by string regions, including both
            // opening and closing delimiter quotes.
            let (string_region_mask, opening_quote_mask) =
                compute_string_region_mask(quote_mask, len, &mut state.in_string);

            let valid_mask = low_bits(len);
            let outside_plain_mask = valid_mask & !string_region_mask;

            // Stage 2c: structurals only count outside strings.
            let structural_mask = raw.structural & outside_plain_mask;

            let outside_whitespace_mask = raw.whitespace & outside_plain_mask;
            let ordinary_token_mask = outside_plain_mask & !(raw.structural | raw.whitespace);
            let boundary_mask = structural_mask | outside_whitespace_mask;

            let pseudo_structural_mask = compute_pseudo_structural_mask(
                ordinary_token_mask,
                opening_quote_mask,
                boundary_mask,
                len,
                &mut state.previous_was_boundary,
            );

            // Stage 3: sparse scalar emission from masks.
            emit_bits(structural_mask, base, &mut structural);
            emit_bits(pseudo_structural_mask, base, &mut pseudo_structural);
            emit_bits(
                structural_mask | pseudo_structural_mask,
                base,
                &mut significant,
            );

            chunks.push(ChunkScan {
                base,
                structural_mask,
                quote_mask,
                backslash_mask: raw.backslash,
                whitespace_mask: raw.whitespace,
                pseudo_structural_mask,
            });
        }

        if state.in_string {
            return Err(error(self.input.len(), ParseErrorKind::UnclosedString));
        }

        Ok(StructuralIndex {
            structural,
            pseudo_structural,
            significant,
            chunks,
        })
    }
}

// ---------------------------------------------------------------------------
// prefix_xor
// ---------------------------------------------------------------------------

/// Prefix XOR of the bits of `x`: bit i of the result = XOR of bits 0..=i of
/// `x`. Equivalently, carryless multiplication of `x` by the all-ones
/// polynomial. Used to turn a set of quote positions into a string-interior
/// mask.
///
/// On aarch64 with the `aes` target feature (`+aes` implies PMULL), this uses
/// `vmull_p64`. Apple Silicon enables `aes` by default; Linux aarch64 generic
/// targets need `RUSTFLAGS="-C target-feature=+aes"` or `-C target-cpu=native`
/// to activate this path. Otherwise the 6-step scalar shift ladder is used.
#[inline(always)]
fn prefix_xor(x: u64) -> u64 {
    #[cfg(all(target_arch = "aarch64", target_feature = "aes"))]
    {
        // SAFETY: target_feature = "aes" guarantees PMULL is available.
        unsafe { prefix_xor_pmull(x) }
    }
    #[cfg(not(all(target_arch = "aarch64", target_feature = "aes")))]
    {
        prefix_xor_scalar(x)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[inline(always)]
fn prefix_xor_scalar(mut x: u64) -> u64 {
    x ^= x << 1;
    x ^= x << 2;
    x ^= x << 4;
    x ^= x << 8;
    x ^= x << 16;
    x ^= x << 32;
    x
}

#[cfg(all(target_arch = "aarch64", target_feature = "aes"))]
#[target_feature(enable = "aes,neon")]
unsafe fn prefix_xor_pmull(x: u64) -> u64 {
    use std::arch::aarch64::{vgetq_lane_u64, vmull_p64, vreinterpretq_u64_p128};
    use std::mem::transmute;
    // Carryless multiply of x by all-ones (the "carry-less shift-and-XOR"
    // polynomial) equals prefix XOR. Take only the low 64 bits of the 128-bit
    // PMULL result.
    // SAFETY: target_feature = "aes" guarantees PMULL is available; transmute
    // between u64 and poly64_t (same size, no invalid bit patterns).
    unsafe {
        let result = vmull_p64(transmute(x), transmute(!0u64));
        vgetq_lane_u64(vreinterpretq_u64_p128(result), 0)
    }
}

// ---------------------------------------------------------------------------
// Stage 2a: find unescaped quote positions (odd-backslash-run detection)
// ---------------------------------------------------------------------------

/// Returns the mask of quote bytes not preceded by an odd-length run of
/// backslashes — the quotes that actually toggle string state.
///
/// `escape_carry` (bit 0) is 1 if an odd-length backslash run ended at the
/// last byte of the previous chunk (so the first byte here is escaped). It is
/// updated to the carry for the next chunk.
#[inline(always)]
fn unescaped_quote_mask(quote: u64, backslash: u64, len: usize, escape_carry: &mut u64) -> u64 {
    // First byte of each contiguous backslash run (a run can't start where the
    // previous byte was also a backslash, or where the incoming escape carry
    // applies).
    let starts = backslash & !(backslash << 1 | *escape_carry);
    let even_starts = starts & EVEN_BITS;
    let odd_starts = starts & ODD_BITS;

    // Adding start-of-run bits to the run propagates a carry to one past the
    // run's last byte. That carry bit position tells us the parity.
    let (even_carries, _even_overflow) = backslash.overflowing_add(even_starts);

    // The incoming escape_carry represents a run that started at the virtual
    // odd position -1 in the previous chunk and continues into this one.
    // Adding it as a carry-in to the odd_starts sum correctly extends that
    // run through any leading backslashes in this chunk.
    let (tmp_odd, odd_ov1) = backslash.overflowing_add(odd_starts);
    let (odd_carries, odd_ov2) = tmp_odd.overflowing_add(*escape_carry);
    let odd_overflow = odd_ov1 | odd_ov2;

    // Strip carries that land back on backslash bytes (inside the run).
    let even_carry_ends = even_carries & !backslash;
    let odd_carry_ends = odd_carries & !backslash;

    // even_starts + odd_length → carry lands at ODD position → escape byte is ODD
    // odd_starts  + odd_length → carry lands at EVEN position → escape byte is EVEN
    let escape_mask = (even_carry_ends & ODD_BITS) | (odd_carry_ends & EVEN_BITS);

    // Carry-out for the next chunk: did an odd-length run straddle the boundary?
    // Position `len` is even or odd, and only one of the two additions produces
    // a carry there for an odd-length run:
    //   len even → odd_starts (odd-position-start) runs escape at even positions
    //   len odd  → even_starts runs escape at odd positions
    *escape_carry = if len == 64 {
        odd_overflow as u64 // len=64 is always even
    } else if len % 2 == 0 {
        (odd_carries >> len) & 1
    } else {
        (even_carries >> len) & 1
    };

    quote & !escape_mask
}

// ---------------------------------------------------------------------------
// Stage 2b: string region mask via prefix-XOR
// ---------------------------------------------------------------------------

/// Computes the mask of bytes inside string regions (including both delimiter
/// quotes) and the mask of opening-quote positions.
///
/// `in_string` carries whether the scanner was inside a string at the start
/// of this chunk and is updated to reflect the end of the chunk.
///
/// Both returned masks are guaranteed to have no bits set beyond position
/// `len - 1`.
#[inline(always)]
fn compute_string_region_mask(uq: u64, len: usize, in_string: &mut bool) -> (u64, u64) {
    // prefix_xor(uq): bit i = 1 iff an odd number of unescaped quotes exist
    // in [0, i]. Toggling by carry_in (all-ones when already in a string)
    // accounts for the state carried across chunk boundaries.
    let carry_in: u64 = if *in_string { !0u64 } else { 0 };
    let prefix = prefix_xor(uq) ^ carry_in;

    // After the XOR: prefix=1 at opening quotes and throughout their interiors;
    // prefix=0 at closing quotes and outside strings. Combining with uq itself
    // (which marks closing quotes) gives the full string region.
    let valid = low_bits(len);
    let opening_quote_mask = prefix & uq & valid;
    let string_region_mask = (prefix | uq) & valid;

    // Bit len-1 of prefix: 1 means we finish this chunk inside an open string.
    *in_string = len > 0 && (prefix >> (len - 1)) & 1 != 0;

    (string_region_mask, opening_quote_mask)
}

// ---------------------------------------------------------------------------
// Stage 2c: pseudo-structural (token-start) mask
// ---------------------------------------------------------------------------

/// Computes the positions of token starts: every opening string quote, plus
/// every ordinary (non-structural, non-whitespace, outside-string) byte that
/// immediately follows a structural or whitespace boundary.
///
/// `previous_was_boundary` carries whether the last byte of the previous chunk
/// was a boundary and is updated to reflect the last byte of this chunk.
///
/// **Carry-out note:** for invalid ZTI inputs whose last chunk byte is inside
/// an open string (no boundary or token event there), the carry may diverge
/// from the prior event-driven implementation's "unchanged" semantics. In valid
/// ZTI a closing quote is always followed by `;` or `]`, so the next ordinary
/// token always sees a fresh boundary in its own chunk and the carry value is
/// irrelevant in that position.
#[inline(always)]
fn compute_pseudo_structural_mask(
    ordinary_token_mask: u64,
    opening_quote_mask: u64,
    boundary_mask: u64,
    len: usize,
    previous_was_boundary: &mut bool,
) -> u64 {
    let follows_boundary = (boundary_mask << 1) | (*previous_was_boundary as u64);
    let pseudo_structural_mask = opening_quote_mask | (ordinary_token_mask & follows_boundary);

    *previous_was_boundary = len > 0 && (boundary_mask >> (len - 1)) & 1 != 0;

    pseudo_structural_mask
}

// ---------------------------------------------------------------------------
// Bit utilities
// ---------------------------------------------------------------------------

#[inline(always)]
fn emit_bits(mut bits: u64, base: usize, out: &mut Vec<usize>) {
    while bits != 0 {
        let bit = bits.trailing_zeros() as usize;
        out.push(base + bit);
        bits &= bits - 1;
    }
}

#[inline(always)]
fn low_bits(len: usize) -> u64 {
    debug_assert!(len <= 64);

    if len == 64 {
        u64::MAX
    } else {
        (1_u64 << len) - 1
    }
}

#[cfg(test)]
fn mask_range(start: usize, end: usize) -> u64 {
    debug_assert!(start <= end);
    debug_assert!(end <= 64);

    low_bits(end) & !low_bits(start)
}

// ---------------------------------------------------------------------------
// Oracle: verbatim copies of the pre-branchless Stage 2 implementations.
// Used only in tests. Will be removed in a follow-up PR once the branchless
// implementations have soaked.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod oracle {
    pub(super) fn unescaped_quote_mask(
        quote_mask: u64,
        backslash_mask: u64,
        len: usize,
        escaped: &mut bool,
    ) -> u64 {
        let mut delimiter_quote_mask = 0_u64;
        let mut events = quote_mask | backslash_mask;
        let mut previous_event_bit = None::<usize>;

        while events != 0 {
            let bit = events.trailing_zeros() as usize;
            let bit_mask = 1_u64 << bit;

            match previous_event_bit {
                None if bit != 0 => *escaped = false,
                Some(previous) if bit != previous + 1 => *escaped = false,
                _ => {}
            }

            if (quote_mask & bit_mask) != 0 {
                if !*escaped {
                    delimiter_quote_mask |= bit_mask;
                }
                *escaped = false;
            } else {
                *escaped = !*escaped;
            }

            previous_event_bit = Some(bit);
            events &= events - 1;
        }

        match previous_event_bit {
            Some(bit) if bit + 1 == len => {}
            _ => *escaped = false,
        }

        delimiter_quote_mask
    }

    pub(super) fn compute_string_region_mask(
        delimiter_quote_mask: u64,
        len: usize,
        in_string: &mut bool,
    ) -> (u64, u64) {
        let mut string_region_mask = 0_u64;
        let mut opening_quote_mask = 0_u64;
        let mut cursor = 0_usize;
        let mut quotes = delimiter_quote_mask;

        while quotes != 0 {
            let bit = quotes.trailing_zeros() as usize;
            let bit_mask = 1_u64 << bit;

            if *in_string {
                string_region_mask |= super::mask_range(cursor, bit + 1);
                *in_string = false;
                cursor = bit + 1;
            } else {
                opening_quote_mask |= bit_mask;
                cursor = bit;
                *in_string = true;
            }

            quotes &= quotes - 1;
        }

        if *in_string {
            string_region_mask |= super::mask_range(cursor, len);
        }

        (string_region_mask, opening_quote_mask)
    }

    pub(super) fn compute_pseudo_structural_mask(
        ordinary_token_mask: u64,
        opening_quote_mask: u64,
        boundary_mask: u64,
        previous_was_boundary: &mut bool,
    ) -> u64 {
        let mut pseudo_structural_mask = 0_u64;
        let mut events = ordinary_token_mask | opening_quote_mask | boundary_mask;

        while events != 0 {
            let bit = events.trailing_zeros() as usize;
            let bit_mask = 1_u64 << bit;

            if (opening_quote_mask & bit_mask) != 0 {
                pseudo_structural_mask |= bit_mask;
                *previous_was_boundary = false;
            } else if (boundary_mask & bit_mask) != 0 {
                *previous_was_boundary = true;
            } else {
                if *previous_was_boundary {
                    pseudo_structural_mask |= bit_mask;
                }
                *previous_was_boundary = false;
            }

            events &= events - 1;
        }

        pseudo_structural_mask
    }
}

// ---------------------------------------------------------------------------
// Bit-math parity tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod bitmath_tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
    }

    // -----------------------------------------------------------------------
    // prefix_xor: scalar ladder vs fast path
    // -----------------------------------------------------------------------

    #[test]
    fn prefix_xor_scalar_vs_fast() {
        let mut lcg = Lcg(0xdead_beef_cafe_1234);
        for _ in 0..10_000 {
            let x = lcg.next();
            assert_eq!(
                prefix_xor_scalar(x),
                prefix_xor(x),
                "prefix_xor mismatch for x={x:#018x}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // unescaped_quote_mask: exhaustive 8-bit parity vs oracle (3^8 = 6561 × 2)
    // -----------------------------------------------------------------------

    #[test]
    fn unescaped_quote_exhaustive_8bit() {
        for carry_in in [0u64, 1u64] {
            // Encode each of the 8 byte positions as 0=neither, 1=quote, 2=backslash.
            for encoded in 0u32..6561 {
                let mut quote = 0u64;
                let mut backslash = 0u64;
                let mut n = encoded;
                for bit in 0..8 {
                    match n % 3 {
                        1 => quote |= 1 << bit,
                        2 => backslash |= 1 << bit,
                        _ => {}
                    }
                    n /= 3;
                }

                let mut carry_new = carry_in;
                let new_r = unescaped_quote_mask(quote, backslash, 8, &mut carry_new);

                let mut escaped_old = carry_in != 0;
                let old_r = oracle::unescaped_quote_mask(quote, backslash, 8, &mut escaped_old);

                assert_eq!(
                    new_r, old_r,
                    "mask mismatch q={quote:#010x} b={backslash:#010x} carry_in={carry_in}"
                );
                assert_eq!(
                    carry_new, escaped_old as u64,
                    "carry mismatch q={quote:#010x} b={backslash:#010x} carry_in={carry_in}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // unescaped_quote_mask: backslash runs straddling chunk boundaries
    // -----------------------------------------------------------------------

    #[test]
    fn unescaped_quote_straddle_runs() {
        // `run_len` backslashes followed by a quote: even run → unescaped, odd → escaped.
        for run_len in 0usize..=130 {
            let mut input = vec![b'\\'; run_len];
            input.push(b'"');

            let mut carry = 0u64;
            let mut found_unescaped = false;
            let mut byte_offset = 0usize;

            for chunk in input.chunks(64) {
                let len = chunk.len();
                let mut q = 0u64;
                let mut bs = 0u64;
                for (i, &b) in chunk.iter().enumerate() {
                    if b == b'"' {
                        q |= 1 << i;
                    }
                    if b == b'\\' {
                        bs |= 1 << i;
                    }
                }

                let uq = unescaped_quote_mask(q, bs, len, &mut carry);

                if run_len >= byte_offset && run_len < byte_offset + len {
                    let bit_in_chunk = run_len - byte_offset;
                    if (uq >> bit_in_chunk) & 1 == 1 {
                        found_unescaped = true;
                    }
                }
                byte_offset += len;
            }

            let expected_unescaped = run_len % 2 == 0;
            assert_eq!(
                found_unescaped, expected_unescaped,
                "run_len={run_len}: expected unescaped={expected_unescaped}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // unescaped_quote_mask: randomized multi-chunk parity
    // -----------------------------------------------------------------------

    #[test]
    fn unescaped_quote_randomized_parity() {
        let mut lcg = Lcg(0x1234_5678_9abc_def0);
        for _ in 0..10_000 {
            let len = (lcg.next() % 64 + 1) as usize;
            let valid = low_bits(len);
            let a = lcg.next() & valid;
            let b = lcg.next() & valid & !a;
            let (quote, backslash) = (a, b);
            let carry_in = lcg.next() & 1;

            let mut carry_new = carry_in;
            let new_r = unescaped_quote_mask(quote, backslash, len, &mut carry_new);

            let mut escaped_old = carry_in != 0;
            let old_r = oracle::unescaped_quote_mask(quote, backslash, len, &mut escaped_old);

            assert_eq!(new_r, old_r, "randomized uq mask mismatch");
            assert_eq!(
                carry_new, escaped_old as u64,
                "randomized uq carry mismatch"
            );
        }
    }

    // -----------------------------------------------------------------------
    // compute_string_region_mask: randomized parity
    // -----------------------------------------------------------------------

    #[test]
    fn string_region_randomized_parity() {
        let mut lcg = Lcg(0xabcd_ef01_2345_6789);
        for _ in 0..10_000 {
            let len = (lcg.next() % 64 + 1) as usize;
            let valid = low_bits(len);
            let uq = lcg.next() & valid;
            let in_str_init = lcg.next() & 1 != 0;

            let mut in_str_new = in_str_init;
            let (srm_new, oq_new) = compute_string_region_mask(uq, len, &mut in_str_new);

            let mut in_str_old = in_str_init;
            let (srm_old, oq_old) = oracle::compute_string_region_mask(uq, len, &mut in_str_old);

            assert_eq!(
                srm_new, srm_old,
                "string_region_mask mismatch uq={uq:#018x} len={len} in_str={in_str_init}"
            );
            assert_eq!(
                oq_new, oq_old,
                "opening_quote_mask mismatch uq={uq:#018x} len={len} in_str={in_str_init}"
            );
            assert_eq!(
                in_str_new, in_str_old,
                "in_string carry mismatch uq={uq:#018x} len={len} in_str={in_str_init}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // compute_pseudo_structural_mask: parity on grammar-valid inputs only.
    // The branchless formula's carry-out may differ from the oracle's on
    // invalid inputs (see the function's doc comment). Grammar-valid inputs
    // have ordinary token bytes only following boundary bytes, so the carry
    // from any chunk whose last event is a boundary always equals
    // boundary_mask[len-1] (valid ZTI never ends a chunk on a mid-string byte
    // without a following structural or whitespace within the same chunk).
    // -----------------------------------------------------------------------

    #[test]
    fn pseudo_structural_randomized_parity_valid() {
        let mut lcg = Lcg(0xfeed_face_dead_beef);
        for _ in 0..10_000 {
            let len = (lcg.next() % 64 + 1) as usize;
            let valid = low_bits(len);

            let boundary_mask = lcg.next() & valid;
            let opening_quote_mask = lcg.next() & valid & !boundary_mask;
            // Only allow ordinary tokens at positions that follow a boundary
            // within this chunk (grammar-valid constraint) to avoid the carry
            // divergence documented on the function.
            let follows = (boundary_mask << 1) | 1u64; // assume prev was boundary
            let ordinary_token_mask =
                lcg.next() & valid & !boundary_mask & !opening_quote_mask & follows;

            // Skip cases where the last byte has no event at all (that's the
            // exact scenario where the two impls diverge in carry-out).
            let all_events = ordinary_token_mask | opening_quote_mask | boundary_mask;
            if all_events != 0 {
                let last_event_bit = 63 - all_events.leading_zeros() as usize;
                if last_event_bit < len - 1 {
                    // Last byte is event-free → potential carry divergence.
                    continue;
                }
            }

            let prev_init = lcg.next() & 1 != 0;
            let mut prev_new = prev_init;
            let mut prev_old = prev_init;

            let new_r = compute_pseudo_structural_mask(
                ordinary_token_mask,
                opening_quote_mask,
                boundary_mask,
                len,
                &mut prev_new,
            );
            let old_r = oracle::compute_pseudo_structural_mask(
                ordinary_token_mask,
                opening_quote_mask,
                boundary_mask,
                &mut prev_old,
            );

            assert_eq!(new_r, old_r, "pseudo_structural mask mismatch");
            assert_eq!(prev_new, prev_old, "pseudo_structural carry mismatch");
        }
    }
}
