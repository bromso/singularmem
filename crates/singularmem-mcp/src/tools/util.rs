//! Shared helpers used by tool handlers.

use singularmem_core::{Store, StoreOptions};

use crate::{Config, Result};

/// Open the store for read-side handlers, honouring `config.read_only`.
/// When read-only, `SQLite` is opened with `read_only=true` as a third
/// safety layer (in addition to the dispatch-level + list-level guards).
///
/// # Errors
///
/// Returns whatever error `Store::open` / `Store::open_with_options`
/// raises (e.g., I/O, malformed `SQLite` file).
pub fn open_store_for_reading(config: &Config) -> Result<Store> {
    if config.read_only {
        Ok(Store::open_with_options(
            &config.store_path,
            StoreOptions { read_only: true },
        )?)
    } else {
        Ok(Store::open(&config.store_path)?)
    }
}
