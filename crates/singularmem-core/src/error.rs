//! The library's error type. Each variant carries the three pieces Principle VII
//! requires: what failed, what was attempted, what state was preserved.

use crate::id::FactId;
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
    SupersedesNotFound {
        /// The supersedes target that could not be located.
        id: ItemId,
    },

    /// A point read or revision walk did not find the requested item.
    #[error("item {id} not found")]
    NotFound {
        /// The ID that was looked up.
        id: ItemId,
    },

    /// `invalidate_fact`/`supersede_fact` found no open head for the given
    /// triple. Nothing was written. (`supersede_fact` tolerates this for the
    /// old fact and reports `old: None` instead.)
    #[error("no open fact {subject} —{predicate}→ {object}; nothing changed")]
    FactNotFound {
        /// Subject as the caller wrote it.
        subject: String,
        /// Predicate as the caller wrote it.
        predicate: String,
        /// Object as the caller wrote it (entity name or literal value).
        object: String,
    },

    /// A fact id was looked up (`get_fact`, `fact_history`) and does not
    /// exist in the store.
    #[error("fact {id} not found")]
    FactIdNotFound {
        /// The fact id that was looked up.
        id: FactId,
    },

    /// `fact_history` walked forward along `supersedes` and found more than
    /// one revision superseding the same one — a forked chain. The library
    /// refuses to pick a branch (Principle VII).
    #[error(
        "fact chain forks at {} competing revisions; candidates {}",
        candidates.len(),
        candidates.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    AmbiguousFactRevision {
        /// The competing revisions that all supersede the same fact.
        candidates: Vec<FactId>,
    },

    /// `latest_revision` walked forward from an item and found multiple
    /// candidates that nothing supersedes — a fork. The library refuses to
    /// guess (Principle VII).
    #[error("ambiguous latest revision: {} candidates", candidates.len())]
    AmbiguousLatest {
        /// The competing head candidates that all claim to supersede the same prior item.
        candidates: Vec<ItemId>,
    },

    /// The store file is at a format version newer than this binary supports.
    #[error("store format version {found} is newer than supported maximum {max_supported}")]
    UnsupportedFormatVersion {
        /// The version found in the file's `singularmem_meta` row.
        found: String,
        /// The highest version this binary knows how to read.
        max_supported: &'static str,
    },

    /// An in-place format migration failed or was refused. The store is left
    /// at the `from` version; nothing was partially applied.
    #[error("migrating store format {from} -> {to} failed: {reason}; store left at {from}")]
    Migration {
        /// Version found on disk.
        from: String,
        /// Version the binary tried to reach.
        to: &'static str,
        /// Why the migration could not complete.
        reason: String,
    },

    /// A `NewItem.external_id` collides with an existing item's. The new
    /// item was not persisted.
    #[error("external_id {external_id:?} already exists in store; new item was not persisted")]
    ExternalIdConflict {
        /// The colliding external id.
        external_id: String,
    },

    /// A write was attempted against a read-only store.
    #[error("store is opened read-only; the {operation} operation requires write access")]
    ReadOnly {
        /// The name of the write operation that was refused.
        operation: &'static str,
    },

    /// A string failed to parse as a ULID.
    #[error("invalid ULID: {0}")]
    InvalidId(#[from] ulid::DecodeError),

    /// `SQLite` reported an error during a named operation. Any transaction was
    /// rolled back.
    #[error("SQLite error during {context}: {source}; rolled back")]
    Sqlite {
        /// Short tag naming what the library was doing when `SQLite` errored.
        context: &'static str,
        /// The underlying `SQLite` error.
        #[source]
        source: rusqlite::Error,
    },

    /// Filesystem or I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation or deserialisation failed (e.g. while emitting export-v1).
    #[error("JSON error during {context}: {source}")]
    Json {
        /// Short tag naming what the library was doing when JSON failed.
        context: &'static str,
        /// The underlying `serde_json` error.
        #[source]
        source: serde_json::Error,
    },
}
