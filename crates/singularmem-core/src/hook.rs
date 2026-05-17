//! `IndexHook` — extension point for search index implementations.
//!
//! The trait is intentionally minimal: three methods, no associated types,
//! no Tantivy or vector-index types in the signature. Implementations live in
//! external crates (e.g. `singularmem-search` provides a Tantivy impl).
//!
//! Hook failures DO NOT roll back the underlying `SQLite` write. Per
//! Principle VII (Honest Failure Modes), `Store::ingest`'s contract is "the
//! item is durably stored"; if the hook fails afterward, the item is in the
//! store but un-searchable. The hook implementation is expected to log a
//! `tracing::warn!` naming the item ID; the user recovers via
//! `singularmem reindex`.

use crate::{Item, Result};

/// Hook called by `Store::ingest` / `ingest_many` for each persisted `Item`,
/// and by the `reindex` flow for each iterated item.
pub trait IndexHook: Send + Sync {
    /// Called once per newly-persisted item from `ingest` / `ingest_many`.
    /// Errors are logged by the caller, not propagated to the `ingest` result.
    ///
    /// # Errors
    ///
    /// Returns an error if the hook implementation fails. The caller logs
    /// the error but does NOT roll back the `SQLite` write.
    fn on_ingest(&self, item: &Item) -> Result<()>;

    /// Called once per item during a full reindex. Implementations may batch.
    ///
    /// # Errors
    ///
    /// Returns an error if the hook implementation fails.
    fn on_reindex(&self, item: &Item) -> Result<()>;

    /// Called after a reindex batch (or after each single ingest) to commit
    /// pending writes. Errors are logged, not propagated.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit fails.
    fn commit(&self) -> Result<()>;
}
