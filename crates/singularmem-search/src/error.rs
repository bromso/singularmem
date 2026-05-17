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
    Embedding { context: &'static str, reason: String },

    /// Could not download embedding model weights.
    #[error("could not download embedding model {model}: {reason}")]
    ModelDownload { model: String, reason: String },

    /// Model files on disk are missing or invalid.
    #[error("invalid model files at {path}: {reason}; expected ONNX weights + tokenizer")]
    InvalidModelFiles { path: std::path::PathBuf, reason: String },

    /// Vector dimension mismatch between the index metadata and the embedder.
    #[error("vector dimension mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },

    /// The vector index was built with a different model than the current one.
    #[error(
        "vector index at {path} was built with model {found_model}; \
         current Embedder uses {expected_model}; \
         run `singularmem reindex --with-embeddings --reset-vectors --force` to rebuild"
    )]
    ModelMismatch {
        path: std::path::PathBuf,
        found_model: String,
        expected_model: String,
    },

    /// `USearch` library error.
    #[error("USearch error during {context}: {reason}")]
    Usearch { context: &'static str, reason: String },
}
