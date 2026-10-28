#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Default maximum characters per chunk (approx. 200–250 tokens).
pub const DEFAULT_CHUNK_MAX_CHARS: usize = 1000;

/// Default character overlap between consecutive chunks to preserve context across boundaries.
pub const DEFAULT_CHUNK_OVERLAP_CHARS: usize = 100;

/// A bounded, deterministic text chunk with original document offsets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextChunk {
    /// 0-indexed sequential chunk index within the document.
    pub index: usize,
    /// Chunk text content.
    pub text: String,
    /// 0-indexed character start offset in the original document.
    pub char_start: usize,
    /// 0-indexed character end offset (exclusive) in the original document.
    pub char_end: usize,
}

/// Pure, deterministic, bounded text chunker.
///
/// Guarantees:
/// - Never panics on any valid UTF-8 string input.
/// - Every chunk's character count is `<= max_chunk_chars` (and `>= 1` if input is non-empty).
/// - Chunks together cover all character indices of the input (`char_end == total_chars`).
/// - Multibyte UTF-8 characters are never split mid-character.
/// - Empty string returns an empty `Vec<TextChunk>`.
/// - Whitespace-only string returns bounded chunk(s) without infinite loops.
/// - Strictly forward progress guaranteed (`start` advances each step).
pub fn chunk_text(text: &str, max_chunk_chars: usize, overlap_chars: usize) -> Vec<TextChunk> {
    if text.is_empty() {
        return Vec::new();
    }

    let max_chars = max_chunk_chars.max(1);
    let overlap = overlap_chars.min(max_chars.saturating_sub(1));

    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();
    if total_chars == 0 {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    let mut index = 0;

    while start < total_chars {
        let hard_end = (start + max_chars).min(total_chars);
        let mut split_point = hard_end;

        // Attempt a natural boundary break (paragraph break, sentence end, or whitespace)
        // only when we are not already at the end of the text.
        if hard_end < total_chars {
            let min_split = start + (max_chars / 2).max(1);
            if let Some(pos) = (min_split..hard_end)
                .rev()
                .find(|&i| chars[i] == '\n' || chars[i] == '.' || chars[i].is_whitespace())
            {
                split_point = pos + 1;
            }
        }

        let chunk_slice = &chars[start..split_point];
        let chunk_str: String = chunk_slice.iter().collect();

        chunks.push(TextChunk {
            index,
            text: chunk_str,
            char_start: start,
            char_end: split_point,
        });
        index += 1;

        if split_point >= total_chars {
            break;
        }

        let next_start = if overlap > 0 {
            split_point.saturating_sub(overlap)
        } else {
            split_point
        };

        // Strictly guarantee forward progress
        start = next_start.max(start + 1);
    }

    chunks
}
