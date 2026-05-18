//! Singularmem retrieval adapter for Google Gemini.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! em-dash-separated `Source N` headers with a leading directive line
//! that primes Gemini to ground its answer in the listed sources.
//! Per-block header is `Source N:` when both `source` and `tags` are
//! absent, otherwise `Source N — source: ..., tags: ...:` with the
//! metadata comma-joined. Content immediately follows on the next line.
//!
//! The em-dash separator is U+2014 (UTF-8); Rust source files are UTF-8
//! so the literal compiles fine. Project precedent: sub-project 2c's
//! `--show-ranks` output uses the same character.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-gemini-3d-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for Google Gemini. Stateless unit struct.
pub struct GeminiAdapter;

impl Adapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return format!("No grounding sources matched for query: {:?}\n", ctx.query);
        }
        let mut out = String::new();
        let _ = writeln!(out, "Use the following sources to ground your answer.");
        let _ = writeln!(out);
        for (i, block) in ctx.blocks.iter().enumerate() {
            // Build the per-block header. Four cases:
            //   "Source N:"                                   (no source, no tags)
            //   "Source N — source: X:"                       (source only)
            //   "Source N — tags: a, b:"                      (tags only)
            //   "Source N — source: X, tags: a, b:"           (both)
            let mut parts: Vec<String> = Vec::new();
            if let Some(s) = &block.source {
                parts.push(format!("source: {s}"));
            }
            if !block.tags.is_empty() {
                parts.push(format!("tags: {}", block.tags.join(", ")));
            }
            if parts.is_empty() {
                let _ = writeln!(out, "Source {}:", i + 1);
            } else {
                let _ = writeln!(out, "Source {} \u{2014} {}:", i + 1, parts.join(", "));
            }
            // Content immediately follows the header on the next line.
            let _ = writeln!(out, "{}", block.content);
            // Blank line between blocks; no trailing blank after the last.
            if i + 1 < ctx.blocks.len() {
                let _ = writeln!(out);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_retrieve::MemoryBlock;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    #[test]
    fn name_returns_gemini() {
        assert_eq!(GeminiAdapter.name(), "gemini");
    }

    fn sample_block(id_str: &str, source: Option<&str>, tags: Vec<&str>) -> MemoryBlock {
        MemoryBlock {
            id: ItemId::from_str(id_str).unwrap(),
            content: "the quick brown fox jumps over the lazy dog".to_string(),
            score: 0.5,
            score_kind: ScoreKind::Rrf,
            source: source.map(String::from),
            tags: tags.into_iter().map(String::from).collect(),
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        }
    }

    fn sample_context(blocks: Vec<MemoryBlock>, query: &str) -> RetrievedContext {
        RetrievedContext {
            blocks,
            query: query.to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 0,
        }
    }

    #[test]
    fn format_includes_grounding_instruction_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(
            out.contains("Use the following sources to ground your answer."),
            "missing grounding instruction: {out}"
        );
    }

    #[test]
    fn format_emits_one_indexed_source_headers() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(
            out.lines().any(|l| l.starts_with("Source 1")),
            "missing Source 1 header: {out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("Source 2")),
            "missing Source 2 header: {out}"
        );
        assert!(
            !out.lines().any(|l| l.starts_with("Source 0")),
            "0-indexed slipped in: {out}"
        );
    }

    #[test]
    fn format_header_with_source_only() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec![],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        let header_line = out
            .lines()
            .find(|l| l.starts_with("Source 1"))
            .expect("Source 1 header should exist");
        assert!(
            header_line.contains("\u{2014} source: claude-conversation:abc-123"),
            "missing source segment: {header_line}"
        );
        assert!(
            !header_line.contains("tags:"),
            "unexpected tags segment in source-only header: {header_line}"
        );
        assert!(
            header_line.ends_with(':'),
            "header should end with colon: {header_line}"
        );
    }

    #[test]
    fn format_header_with_tags_only() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                None,
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        let header_line = out
            .lines()
            .find(|l| l.starts_with("Source 1"))
            .expect("Source 1 header should exist");
        assert!(
            header_line.contains("\u{2014} tags: fox, animals"),
            "missing tags segment: {header_line}"
        );
        assert!(
            !header_line.contains("source:"),
            "unexpected source segment in tags-only header: {header_line}"
        );
        assert!(
            header_line.ends_with(':'),
            "header should end with colon: {header_line}"
        );
    }

    #[test]
    fn format_header_with_both_source_and_tags() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(
            out.contains(
                "Source 1 \u{2014} source: claude-conversation:abc-123, tags: fox, animals:"
            ),
            "missing combined source+tags header: {out}"
        );
    }

    #[test]
    fn format_header_bare_when_no_metadata() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        let header_line = out
            .lines()
            .find(|l| l.starts_with("Source 1"))
            .expect("Source 1 header should exist");
        assert_eq!(
            header_line, "Source 1:",
            "bare header should be 'Source 1:': got {header_line:?}"
        );
    }

    #[test]
    fn format_separates_blocks_with_blank_line() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        let lines: Vec<&str> = out.lines().collect();
        let idx_2 = lines
            .iter()
            .position(|l| l.starts_with("Source 2"))
            .expect("Source 2 header should exist");
        assert!(idx_2 > 0, "Source 2 should not be the first line");
        assert!(
            lines[idx_2 - 1].is_empty(),
            "expected blank line immediately before Source 2 header; got: {:?}",
            lines[idx_2 - 1]
        );
    }

    #[test]
    fn format_does_not_emit_trailing_blank_line_after_last_block() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(
            out.ends_with("lazy dog\n"),
            "expected trailing 'lazy dog\\n' but got: {out:?}"
        );
        assert!(
            !out.ends_with("\n\n"),
            "trailing blank line after last block: {out:?}"
        );
    }

    #[test]
    fn format_empty_context_emits_no_match_line_with_grounding_phrasing() {
        let ctx = sample_context(vec![], "nothing here");
        let out = GeminiAdapter.format(&ctx);
        assert_eq!(
            out,
            "No grounding sources matched for query: \"nothing here\"\n"
        );
        assert!(
            out.contains("grounding"),
            "empty state should use grounding vocabulary: {out}"
        );
    }

    #[test]
    fn format_does_not_include_score_or_id_or_created_at() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("some-source"),
                vec!["t1"],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(!out.contains("0.5000"), "score appeared in output: {out}");
        assert!(
            !out.contains("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "ULID appeared in output: {out}"
        );
        assert!(
            !out.contains("2026-05-12"),
            "created_at appeared in output: {out}"
        );
    }

    #[test]
    fn format_preserves_multiline_content_verbatim() {
        let mut block = sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]);
        block.content = "line one\nline two\nline three".to_string();
        let ctx = sample_context(vec![block], "x");
        let out = GeminiAdapter.format(&ctx);
        assert!(out.contains("line one"), "missing line one: {out}");
        assert!(out.contains("line two"), "missing line two: {out}");
        assert!(out.contains("line three"), "missing line three: {out}");
    }
}
