//! Error type for ingestion sources. Every variant names the file involved.

use std::path::PathBuf;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors this crate surfaces.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The store rejected or failed an operation.
    #[error(transparent)]
    Core(#[from] singularmem_core::Error),

    /// Reading a path failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path being read.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A JSONL line could not be parsed. The line is skipped.
    #[error("malformed JSON at {path}:{line}: {source}")]
    Json {
        /// The transcript file.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Underlying error.
        #[source]
        source: serde_json::Error,
    },

    /// A path given by the caller does not exist.
    #[error("path not found: {path}")]
    NotFound {
        /// The missing path.
        path: PathBuf,
    },
}
