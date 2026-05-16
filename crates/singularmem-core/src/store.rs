//! The `Store` type — SQLite-backed memory store.
//!
//! `Store::open` is the primary entry point. See also `StoreOptions` for
//! tuning how the store is opened.
//!
//! Full implementation arrives in Task 5 (Phase C).

/// Options controlling how a [`Store`] is opened.
#[derive(Debug, Default, Clone)]
pub struct StoreOptions {
    /// Open in read-only mode. Writes will fail with [`crate::Error::ReadOnly`].
    pub read_only: bool,
}

/// SQLite-backed memory store.
///
/// # Note
///
/// This is a stub — full implementation arrives in Phase C (Task 5).
#[derive(Debug)]
pub struct Store;
