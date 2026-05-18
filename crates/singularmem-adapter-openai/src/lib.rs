//! Singularmem retrieval adapter for OpenAI/Codex.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! bracketed citation markers (`[N]`) with a leading directive line
//! that primes GPT-family models to cite back by index. Same-line
//! `source:` in the `[N]` header when present; own-line `tags:` below;
//! blank line; full content emitted verbatim (no escaping).
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-openai-3c-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for `OpenAI` / `OpenAI` Codex. Stateless unit struct.
pub struct OpenAiAdapter;

impl Adapter for OpenAiAdapter {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return format!("No memories matched for query: {:?}\n", ctx.query);
        }
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Use the following retrieved memories. Cite by [N] index."
        );
        let _ = writeln!(out);
        for (i, block) in ctx.blocks.iter().enumerate() {
            // Per-block header: bracket marker + optional source on same line.
            if let Some(source) = &block.source {
                let _ = writeln!(out, "[{}] source: {}", i + 1, source);
            } else {
                let _ = writeln!(out, "[{}]", i + 1);
            }
            if !block.tags.is_empty() {
                let _ = writeln!(out, "tags: {}", block.tags.join(", "));
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", block.content);
            // Separator between blocks: blank line. No trailing blank
            // after the last block.
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

    #[test]
    fn name_returns_openai() {
        assert_eq!(OpenAiAdapter.name(), "openai");
    }

    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_retrieve::MemoryBlock;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

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
    fn format_includes_citation_instruction_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.contains("Use the following retrieved memories. Cite by [N] index."),
            "missing citation instruction: {out}"
        );
    }

    #[test]
    fn format_emits_one_indexed_bracket_markers() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.lines().any(|l| l.starts_with("[1]")),
            "missing [1] header: {out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("[2]")),
            "missing [2] header: {out}"
        );
        assert!(
            !out.lines().any(|l| l.starts_with("[0]")),
            "0-indexed slipped in: {out}"
        );
    }

    #[test]
    fn format_includes_source_on_header_line_when_present() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec![],
            )],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.contains("[1] source: claude-conversation:abc-123"),
            "missing source on header: {out}"
        );
    }

    #[test]
    fn format_omits_source_keyword_when_none() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        let header_line = out
            .lines()
            .find(|l| l.starts_with("[1]"))
            .expect("[1] header should exist");
        assert!(
            !header_line.contains("source:"),
            "unexpected source on bare header: {header_line}"
        );
    }

    #[test]
    fn format_includes_tags_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                None,
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.lines().any(|l| l == "tags: fox, animals"),
            "missing tags line: {out}"
        );
    }

    #[test]
    fn format_omits_tags_when_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            !out.lines().any(|l| l.starts_with("tags:")),
            "unexpected tags line: {out}"
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
        let out = OpenAiAdapter.format(&ctx);
        let lines: Vec<&str> = out.lines().collect();
        let idx_2 = lines
            .iter()
            .position(|l| l.starts_with("[2]"))
            .expect("[2] header should exist");
        assert!(idx_2 > 0, "[2] should not be the first line");
        assert!(
            lines[idx_2 - 1].is_empty(),
            "expected blank line immediately before [2] header; got: {:?}",
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
        let out = OpenAiAdapter.format(&ctx);
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
    fn format_empty_context_emits_no_match_line_without_brackets() {
        let ctx = sample_context(vec![], "nothing here");
        let out = OpenAiAdapter.format(&ctx);
        assert_eq!(out, "No memories matched for query: \"nothing here\"\n");
        assert!(
            !out.contains('['),
            "[ leaked into empty-state output: {out}"
        );
        assert!(
            !out.contains(']'),
            "] leaked into empty-state output: {out}"
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
        let out = OpenAiAdapter.format(&ctx);
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
        let out = OpenAiAdapter.format(&ctx);
        assert!(out.contains("line one"), "missing line one: {out}");
        assert!(out.contains("line two"), "missing line two: {out}");
        assert!(out.contains("line three"), "missing line three: {out}");
    }

    #[test]
    fn format_full_block_has_blank_line_between_metadata_and_content() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("src"),
                vec!["t1"],
            )],
            "x",
        );
        let out = OpenAiAdapter.format(&ctx);
        let lines: Vec<&str> = out.lines().collect();
        let content_idx = lines
            .iter()
            .position(|l| l.starts_with("the quick brown fox"))
            .expect("content line should exist");
        assert!(content_idx > 0, "content should not be the first line");
        assert!(
            lines[content_idx - 1].is_empty(),
            "expected blank line before content; got: {:?}",
            lines[content_idx - 1]
        );
    }
}
