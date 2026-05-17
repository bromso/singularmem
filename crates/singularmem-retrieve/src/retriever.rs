//! `Retriever` composes `HybridSearcher` + `Store::get` into prompt-ready
//! memory blocks. The struct borrows references to both so callers retain
//! ownership of the underlying components.

use std::time::Duration;

use jiff::Timestamp;
use serde::Serialize;
use singularmem_core::ItemId;
use singularmem_search::{HybridSearchOptions, ScoreKind};

/// Options controlling a `Retriever::retrieve` call.
#[derive(Debug, Clone)]
pub struct RetrieveOptions {
    /// Maximum number of memory blocks to return. Default: 10.
    pub max_blocks: usize,
    /// Minimum score for a hit to be included. Default: 0.0.
    /// Applied BEFORE `max_blocks` truncation so low-relevance hits
    /// don't crowd out genuinely-relevant matches.
    pub min_score: f32,
    /// Underlying hybrid-search options (passed through to `HybridSearcher`).
    pub search: HybridSearchOptions,
}

impl Default for RetrieveOptions {
    fn default() -> Self {
        Self {
            max_blocks: 10,
            min_score: 0.0,
            search: HybridSearchOptions::default(),
        }
    }
}

/// Results of a retrieval call.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievedContext {
    /// Memory blocks in descending score order, truncated to `max_blocks`.
    pub blocks: Vec<MemoryBlock>,
    /// The query that was retrieved against.
    pub query: String,
    /// Wall-clock duration of the entire `Retriever::retrieve` call
    /// (including the underlying search AND the per-hit `Store::get` reads).
    pub elapsed: Duration,
    /// Number of distinct documents considered for fusion (lexical ∪ semantic),
    /// from `HybridSearchResults::total_fused`. Use as denominator for
    /// "showed N of M considered".
    pub total_considered: usize,
}

/// One memory block in a [`RetrievedContext`]. Carries the full item content
/// (not a snippet) plus enough metadata for adapters to format provenance.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryBlock {
    /// The matched item's ID.
    pub id: ItemId,
    /// FULL content from `Store::get`, not the Tantivy-trimmed snippet.
    pub content: String,
    /// Score whose meaning depends on `score_kind`.
    pub score: f32,
    /// Tells the consumer what `score` represents (RRF / BM25 / Cosine).
    pub score_kind: ScoreKind,
    /// Free-form provenance label from the underlying [`singularmem_core::Item`].
    pub source: Option<String>,
    /// Tags from the underlying [`singularmem_core::Item`].
    pub tags: Vec<String>,
    /// Wall-clock timestamp the store assigned at ingest.
    pub created_at: Timestamp,
}

use singularmem_core::Store;
use singularmem_search::HybridSearcher;

/// Composes a hybrid search + per-hit store reads into prompt-ready
/// `MemoryBlock`s.
///
/// Borrows references to `Store` and `HybridSearcher` — same borrow pattern
/// `HybridSearcher` uses for its underlying indexes. Callers retain
/// ownership of the underlying components.
pub struct Retriever<'a> {
    /// Borrowed reference to the underlying memory store.
    pub store: &'a Store,
    /// Borrowed reference to the hybrid searcher.
    pub searcher: &'a HybridSearcher<'a>,
}

impl<'a> Retriever<'a> {
    /// Construct a `Retriever` borrowing the given store and searcher.
    #[must_use]
    pub const fn new(store: &'a Store, searcher: &'a HybridSearcher<'a>) -> Self {
        Self { store, searcher }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_match_spec() {
        let o = RetrieveOptions::default();
        assert_eq!(o.max_blocks, 10);
        assert!((o.min_score - 0.0).abs() < f32::EPSILON);
        // search field defaults pulled from HybridSearchOptions; we don't
        // re-assert those here because sub-project 2c already tests them.
    }

    use singularmem_core::Store;
    use singularmem_search::testing::MockEmbedder;
    use singularmem_search::{EmbedderIndex, HybridSearcher, Index};
    use tempfile::TempDir;

    #[test]
    fn new_holds_references_to_store_and_searcher() {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();
        let lex = Index::open(dir.path().join("lex")).unwrap();
        let sem =
            EmbedderIndex::open(dir.path().join("sem"), Box::new(MockEmbedder::default()))
                .unwrap();
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        // The struct fields are public; we can observe the borrowed references.
        assert!(std::ptr::eq(retriever.store, &store));
        assert!(std::ptr::eq(retriever.searcher, &searcher));
    }
}
