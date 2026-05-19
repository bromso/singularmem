//! Napi-exposed wrappers for the four constitutional Principle II provider
//! adapters: plain, claude, openai, gemini. Each exposes a synchronous
//! `format(ctx)` method. The `adapters` namespace is constructed on the JS
//! side from these four classes (the post-build patch script wires it up).

use std::str::FromStr as _;

use singularmem_retrieve::Adapter as AdapterTrait;

/// Plain Markdown adapter.
#[napi]
pub struct PlainAdapter;

impl Default for PlainAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl PlainAdapter {
    #[napi(constructor)]
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The adapter's stable name; matches `adapter.name` on the JS side.
    #[napi(getter)]
    #[must_use]
    pub fn name(&self) -> String {
        singularmem_retrieve::PlainAdapter.name().to_string()
    }

    /// Format the given context as Markdown blocks.
    #[napi]
    #[must_use]
    pub fn format(&self, ctx: crate::types::RetrievedContext) -> String {
        let core_ctx = napi_ctx_to_core(ctx);
        singularmem_retrieve::PlainAdapter.format(&core_ctx)
    }
}

/// Anthropic Claude `<documents>` XML adapter.
#[napi]
pub struct ClaudeAdapter;

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl ClaudeAdapter {
    #[napi(constructor)]
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[napi(getter)]
    #[must_use]
    pub fn name(&self) -> String {
        singularmem_adapter_claude::ClaudeAdapter.name().to_string()
    }

    #[napi]
    #[must_use]
    pub fn format(&self, ctx: crate::types::RetrievedContext) -> String {
        let core_ctx = napi_ctx_to_core(ctx);
        singularmem_adapter_claude::ClaudeAdapter.format(&core_ctx)
    }
}

/// `OpenAI` bracketed-citation adapter.
#[napi]
pub struct OpenAiAdapter;

impl Default for OpenAiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl OpenAiAdapter {
    #[napi(constructor)]
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[napi(getter)]
    #[must_use]
    pub fn name(&self) -> String {
        singularmem_adapter_openai::OpenAiAdapter.name().to_string()
    }

    #[napi]
    #[must_use]
    pub fn format(&self, ctx: crate::types::RetrievedContext) -> String {
        let core_ctx = napi_ctx_to_core(ctx);
        singularmem_adapter_openai::OpenAiAdapter.format(&core_ctx)
    }
}

/// Google Gemini em-dash "Source N" adapter.
#[napi]
pub struct GeminiAdapter;

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl GeminiAdapter {
    #[napi(constructor)]
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[napi(getter)]
    #[must_use]
    pub fn name(&self) -> String {
        singularmem_adapter_gemini::GeminiAdapter.name().to_string()
    }

    #[napi]
    #[must_use]
    pub fn format(&self, ctx: crate::types::RetrievedContext) -> String {
        let core_ctx = napi_ctx_to_core(ctx);
        singularmem_adapter_gemini::GeminiAdapter.format(&core_ctx)
    }
}

// ── napi-ctx → core-ctx conversion ────────────────────────────────────────────

/// Convert a JS-sent `RetrievedContext` back into the core
/// `singularmem_retrieve::RetrievedContext` so the underlying `Adapter::format`
/// can consume it. Because the napi `MemoryBlock` is FLAT (same shape as the
/// core type), this is a near-1:1 copy plus a few type conversions
/// (ULID string → `ItemId`, ms-since-epoch f64 → `jiff::Timestamp`, score kind
/// string → `ScoreKind` enum).
fn napi_ctx_to_core(
    ctx: crate::types::RetrievedContext,
) -> singularmem_retrieve::RetrievedContext {
    let blocks = ctx
        .blocks
        .into_iter()
        .map(|b| singularmem_retrieve::MemoryBlock {
            id: singularmem_core::item::ItemId::from_str(&b.id)
                .unwrap_or_else(|_| {
                    singularmem_core::item::ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0")
                        .expect("fallback ULID is valid")
                }),
            content: b.content,
            #[allow(clippy::cast_possible_truncation)]
            score: b.score as f32,
            score_kind: match b.kind.as_str() {
                "bm25" => singularmem_search::ScoreKind::Bm25,
                "cosine" => singularmem_search::ScoreKind::Cosine,
                // "rrf" and any unknown kind fall back to Rrf
                _ => singularmem_search::ScoreKind::Rrf,
            },
            source: b.source,
            tags: b.tags,
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            created_at: jiff::Timestamp::from_millisecond(b.created_at as i64)
                .unwrap_or_else(|_| jiff::Timestamp::now()),
        })
        .collect();
    singularmem_retrieve::RetrievedContext {
        query: ctx.query,
        blocks,
        // The Rust type has `elapsed: Duration` and `total_considered: usize`.
        // Set zero-valued defaults — adapters don't read these fields.
        elapsed: std::time::Duration::ZERO,
        total_considered: 0,
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryBlock, RetrievedContext};

    fn sample_ctx() -> RetrievedContext {
        RetrievedContext {
            query: "test query".to_string(),
            blocks: vec![MemoryBlock {
                id: "01HXAAAAAAAAAAAAAAAAAAAAA0".to_string(),
                content: "memory content".to_string(),
                score: 0.5,
                kind: "rrf".to_string(),
                source: Some("test".to_string()),
                tags: vec!["t1".to_string()],
                created_at: 1_700_000_000_000.0,
            }],
        }
    }

    #[test]
    fn plain_adapter_format_populated() {
        let out = PlainAdapter::new().format(sample_ctx());
        assert!(out.contains("memory content"), "plain adapter should include content");
    }

    #[test]
    fn plain_adapter_format_empty() {
        let empty = RetrievedContext { query: "q".to_string(), blocks: vec![] };
        let out = PlainAdapter::new().format(empty);
        assert!(!out.is_empty(), "empty case should still produce some output");
    }

    #[test]
    fn claude_adapter_emits_documents_wrapper() {
        let out = ClaudeAdapter::new().format(sample_ctx());
        assert!(out.contains("<documents>"));
        assert!(out.contains("<document index="));
    }

    #[test]
    fn openai_adapter_emits_bracket_citations() {
        let out = OpenAiAdapter::new().format(sample_ctx());
        assert!(out.contains("[1]"));
    }

    #[test]
    fn gemini_adapter_emits_source_headers() {
        let out = GeminiAdapter::new().format(sample_ctx());
        assert!(out.contains("Source 1"));
    }

    #[test]
    fn adapter_names_match_rust_crates() {
        assert_eq!(PlainAdapter::new().name(), "plain");
        assert_eq!(ClaudeAdapter::new().name(), "claude");
        assert_eq!(OpenAiAdapter::new().name(), "openai");
        assert_eq!(GeminiAdapter::new().name(), "gemini");
    }
}
