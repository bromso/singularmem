//! Hybrid (lexical + semantic) search via Reciprocal Rank Fusion.
//!
//! See `docs/superpowers/specs/2026-05-17-search-v0-hybrid-design.md`
//! for the design rationale.

use serde::Serialize;
use singularmem_core::ItemId;
use std::time::Duration;

/// Options controlling a hybrid search query.
#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    /// Maximum number of fused hits to return. Default: 20.
    pub limit: usize,
    /// Per-ranker overfetch factor. Each underlying ranker fetches
    /// `limit * fetch_multiplier` candidates before fusion. Default: 3.
    pub fetch_multiplier: usize,
    /// RRF damping constant `k`. Larger → flatter weighting between
    /// top-1 and top-N. Default: 60 (Cormack et al. 2009).
    pub rrf_k: usize,
    /// Include lexical snippet highlights (if available). Default: true.
    pub include_snippets: bool,
}

impl Default for HybridSearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            fetch_multiplier: 3,
            rrf_k: 60,
            include_snippets: true,
        }
    }
}

/// Discriminator naming which kind of score `HybridHit::score` carries.
///
/// Lets `--json` consumers interpret the float correctly without inspecting
/// rank fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind {
    /// Fused Reciprocal Rank Fusion score (both rankers ran).
    Rrf,
    /// Tantivy BM25 score (lexical-only mode).
    Bm25,
    /// `USearch` cosine similarity (semantic-only mode).
    Cosine,
}

/// One hit in a `HybridSearchResults`.
#[derive(Debug, Clone, Serialize)]
pub struct HybridHit {
    /// The matched item's ID. Call `Store::get(hit.id)` for the full payload.
    pub id: ItemId,
    /// Score whose meaning depends on `score_kind`.
    pub score: f32,
    /// Tells the consumer what `score` represents.
    pub score_kind: ScoreKind,
    /// 1-based rank in the lexical ranker, or `None` if absent.
    pub lexical_rank: Option<usize>,
    /// 1-based rank in the semantic ranker, or `None` if absent.
    pub semantic_rank: Option<usize>,
    /// Highlighted snippet from the lexical hit, when available.
    /// `None` when `include_snippets` is false OR when the hit did not appear
    /// in the lexical ranker.
    pub snippet: Option<String>,
}

/// Results of a hybrid search query.
#[derive(Debug, Clone, Serialize)]
pub struct HybridSearchResults {
    /// Hits in descending `score` order, with `ItemId` ascending as tie-break.
    pub hits: Vec<HybridHit>,
    /// Wall-clock duration of the entire `HybridSearcher::search` call.
    pub elapsed: Duration,
    /// Number of distinct documents considered for fusion (lexical ∪ semantic).
    pub total_fused: usize,
    /// Number of hits the lexical ranker returned (before fusion), or `None`
    /// if the lexical ranker did not run.
    pub lexical_hits: Option<u64>,
    /// Number of hits the semantic ranker returned (before fusion), or `None`
    /// if the semantic ranker did not run.
    pub semantic_hits: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_match_spec() {
        let o = HybridSearchOptions::default();
        assert_eq!(o.limit, 20);
        assert_eq!(o.fetch_multiplier, 3);
        assert_eq!(o.rrf_k, 60);
        assert!(o.include_snippets);
    }

    #[test]
    fn score_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ScoreKind::Rrf).unwrap(), "\"rrf\"");
        assert_eq!(serde_json::to_string(&ScoreKind::Bm25).unwrap(), "\"bm25\"");
        assert_eq!(
            serde_json::to_string(&ScoreKind::Cosine).unwrap(),
            "\"cosine\""
        );
    }
}
