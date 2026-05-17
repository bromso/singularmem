//! Curated embedding-model registry. The three named models below ship
//! with v0.3.0; users wanting other models use `FastembedEmbedder::from_files`
//! with their own ONNX weights.

/// Curated catalogue of embedding models supported by `FastembedEmbedder::with_model`.
///
/// Any model not in this enum can still be used via `FastembedEmbedder::from_files`,
/// which takes a directory of ONNX weights + tokenizer files.
#[derive(Debug, Clone, Copy)]
pub enum EmbeddingModel {
    /// 384-dim, ~80 MB, English-focused, fast. The default.
    AllMiniLmL6V2,
    /// 384-dim, ~130 MB, English, slightly higher quality than `MiniLM`.
    BgeSmallEnV15,
    /// 768-dim, ~250 MB, English, larger context (8192 tokens via Matryoshka).
    NomicEmbedTextV15,
}

impl EmbeddingModel {
    /// fastembed's enum value for this model.
    pub(crate) const fn fastembed(self) -> fastembed::EmbeddingModel {
        match self {
            Self::AllMiniLmL6V2 => fastembed::EmbeddingModel::AllMiniLML6V2,
            Self::BgeSmallEnV15 => fastembed::EmbeddingModel::BGESmallENV15,
            Self::NomicEmbedTextV15 => fastembed::EmbeddingModel::NomicEmbedTextV15,
        }
    }

    /// Stable `model_id` string written into `VectorIndexMeta`. The `@v1` suffix
    /// is a version anchor so future weight updates trigger a reindex prompt.
    #[must_use]
    pub const fn model_id(self) -> &'static str {
        match self {
            Self::AllMiniLmL6V2 => "sentence-transformers/all-MiniLM-L6-v2@v1",
            Self::BgeSmallEnV15 => "BAAI/bge-small-en-v1.5@v1",
            Self::NomicEmbedTextV15 => "nomic-ai/nomic-embed-text-v1.5@v1",
        }
    }

    /// Embedding dimension for this model.
    #[must_use]
    pub const fn dim(self) -> usize {
        match self {
            Self::AllMiniLmL6V2 | Self::BgeSmallEnV15 => 384,
            Self::NomicEmbedTextV15 => 768,
        }
    }

    /// Soft truncation point in tokens. fastembed's tokenizer enforces this
    /// limit; we emit `tracing::warn!` when an input exceeds it.
    #[must_use]
    pub const fn max_tokens(self) -> usize {
        match self {
            Self::AllMiniLmL6V2 => 256,
            Self::BgeSmallEnV15 => 512,
            Self::NomicEmbedTextV15 => 8192,
        }
    }
}

/// fastembed's model cache directory. Distinct from `dirs::data_dir()` so
/// deleting all stores doesn't waste the download.
pub(crate) fn cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("singularmem")
        .join("models")
}
