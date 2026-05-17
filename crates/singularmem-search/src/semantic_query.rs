//! Types for semantic (vector) search queries and results.
//!
//! These are the public-facing result types returned by
//! [`EmbedderIndex::semantic_search`](crate::EmbedderIndex::semantic_search).

use singularmem_core::ItemId;
use std::time::Duration;

/// Options controlling a semantic search query.
#[derive(Debug, Clone)]
pub struct SemanticSearchOptions {
    /// Maximum number of results to return. Default: 20.
    pub limit: usize,
    /// Minimum cosine similarity score `[−1.0, 1.0]` for a hit to be
    /// included. Default: `0.0` (return everything that scores above zero).
    pub min_score: f32,
}

impl Default for SemanticSearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            min_score: 0.0,
        }
    }
}

/// The results of a semantic search query.
#[derive(Debug)]
pub struct SemanticSearchResults {
    /// Hits in descending score order, filtered by
    /// [`SemanticSearchOptions::min_score`].
    pub hits: Vec<SemanticHit>,
    /// Wall-clock time elapsed for embedding the query and running KNN.
    pub elapsed: Duration,
    /// Total number of vectors in the index at query time.
    pub total_indexed: u64,
}

/// A single hit from a semantic search query.
#[derive(Debug)]
pub struct SemanticHit {
    /// The item identifier.
    pub id: ItemId,
    /// Cosine similarity score in `[-1.0, 1.0]`. Higher = more similar.
    pub score: f32,
}
