//! Shared helpers used by tool handlers.

use singularmem_core::{ScopeFilter, Store, StoreOptions};
use singularmem_retrieve::Adapter;

use crate::{Config, Error, Result};

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

/// Open the store for a `memory_graph_*` writer, with no index hooks
/// (facts aren't indexed by Tantivy/`USearch`, unlike items). Callers must
/// check `config.read_only` themselves, exactly as `memory_ingest` does,
/// since the graph writers' error semantics differ per tool.
///
/// # Errors
///
/// Returns whatever error `Store::open` raises (e.g., I/O, malformed
/// `SQLite` file).
pub fn open_store_for_writing(config: &Config) -> Result<Store> {
    Ok(Store::open(&config.store_path)?)
}

/// Build a [`ScopeFilter`] from a tool's `scope` / `scope_exact` arguments.
///
/// `scope: None` means "no scope restriction" regardless of `exact`.
/// `exact: Some(true)` restricts to an exact scope match; otherwise the
/// filter includes descendants.
///
/// # Errors
///
/// Returns [`crate::Error::Core`] wrapping [`singularmem_core::Error::Validation`]
/// (`field: "scope"`) when `scope` fails validation (empty segment, illegal
/// characters, too many segments, etc.).
pub fn scope_filter(scope: Option<&str>, exact: Option<bool>) -> Result<Option<ScopeFilter>> {
    match scope {
        None => Ok(None),
        Some(p) if exact.unwrap_or(false) => Ok(Some(ScopeFilter::exact(p)?)),
        Some(p) => Ok(Some(ScopeFilter::descendants(p)?)),
    }
}

/// Resolve the adapter named by `name`, falling back to
/// `config.default_adapter` when absent. Shared by `memory_retrieve` and
/// `memory_wakeup`.
///
/// # Errors
///
/// Returns [`crate::Error::UnknownAdapter`] when the wanted name is not one
/// of `config.known_adapters`.
pub fn find_adapter<'a>(config: &'a Config, name: Option<&str>) -> Result<&'a dyn Adapter> {
    let wanted = name.unwrap_or(&config.default_adapter);
    config
        .known_adapters
        .iter()
        .map(AsRef::as_ref)
        .find(|a| a.name() == wanted)
        .ok_or_else(|| Error::UnknownAdapter(wanted.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_filter_none_when_scope_absent() {
        assert_eq!(scope_filter(None, None).unwrap(), None);
        assert_eq!(scope_filter(None, Some(true)).unwrap(), None);
    }

    #[test]
    fn scope_filter_descendants_by_default() {
        let f = scope_filter(Some("a/b"), None).unwrap().unwrap();
        assert_eq!(f.path, "a/b");
        assert!(!f.exact);
    }

    #[test]
    fn scope_filter_exact_when_requested() {
        let f = scope_filter(Some("a/b"), Some(true)).unwrap().unwrap();
        assert_eq!(f.path, "a/b");
        assert!(f.exact);
    }

    #[test]
    fn scope_filter_rejects_invalid_scope() {
        let r = scope_filter(Some("a//b"), None);
        assert!(
            matches!(
                r,
                Err(crate::Error::Core(singularmem_core::Error::Validation {
                    field: "scope",
                    ..
                }))
            ),
            "expected Core(Validation{{field: 'scope'}}), got {r:?}"
        );
    }
}
