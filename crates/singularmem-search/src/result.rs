//! Result types returned from `Index::search`.

use singularmem_core::ItemId;
use std::time::Duration;

/// One ranked search hit. Carries only what the caller needs to look up the
/// full Item via `Store::get` and to display a snippet.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The matched item's ID. The caller can call `Store::get(hit.id)` for the
    /// full payload.
    pub id: ItemId,

    /// BM25 relevance score. Higher is better. Not directly comparable across
    /// queries; use within a single `SearchResults` to rank.
    pub score: f32,

    /// Highlighted snippet from `content` (only if `SearchOptions::include_snippets`).
    /// Approximately 160 characters centered on the highest-scoring term match.
    /// Matched terms are wrapped in `<mark>...</mark>`.
    pub snippet: Option<String>,
}

/// Bundle of hits + query metadata returned from `Index::search`.
#[derive(Debug)]
pub struct SearchResults {
    /// Ranked hits, best first.
    pub hits: Vec<Hit>,
    /// Total number of documents matching the query (may exceed `hits.len()`).
    pub total_matched: u64,
    /// Wall-clock duration of the search call.
    pub elapsed: Duration,
}

/// Options controlling search behaviour.
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    /// Max number of hits to return. Default 20.
    pub limit: usize,
    /// Hits to skip (for pagination). Default 0.
    pub offset: usize,
    /// Include snippet highlights. Default true.
    pub include_snippets: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            offset: 0,
            include_snippets: true,
        }
    }
}
