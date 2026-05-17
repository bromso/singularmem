//! `Embedder` trait — produces fixed-dimension embedding vectors for text.
//!
//! Implementations are synchronous (CPU-bound; no tokio runtime). Output
//! vectors are unit-length (L2-normalized) so cosine similarity reduces to
//! dot product. `FastembedEmbedder` (Task 4) lives below the trait definition;
//! `MockEmbedder` for tests lives in `testing.rs` (Task 3).
//!
//! # Usage
//!
//! ```no_run
//! use singularmem_search::{Embedder, FastembedEmbedder};
//!
//! let e = FastembedEmbedder::new().unwrap();
//! let v = e.embed("hello world").unwrap();
//! assert_eq!(v.len(), e.dim());
//! ```

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

// ── FastembedEmbedder ─────────────────────────────────────────────────────────

use crate::error::Error;
use crate::model::{cache_dir, EmbeddingModel};
use std::path::Path;

/// Concrete `Embedder` backed by fastembed (ONNX runtime + curated catalogue).
///
/// Weights are downloaded on first use and cached in
/// `~/.cache/singularmem/models/` (or the platform equivalent).
pub struct FastembedEmbedder {
    inner: fastembed::TextEmbedding,
    model_id: String,
    dim: usize,
}

impl FastembedEmbedder {
    /// Construct with the default model (`AllMiniLmL6V2`). Downloads weights
    /// on first construction if not cached (~80 MB).
    ///
    /// # Errors
    /// Returns `Error::ModelDownload` on network failure during first-time
    /// fetch; `Error::Embedding` on ONNX init failure.
    pub fn new() -> crate::Result<Self> {
        Self::with_model(EmbeddingModel::AllMiniLmL6V2)
    }

    /// Construct with a non-default model from the curated catalogue.
    ///
    /// # Errors
    /// Returns `Error::ModelDownload` if the model cannot be fetched or
    /// initialized.
    pub fn with_model(model: EmbeddingModel) -> crate::Result<Self> {
        let cache = cache_dir();
        std::fs::create_dir_all(&cache).ok(); // best effort
        let init = fastembed::InitOptions::new(model.fastembed())
            .with_cache_dir(cache)
            .with_show_download_progress(true);
        let inner = fastembed::TextEmbedding::try_new(init).map_err(|e| Error::ModelDownload {
            model: model.model_id().to_string(),
            reason: format!("{e}"),
        })?;
        Ok(Self {
            inner,
            model_id: model.model_id().to_string(),
            dim: model.dim(),
        })
    }

    /// Construct from a directory of ONNX weights + tokenizer files (for
    /// air-gapped use or unsupported models). Caller is responsible for
    /// matching the `model_id` to whatever produced the files.
    ///
    /// Expected directory contents (fastembed convention): `model.onnx` or
    /// `model_quantized.onnx`, `tokenizer.json`, `config.json`, optionally
    /// `tokenizer_config.json` and `special_tokens_map.json`.
    ///
    /// # Errors
    /// Currently always returns `Error::InvalidModelFiles` — real
    /// implementation is deferred to v0.3.1.
    pub fn from_files(model_dir: &Path, _model_id: &str) -> crate::Result<Self> {
        Err(Error::InvalidModelFiles {
            path: model_dir.to_path_buf(),
            reason: "from_files is a planned v0.3.1 feature; not implemented in v0.3.0".to_string(),
        })
    }
}

impl Embedder for FastembedEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn embed(&self, content: &str) -> crate::Result<Vec<f32>> {
        let vectors = self.embed_batch(&[content])?;
        vectors.into_iter().next().ok_or_else(|| Error::Embedding {
            context: "embedding single item (empty result)",
            reason: "fastembed returned zero vectors for one input".to_string(),
        })
    }

    fn embed_batch(&self, items: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
        // Truncation warning — fastembed silently truncates beyond max_tokens.
        // We approximate token count with content length / 4 (avg English).
        for s in items {
            let approx_tokens = s.len() / 4;
            if approx_tokens > 256 {
                // Conservative — uses MiniLM's limit as the warning threshold.
                tracing::warn!(
                    approx_tokens,
                    threshold = 256,
                    "item exceeds approximate token limit; fastembed will truncate"
                );
            }
        }
        let owned: Vec<String> = items.iter().map(|s| (*s).to_string()).collect();
        self.inner.embed(owned, None).map_err(|e| Error::Embedding {
            context: "fastembed inference",
            reason: format!("{e}"),
        })
    }
}
