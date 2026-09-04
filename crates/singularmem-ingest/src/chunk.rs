//! Paragraph-aware text chunking for embedding-friendly item sizes.

/// Default chunk cap in bytes.
pub const DEFAULT_CHUNK_BYTES: usize = 4096;

/// Split `text` into chunks of at most `max_bytes` bytes.
///
/// Paragraphs (separated by a blank line) are packed greedily; a paragraph
/// larger than `max_bytes` is hard-split at the last char boundary that
/// fits. Chunks are trimmed and never empty. Whitespace-only input yields
/// no chunks.
///
/// `max_bytes` below 4 is raised to 4 so any UTF-8 scalar fits; the
/// returned chunks are ≤ `max(max_bytes, 4)` bytes. Runs of whitespace
/// that fall entirely within a hard-split boundary are dropped, not
/// carried into a neighbouring chunk.
#[must_use]
pub fn chunk_text(text: &str, max_bytes: usize) -> Vec<String> {
    let max_bytes = max_bytes.max(4);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        let t = current.trim();
        if !t.is_empty() {
            chunks.push(t.to_string());
        }
        current.clear();
    };

    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if para.len() > max_bytes {
            flush(&mut current, &mut chunks);
            for piece in hard_split(para, max_bytes) {
                chunks.push(piece.to_string());
            }
            continue;
        }
        let needed = if current.is_empty() {
            para.len()
        } else {
            current.len() + 2 + para.len()
        };
        if needed > max_bytes {
            flush(&mut current, &mut chunks);
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }
    flush(&mut current, &mut chunks);
    chunks
}

/// Split `s` into pieces of at most `max_bytes` bytes on char boundaries.
fn hard_split(s: &str, max_bytes: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + max_bytes).min(s.len());
        while end > start && !s.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            // A single char wider than max_bytes; take it whole.
            end = s[start..]
                .char_indices()
                .nth(1)
                .map_or(s.len(), |(i, _)| start + i);
        }
        let piece = s[start..end].trim();
        if !piece.is_empty() {
            out.push(piece);
        }
        start = end;
    }
    out
}
