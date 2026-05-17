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
}
