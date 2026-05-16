//! Reindex driver. The actual logic lives on `Index::reindex_from`; this
//! module exists to keep the iteration / batching strategy in one named place
//! for future extension.
