//! `Adapter` trait + the default `PlainAdapter` formatter.

use crate::retriever::RetrievedContext;

/// Provider adapter contract per Constitution Principle II.
///
/// An adapter is a pure formatting strategy: it takes a [`RetrievedContext`]
/// and renders it as a single prompt-ready string in whatever format the
/// underlying LLM provider prefers (XML for Claude, Markdown for `OpenAI`,
/// plain text for local models).
///
/// **Contract:** implementations MUST be pure functions — no I/O, no
/// network calls, deterministic for identical input. This is enforced
/// by convention (not the type system) and is what makes the trait
/// trivially testable and composable.
pub trait Adapter: Send + Sync {
    /// Stable identifier used in CLI flags and logs.
    /// Lowercase, hyphen-separated. Examples: `"plain"`, `"claude"`, `"openai"`.
    fn name(&self) -> &str;

    /// Render a [`RetrievedContext`] as a single prompt-ready string.
    ///
    /// MUST be a pure function: no I/O, deterministic given the same input.
    fn format(&self, ctx: &RetrievedContext) -> String;
}

/// Default [`Adapter`] implementation. Emits Markdown-shaped output suitable
/// for local LLMs (`Ollama`, `llama.cpp`) and as a baseline for any provider.
///
/// Output shape, per block:
///
/// ```text
/// ## memory N (score=0.XXXX)
/// id: <ULID>
/// created: <RFC3339>
/// source: <provenance-label>      # omitted if None
/// tags: tag1, tag2, tag3          # omitted if empty
///
/// <full content>
/// ---
/// ```
///
/// When there are zero matched blocks, emits a single `[no memories matched
/// query: "..."]` line.
pub struct PlainAdapter;

impl Adapter for PlainAdapter {
    fn name(&self) -> &'static str {
        "plain"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return format!("[no memories matched query: {:?}]\n", ctx.query);
        }
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# {} memor{} for query: {:?}",
            ctx.blocks.len(),
            if ctx.blocks.len() == 1 { "y" } else { "ies" },
            ctx.query
        );
        for (i, block) in ctx.blocks.iter().enumerate() {
            let _ = writeln!(out);
            let _ = writeln!(out, "## memory {} (score={:.4})", i + 1, block.score);
            let _ = writeln!(out, "id: {}", block.id);
            let _ = writeln!(out, "created: {}", block.created_at);
            if let Some(s) = &block.source {
                let _ = writeln!(out, "source: {s}");
            }
            if !block.tags.is_empty() {
                let _ = writeln!(out, "tags: {}", block.tags.join(", "));
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", block.content);
            let _ = writeln!(out, "---");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-line concrete adapter used purely to verify the trait compiles
    /// and is object-safe (`Box<dyn Adapter>` works).
    struct NoopAdapter;
    impl Adapter for NoopAdapter {
        fn name(&self) -> &'static str {
            "noop"
        }
        fn format(&self, _ctx: &RetrievedContext) -> String {
            String::new()
        }
    }

    #[test]
    fn adapter_trait_is_object_safe() {
        let a: Box<dyn Adapter> = Box::new(NoopAdapter);
        assert_eq!(a.name(), "noop");
    }

    use crate::retriever::MemoryBlock;
    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    fn sample_block(id_str: &str, score: f32) -> MemoryBlock {
        MemoryBlock {
            id: ItemId::from_str(id_str).unwrap(),
            content: "the quick brown fox jumps over the lazy dog".to_string(),
            score,
            score_kind: ScoreKind::Rrf,
            source: Some("claude-conversation:abc-123".to_string()),
            tags: vec!["fox".to_string(), "animals".to_string()],
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        }
    }

    fn sample_context(blocks: Vec<MemoryBlock>, query: &str) -> RetrievedContext {
        RetrievedContext {
            blocks,
            query: query.to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 5,
        }
    }

    #[test]
    fn plain_adapter_includes_id_score_content() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0.0328),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", 0.0312),
            ],
            "fox",
        );
        let out = PlainAdapter.format(&ctx);
        // Heading
        assert!(out.contains("## memory 1"), "missing heading: {out}");
        assert!(out.contains("## memory 2"), "missing heading: {out}");
        // Score (formatted to 4 decimals)
        assert!(out.contains("score=0.0328"), "missing score: {out}");
        assert!(out.contains("score=0.0312"), "missing score: {out}");
        // ID
        assert!(
            out.contains("id: 01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "missing id: {out}"
        );
        // Full content
        assert!(
            out.contains("the quick brown fox jumps"),
            "missing content: {out}"
        );
        // Separator
        assert!(out.contains("---"), "missing separator: {out}");
    }

    #[test]
    fn plain_adapter_handles_zero_blocks() {
        let ctx = sample_context(vec![], "nothing here");
        let out = PlainAdapter.format(&ctx);
        assert!(
            out.contains("no memories matched"),
            "missing empty msg: {out}"
        );
        assert!(out.contains("nothing here"), "missing query echo: {out}");
        // No memory headings.
        assert!(
            !out.contains("## memory"),
            "should not have memory headings: {out}"
        );
    }

    #[test]
    fn plain_adapter_omits_optional_fields_when_absent() {
        let mut block = sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0.5);
        block.source = None;
        block.tags = vec![];
        let ctx = sample_context(vec![block], "test");
        let out = PlainAdapter.format(&ctx);
        // No empty source/tags lines.
        assert!(
            !out.contains("source:\n"),
            "empty source line emitted: {out}"
        );
        assert!(!out.contains("tags:\n"), "empty tags line emitted: {out}");
        assert!(
            !out.contains("source: \n"),
            "empty source line emitted: {out}"
        );
        assert!(!out.contains("tags: \n"), "empty tags line emitted: {out}");
    }

    #[test]
    fn plain_adapter_is_deterministic() {
        let ctx = sample_context(vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0.5)], "fox");
        let a = PlainAdapter.format(&ctx);
        let b = PlainAdapter.format(&ctx);
        assert_eq!(a, b, "format must be deterministic");
    }
}
