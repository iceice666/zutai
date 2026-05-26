use crate::charclass::{is_structural, is_whitespace};

use super::RawMasks;

#[inline(always)]
pub(super) fn classify_chunk_scalar(chunk: &[u8]) -> RawMasks {
    let mut masks = RawMasks::default();

    for (bit, byte) in chunk.iter().copied().enumerate() {
        masks.quote |= ((byte == b'"') as u64) << bit;
        masks.backslash |= ((byte == b'\\') as u64) << bit;
        masks.whitespace |= (is_whitespace(byte) as u64) << bit;
        masks.structural |= (is_structural(byte) as u64) << bit;
    }

    masks
}
