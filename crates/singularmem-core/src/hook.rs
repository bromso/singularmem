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

/// Composite `IndexHook` that fans calls out to multiple underlying hooks.
///
/// Each hook runs independently; one hook's failure does NOT prevent later
/// hooks from running (the loop catches errors per-hook, logs via
/// `tracing::warn!`, and returns the FIRST error after all hooks have been
/// tried).
///
/// Use this when you need to wire two or more `IndexHook` implementations
/// (e.g. Tantivy lexical + `USearch` vector) into a single `Store`. The
/// `Store::open_with_hooks` constructor wraps a `Vec<Box<dyn IndexHook>>`
/// into a `MultiHook` for you.
pub struct MultiHook {
    hooks: Vec<Box<dyn IndexHook>>,
}

impl MultiHook {
    /// Construct from an ordered list of hooks. Order is preserved: hooks
    /// run in the order given, which only matters for visibility of
    /// `tracing::warn!` lines.
    #[must_use]
    pub fn new(hooks: Vec<Box<dyn IndexHook>>) -> Self {
        Self { hooks }
    }
}

impl IndexHook for MultiHook {
    fn on_ingest(&self, item: &crate::Item) -> crate::Result<()> {
        run_all(self.hooks.iter(), "on_ingest", |h| h.on_ingest(item))
    }

    fn on_reindex(&self, item: &crate::Item) -> crate::Result<()> {
        run_all(self.hooks.iter(), "on_reindex", |h| h.on_reindex(item))
    }

    fn commit(&self) -> crate::Result<()> {
        run_all(self.hooks.iter(), "commit", |h| h.commit())
    }
}

fn run_all<'a, I, F>(hooks: I, op: &'static str, mut call: F) -> crate::Result<()>
where
    I: Iterator<Item = &'a Box<dyn IndexHook>>,
    F: FnMut(&dyn IndexHook) -> crate::Result<()>,
{
    let mut first_err: Option<crate::Error> = None;
    for (i, hook) in hooks.enumerate() {
        if let Err(e) = call(hook.as_ref()) {
            tracing::warn!(
                hook_index = i,
                op = op,
                error = %e,
                "MultiHook member failed; other hooks will still run"
            );
            if first_err.is_none() {
                first_err = Some(e);
            }
        }
    }
    first_err.map_or(Ok(()), Err)
}
