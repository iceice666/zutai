mod scalar;

#[cfg(not(target_arch = "aarch64"))]
use scalar::classify_chunk_scalar as classify_chunk;

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(target_arch = "aarch64")]
use neon::classify_chunk_neon as classify_chunk;

use crate::error::{ParseError, ParseErrorKind, error};

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
    escaped: bool,
    previous_was_boundary: bool,
}

impl Default for ScanState {
    fn default() -> Self {
        Self {
            in_string: false,
            escaped: false,
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

            // Stage 1: raw per-byte classification. Architecture-specific
            // SIMD can accelerate this without changing later scanner stages.
            let raw = classify_chunk(chunk);

            // Stage 2: quotes that actually toggle string state.
            let quote_mask =
                unescaped_quote_mask(raw.quote, raw.backslash, chunk.len(), &mut state.escaped);

            // Stage 3: bytes occupied by string regions, including both
            // opening and closing delimiter quotes.
            let (string_region_mask, opening_quote_mask) =
                compute_string_region_mask(quote_mask, chunk.len(), &mut state.in_string);

            let valid_mask = low_bits(chunk.len());
            let outside_plain_mask = valid_mask & !string_region_mask;

            // Stage 4: structurals only count outside strings.
            let structural_mask = raw.structural & outside_plain_mask;

            // Whitespace is recorded raw in ChunkScan, but only outside-string
            // whitespace acts as a token boundary.
            let outside_whitespace_mask = raw.whitespace & outside_plain_mask;

            // Ordinary token bytes are non-structural, non-whitespace bytes
            // outside string regions. Opening quotes are handled separately,
            // because your original code always treated opening quotes as
            // pseudo-structural, regardless of previous_was_boundary.
            let ordinary_token_mask = outside_plain_mask & !(raw.structural | raw.whitespace);

            let boundary_mask = structural_mask | outside_whitespace_mask;

            let pseudo_structural_mask = compute_pseudo_structural_mask(
                ordinary_token_mask,
                opening_quote_mask,
                boundary_mask,
                &mut state.previous_was_boundary,
            );

            // Stage 5: sparse scalar emission from masks.
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

/// Returns quote bytes that are not escaped by an odd-length immediately
/// preceding backslash run.
///
/// This intentionally mirrors the original scalar state:
///
/// escaped = byte == b'\\' && !escaped;
///
/// but does it over only quote/backslash event bits.
fn unescaped_quote_mask(
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

        // Any skipped byte between quote/backslash events is a non-backslash
        // byte, so it clears the escaped state.
        match previous_event_bit {
            None if bit != 0 => *escaped = false,
            Some(previous) if bit != previous + 1 => *escaped = false,
            _ => {}
        }

        if (quote_mask & bit_mask) != 0 {
            if !*escaped {
                delimiter_quote_mask |= bit_mask;
            }

            // A quote byte itself is not a backslash, escaped or not.
            *escaped = false;
        } else {
            // Consecutive backslashes toggle escape parity.
            *escaped = !*escaped;
        }

        previous_event_bit = Some(bit);
        events &= events - 1;
    }

    // If the last quote/backslash event was not the final byte of the chunk,
    // then some non-backslash byte followed it and cleared escape state.
    match previous_event_bit {
        Some(bit) if bit + 1 == len => {}
        _ => *escaped = false,
    }

    delimiter_quote_mask
}

/// Computes the region currently occupied by strings, including both delimiter
/// quotes. Also returns opening quote positions, because those are
/// pseudo-structural positions in this scanner.
fn compute_string_region_mask(
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
            // Closing quote: include bytes from the active string start
            // through this quote.
            string_region_mask |= mask_range(cursor, bit + 1);
            *in_string = false;
            cursor = bit + 1;
        } else {
            // Opening quote: remember it as pseudo-structural and start
            // string coverage here.
            opening_quote_mask |= bit_mask;
            cursor = bit;
            *in_string = true;
        }

        quotes &= quotes - 1;
    }

    if *in_string {
        string_region_mask |= mask_range(cursor, len);
    }

    (string_region_mask, opening_quote_mask)
}

/// Computes pseudo-structural token starts.
///
/// Opening string quotes are always pseudo-structural, matching the behavior
/// of the original scanner. Ordinary token bytes only become pseudo-structural
/// when they follow a boundary.
fn compute_pseudo_structural_mask(
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

#[inline(always)]
fn mask_range(start: usize, end: usize) -> u64 {
    debug_assert!(start <= end);
    debug_assert!(end <= 64);

    low_bits(end) & !low_bits(start)
}
