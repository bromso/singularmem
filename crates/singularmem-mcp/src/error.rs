//! Error type for the MCP server.

use std::path::PathBuf;

/// Alias for `std::result::Result<T, Error>` used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by the MCP server.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Server is launched in read-only mode and the request would write.
    #[error("server is read-only; memory_ingest is disabled")]
    ReadOnly,

    /// Could not parse an `ItemId` argument.
    #[error("invalid item ID: {0}")]
    InvalidId(String),

    /// Underlying retrieve-crate failure.
    #[error("{0}")]
    Retrieve(#[from] singularmem_retrieve::Error),

    /// Underlying search-crate failure (bubbled through retrieve).
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),

    /// Underlying core-crate failure (bubbled through retrieve).
    #[error("{0}")]
    Core(#[from] singularmem_core::Error),

    /// Client requested an adapter name not in the registry.
    #[error("unknown adapter '{0}'; known adapters: plain, claude, openai, gemini")]
    UnknownAdapter(String),

    /// I/O error during transport setup or store I/O.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Store path is invalid (e.g., parent dir doesn't exist).
    #[error("invalid store path {path}: {reason}")]
    InvalidStorePath {
        /// The path that was attempted.
        path: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
}
