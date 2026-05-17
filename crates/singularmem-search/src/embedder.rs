//! `Embedder` trait — produces fixed-dimension embedding vectors for text.
//!
//! Implementations are synchronous (CPU-bound; no tokio runtime). Output
//! vectors are unit-length (L2-normalized) so cosine similarity reduces to
//! dot product. `FastembedEmbedder` (Task 4) lives below the trait definition;
//! `MockEmbedder` for tests lives in `testing.rs` (Task 3).

use crate::error::Result;

/// Produces fixed-dimension embedding vectors for a text item.
pub trait Embedder: Send + Sync {
    /// The fixed embedding dimension (e.g. 384 for all-MiniLM-L6-v2).
    /// MUST be constant across all calls for a given implementation.
    fn dim(&self) -> usize;

    /// Stable identifier for the underlying model (e.g.
    /// `"sentence-transformers/all-MiniLM-L6-v2@v1"`). Stored in the vector
    /// index metadata; mismatch at open time triggers a reindex prompt.
    fn model_id(&self) -> &str;

    /// Embed one item. Returns a `dim()`-length unit-length f32 vector.
    ///
    /// # Errors
    /// Returns `Error::Embedding` on inference failure.
    fn embed(&self, content: &str) -> Result<Vec<f32>>;

    /// Embed a batch of items. Default impl loops over `embed`; concrete
    /// impls (like `FastembedEmbedder`) override with batched inference.
    ///
    /// # Errors
    /// Returns `Error::Embedding` if any individual `embed` call fails.
    fn embed_batch(&self, items: &[&str]) -> Result<Vec<Vec<f32>>> {
        items.iter().map(|s| self.embed(s)).collect()
    }
}
