//! Error type for the search crate. Each variant carries the three pieces
//! Principle VII requires: what failed, what was attempted, what state was
//! preserved.

use std::path::PathBuf;

/// Alias for `std::result::Result<T, Error>` used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by `singularmem-search` operations.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Tantivy library error during a named operation.
    #[error("Tantivy error during {context}: {source}")]
    Tantivy {
        /// Short tag naming what the library was doing when Tantivy errored.
        context: &'static str,
        /// The underlying Tantivy error.
        #[source]
        source: tantivy::TantivyError,
    },

    /// User-supplied query string could not be parsed.
    #[error("could not parse search query: {0}")]
    QueryParse(String),

    /// The Tantivy index directory does not exist or is empty.
    #[error(
        "Tantivy index at {path} is missing or unreadable; run `singularmem reindex` to rebuild"
    )]
    IndexMissing {
        /// Filesystem path that was attempted.
        path: PathBuf,
    },

    /// The Tantivy index directory exists but the contents are corrupted or
    /// incompatible.
    #[error("Tantivy index at {path} appears corrupted: {reason}; run `singularmem reindex`")]
    IndexCorrupted {
        /// Filesystem path that was attempted.
        path: PathBuf,
        /// Human-readable explanation.
        reason: String,
    },

    /// Filesystem or I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Embedding inference failure.
    #[error("embedding inference failed during {context}: {reason}")]
    Embedding {
        /// Short tag naming what the library was doing when inference failed.
        context: &'static str,
        /// Human-readable explanation from the underlying inference engine.
        reason: String,
    },

    /// Could not download embedding model weights.
    #[error("could not download embedding model {model}: {reason}")]
    ModelDownload {
        /// The `model_id` the library attempted to download.
        model: String,
        /// Human-readable explanation from the downloader.
        reason: String,
    },

    /// Model files on disk are missing or invalid.
    #[error("invalid model files at {path}: {reason}; expected ONNX weights + tokenizer")]
    InvalidModelFiles {
        /// Filesystem path that was attempted.
        path: std::path::PathBuf,
        /// Human-readable explanation of what's missing or wrong.
        reason: String,
    },

    /// Vector dimension mismatch between the index metadata and the embedder.
    #[error("vector dimension mismatch: expected {expected}, got {got}")]
    DimMismatch {
        /// Dimensionality expected by the index (from `VectorIndexMeta.dim`).
        expected: usize,
        /// Dimensionality supplied by the caller (from `Embedder::dim()` or the input vector).
        got: usize,
    },

    /// The vector index was built with a different model than the current one.
    #[error(
        "vector index at {path} was built with model {found_model}; \
         current Embedder uses {expected_model}; \
         run `singularmem reindex --with-embeddings --reset-vectors --force` to rebuild"
    )]
    ModelMismatch {
        /// Filesystem path of the vector index directory.
        path: std::path::PathBuf,
        /// The `model_id` recorded in `.meta.json`.
        found_model: String,
        /// The `model_id` the current `Embedder` advertises.
        expected_model: String,
    },

    /// `USearch` library error.
    #[error("USearch error during {context}: {reason}")]
    Usearch {
        /// Short tag naming what the library was doing when `USearch` errored.
        context: &'static str,
        /// Human-readable explanation from `USearch`.
        reason: String,
    },

    /// Neither lexical nor vector index exists for this store.
    #[error(
        "no search index exists for this store; \
         run `singularmem reindex` (and optionally `--with-embeddings`) first"
    )]
    NoIndexes,

    /// User requested `--mode hybrid` but only one of the two indexes exists.
    #[error(
        "hybrid search requires both indexes; {missing} index missing at {path}; \
         run `singularmem reindex --with-embeddings` to build both"
    )]
    HybridMissingIndex {
        /// Which side was missing — `"lexical"` or `"semantic"`.
        missing: &'static str,
        /// Path that was probed.
        path: std::path::PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn no_indexes_error_message_mentions_reindex() {
        let e = Error::NoIndexes;
        let msg = e.to_string();
        assert!(
            msg.contains("no search index exists"),
            "message should explain the failure: got {msg:?}"
        );
        assert!(
            msg.contains("reindex"),
            "message should tell the user the fix: got {msg:?}"
        );
    }

    #[test]
    fn hybrid_missing_index_error_names_the_missing_side() {
        let e = Error::HybridMissingIndex {
            missing: "semantic",
            path: PathBuf::from("/tmp/foo.vectors"),
        };
        let msg = e.to_string();
        assert!(msg.contains("semantic"), "missing side must appear: {msg:?}");
        assert!(
            msg.contains("/tmp/foo.vectors"),
            "path must appear: {msg:?}"
        );
        assert!(
            msg.contains("reindex --with-embeddings"),
            "fix must appear: {msg:?}"
        );
    }
}
