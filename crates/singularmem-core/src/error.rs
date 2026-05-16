//! The library's error type. Each variant carries the three pieces Principle VII
//! requires: what failed, what was attempted, what state was preserved.

use crate::item::ItemId;

/// Result alias used throughout the library.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors the library can surface.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// A field on a `NewItem` did not pass validation; the ingest did not run.
    #[error("validation failed for {field}: {reason}; no state changed")]
    Validation {
        /// Name of the field, e.g. `"content"`, `"tags"`, `"metadata"`.
        field: &'static str,
        /// Human-readable explanation.
        reason: String,
    },

    /// `NewItem.supersedes` referenced an ID that does not exist in the store.
    /// The new item was not persisted.
    #[error("supersedes target {id} not found in store; new item was not persisted")]
    SupersedesNotFound { id: ItemId },

    /// A point read or revision walk did not find the requested item.
    #[error("item {id} not found")]
    NotFound { id: ItemId },

    /// `latest_revision` walked forward from an item and found multiple
    /// candidates that nothing supersedes — a fork. The library refuses to
    /// guess (Principle VII).
    #[error("ambiguous latest revision: {} candidates", candidates.len())]
    AmbiguousLatest { candidates: Vec<ItemId> },

    /// The store file is at a format version newer than this binary supports.
    #[error("store format version {found} is newer than supported maximum {max_supported}")]
    UnsupportedFormatVersion {
        found: String,
        max_supported: &'static str,
    },

    /// A write was attempted against a read-only store.
    #[error("store is opened read-only; the {operation} operation requires write access")]
    ReadOnly { operation: &'static str },

    /// A string failed to parse as a ULID.
    #[error("invalid ULID: {0}")]
    InvalidId(#[from] ulid::DecodeError),

    /// `SQLite` reported an error during a named operation. Any transaction was
    /// rolled back.
    #[error("SQLite error during {context}: {source}; rolled back")]
    Sqlite {
        context: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    /// Filesystem or I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation or deserialisation failed (e.g. while emitting export-v1).
    #[error("JSON error during {context}: {source}")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },
}
