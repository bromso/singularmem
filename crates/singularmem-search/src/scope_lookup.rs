//! `ScopeLookup` — how the semantic ranker learns an item's scope without
//! the vector sidecar storing one.
//!
//! `USearch` indexes vectors keyed by `ItemId` and has no filter hook, so a
//! scoped semantic search fetches candidates first and drops the ones whose
//! scope does not match. The scope comes from this trait, normally backed by
//! the `SQLite` store the vectors were built from.

use singularmem_core::ItemId;

/// Resolve an item's scope. Implemented by `singularmem_core::Store`;
/// tests may implement it with a `HashMap`.
pub trait ScopeLookup {
    /// The item's scope, or `None` if unscoped or unknown.
    fn scope_of(&self, id: ItemId) -> Option<String>;
}

/// A store error during lookup is logged and the item is treated as
/// unscoped; under an active filter that means the hit is dropped rather
/// than let through (fail-closed).
impl ScopeLookup for singularmem_core::Store {
    fn scope_of(&self, id: ItemId) -> Option<String> {
        // `Self::scope_of` is the store's inherent method, not this trait's.
        match Self::scope_of(self, id) {
            Ok(scope) => scope,
            Err(e) => {
                tracing::warn!(item_id = %id, error = %e, "scope lookup failed; treating hit as unscoped");
                None
            }
        }
    }
}
