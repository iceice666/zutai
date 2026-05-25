use crate::charclass::{is_structural, is_whitespace};
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
    pub structural_mask: u64,
    pub quote_mask: u64,
    pub backslash_mask: u64,
    pub whitespace_mask: u64,
    pub pseudo_structural_mask: u64,
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
        let mut chunks = Vec::new();
        let mut in_string = false;
        let mut escaped = false;
        let mut previous_was_boundary = true;

        for (chunk_index, chunk) in self.bytes.chunks(64).enumerate() {
            let base = chunk_index * 64;
            let mut chunk_scan = ChunkScan {
                base,
                structural_mask: 0,
                quote_mask: 0,
                backslash_mask: 0,
                whitespace_mask: 0,
                pseudo_structural_mask: 0,
            };

            for (bit, byte) in chunk.iter().copied().enumerate() {
                let offset = base + bit;
                let mask = 1_u64 << bit;
                let was_in_string = in_string;

                if is_whitespace(byte) {
                    chunk_scan.whitespace_mask |= mask;
                }

                if byte == b'\\' {
                    chunk_scan.backslash_mask |= mask;
                }

                if byte == b'"' && !escaped {
                    chunk_scan.quote_mask |= mask;
                    if !in_string {
                        chunk_scan.pseudo_structural_mask |= mask;
                        pseudo_structural.push(offset);
                        previous_was_boundary = false;
                    }
                    in_string = !in_string;
                }

                if !was_in_string && !in_string {
                    if is_structural(byte) {
                        chunk_scan.structural_mask |= mask;
                        structural.push(offset);
                        previous_was_boundary = true;
                    } else if is_whitespace(byte) {
                        previous_was_boundary = true;
                    } else if previous_was_boundary {
                        chunk_scan.pseudo_structural_mask |= mask;
                        pseudo_structural.push(offset);
                        previous_was_boundary = false;
                    } else {
                        previous_was_boundary = false;
                    }
                }

                escaped = byte == b'\\' && !escaped;
            }

            chunks.push(chunk_scan);
        }

        if in_string {
            return Err(error(self.input.len(), ParseErrorKind::UnclosedString));
        }

        let mut significant = Vec::with_capacity(structural.len() + pseudo_structural.len());
        significant.extend(structural.iter().copied());
        significant.extend(pseudo_structural.iter().copied());
        significant.sort_unstable();

        Ok(StructuralIndex {
            structural,
            pseudo_structural,
            significant,
            chunks,
        })
    }
}
