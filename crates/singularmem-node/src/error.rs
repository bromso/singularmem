//! Translation from `singularmem_core::Error` to `napi::Error` with a
//! stable `.code` property exposed on the JS side.
//!
//! ## napi-rs 2.x error shape
//!
//! `napi::Error<S>` is generic over the status type `S: AsRef<str>`.
//! When napi throws the error into JS it calls `napi_create_error(env,
//! error_code, message, ...)` where `error_code = status.as_ref()`.  Node.js
//! sets the resulting JS Error's `.code` property to `error_code` and
//! `.message` to `reason`.
//!
//! The default `NapiError` type alias is `Error<Status>` (a fixed enum whose
//! `as_ref()` yields strings like `"GenericFailure"`).  To emit a custom
//! `.code` we use `Error<&'static str>` so `status.as_ref()` returns the
//! variant name directly.

use std::ptr;

use napi::{Env, Error as NapiError, JsUnknown, NapiValue};
use singularmem_core::Error as CoreError;

/// Wrapper around the core error so we can implement `From` on a foreign type.
pub struct NodeError(pub CoreError);

impl From<CoreError> for NodeError {
    fn from(e: CoreError) -> Self {
        Self(e)
    }
}

/// Converts `NodeError` into a `napi::Error<&'static str>` whose `status`
/// field is the variant name string.  When napi throws this error into
/// JavaScript the resulting `Error` object has `.code === <variant name>` and
/// `.message === <human-readable description>`.
impl From<NodeError> for NapiError<&'static str> {
    fn from(e: NodeError) -> Self {
        let (code, message): (&'static str, String) = match e.0 {
            CoreError::Validation { field, reason } => (
                "Validation",
                format!("validation failed for {field}: {reason}"),
            ),
            CoreError::SupersedesNotFound { id } => (
                "SupersedesNotFound",
                format!("supersedes target {id} not found"),
            ),
            CoreError::NotFound { id } => ("NotFound", format!("item {id} not found")),
            CoreError::AmbiguousLatest { candidates } => (
                "AmbiguousLatest",
                format!("ambiguous latest revision: {} candidates", candidates.len()),
            ),
            CoreError::UnsupportedFormatVersion {
                found,
                max_supported,
            } => (
                "UnsupportedFormatVersion",
                format!(
                    "store format version {found} is newer than supported maximum {max_supported}"
                ),
            ),
            CoreError::ReadOnly { operation } => (
                "ReadOnly",
                format!("store is read-only; {operation} requires write access"),
            ),
            CoreError::InvalidId(e) => ("InvalidId", format!("invalid ULID: {e}")),
            CoreError::Sqlite { context, source } => {
                ("Sqlite", format!("SQLite error during {context}: {source}"))
            }
            CoreError::Io(e) => ("Io", format!("I/O error: {e}")),
            CoreError::Json { context, source } => {
                ("Json", format!("JSON error during {context}: {source}"))
            }
        };
        NapiError::new(code, message)
    }
}

/// Map a `singularmem_search::Error` to `(code, message)`. Used by both
/// standalone search error paths and the `retrieve::Error::Search` unwrapping.
#[allow(dead_code)] // called by search/retrieve tasks added in Task 3/4
pub fn map_search_error(e: singularmem_search::Error) -> (&'static str, String) {
    use singularmem_search::Error as SE;
    match e {
        SE::Tantivy { context, source } => (
            "Tantivy",
            format!("Tantivy error during {context}: {source}"),
        ),
        SE::QueryParse(msg) => ("QueryParse", format!("could not parse search query: {msg}")),
        SE::IndexMissing { path } => (
            "IndexMissing",
            format!(
                "Tantivy index at {} is missing or unreadable; run `singularmem reindex` to rebuild",
                path.display()
            ),
        ),
        SE::IndexCorrupted { path, reason } => (
            "IndexCorrupted",
            format!(
                "Tantivy index at {} appears corrupted: {reason}; run `singularmem reindex`",
                path.display()
            ),
        ),
        SE::Io(e) => ("Io", format!("I/O error: {e}")),
        SE::Embedding { context, reason } => (
            "Embedding",
            format!("embedding inference failed during {context}: {reason}"),
        ),
        SE::ModelDownload { model, reason } => (
            "ModelDownload",
            format!("could not download embedding model {model}: {reason}"),
        ),
        SE::InvalidModelFiles { path, reason } => (
            "InvalidModelFiles",
            format!(
                "invalid model files at {}: {reason}; expected ONNX weights + tokenizer",
                path.display()
            ),
        ),
        SE::DimMismatch { expected, got } => (
            "DimMismatch",
            format!("vector dimension mismatch: expected {expected}, got {got}"),
        ),
        SE::ModelMismatch {
            path,
            found_model,
            expected_model,
        } => (
            "ModelMismatch",
            format!(
                "vector index at {} was built with model {found_model}; \
                 current Embedder uses {expected_model}; \
                 run `singularmem reindex --with-embeddings --reset-vectors --force` to rebuild",
                path.display()
            ),
        ),
        SE::Usearch { context, reason } => (
            "Usearch",
            format!("USearch error during {context}: {reason}"),
        ),
        SE::NoIndexes => (
            "NoIndexes",
            "no search index exists for this store; \
             run `singularmem reindex` (and optionally `--with-embeddings`) first"
                .to_string(),
        ),
        SE::HybridMissingIndex { missing, path } => (
            "HybridMissingIndex",
            format!(
                "hybrid search requires both indexes; {missing} index missing at {}; \
                 run `singularmem reindex --with-embeddings` to build both",
                path.display()
            ),
        ),
    }
}

/// Build a napi-rs error from a `singularmem_search::Error`.
#[allow(dead_code)] // called by search/retrieve tasks added in Task 3/4
pub fn from_search_error(e: singularmem_search::Error) -> NapiError<&'static str> {
    let (code, message) = map_search_error(e);
    NapiError::new(code, message)
}

/// Build a napi-rs error from a `singularmem_retrieve::Error`. The `Search(_)`
/// and `Core(_)` wrapper variants unwrap to the innermost meaningful `.code`
/// so JS callers don't have to peel layers.
#[allow(dead_code)] // called by retrieve task added in Task 4
pub fn from_retrieve_error(e: singularmem_retrieve::Error) -> NapiError<&'static str> {
    use singularmem_retrieve::Error as RE;
    match e {
        RE::EmptyQuery => NapiError::new("EmptyQuery", "query is empty".to_string()),
        RE::Search(inner) => from_search_error(inner),
        RE::Core(inner) => NodeError::from(inner).into(),
    }
}

/// Build a napi error for invalid store paths surfaced by the binding layer
/// itself (not by the core).
pub fn invalid_store_path(path: &str) -> NapiError<&'static str> {
    NapiError::new(
        "InvalidStorePath",
        format!("store path is not valid: {path}"),
    )
}

// ── raw-backed N-API error helpers ────────────────────────────────────────────

/// Calls `napi_create_error(env, code, message)` via the N-API layer and
/// returns the resulting `napi_value`.
///
/// # Safety
///
/// `raw_env` must be a valid `napi_env` obtained from the current JS
/// thread's `Env` handle. Must not be called from a libuv worker thread.
/// On failure of any inner N-API call the returned pointer may be null;
/// callers must handle that case downstream (napi-rs's `into_value` does).
pub unsafe fn create_js_error(
    raw_env: napi::sys::napi_env,
    code: &str,
    message: &str,
) -> napi::sys::napi_value {
    let mut code_val = ptr::null_mut();
    // N-API failures here leave the pointer as null_mut(); napi-rs handles
    // a null Error value gracefully in JsError::into_value.
    let _ = unsafe {
        napi::sys::napi_create_string_utf8(raw_env, code.as_ptr().cast(), code.len(), &mut code_val)
    };
    let mut msg_val = ptr::null_mut();
    let _ = unsafe {
        napi::sys::napi_create_string_utf8(
            raw_env,
            message.as_ptr().cast(),
            message.len(),
            &mut msg_val,
        )
    };
    let mut js_err = ptr::null_mut();
    let _ = unsafe { napi::sys::napi_create_error(raw_env, code_val, msg_val, &mut js_err) };
    js_err
}

/// Convert a `napi::Error<&'static str>` (our custom-coded error) into an
/// opaque `napi::Error<Status>` whose `maybe_raw` field points at a
/// pre-built JS error object with the correct string `.code`.
/// Must be called on the JS thread.
///
/// napi-rs checks `maybe_raw` first in `JsError::into_value` and returns the
/// pre-built object directly, bypassing its own status-to-code mapping.
pub fn coded_error_to_napi_raw(env: Env, coded: NapiError<&'static str>) -> NapiError {
    let raw_js_err = unsafe { create_js_error(env.raw(), coded.status, &coded.reason) };
    // Wrap in JsUnknown so we can use the From<JsUnknown> impl which stores
    // the value in `maybe_raw`.
    let js_unknown = unsafe { JsUnknown::from_raw_unchecked(env.raw(), raw_js_err) };
    NapiError::from(js_unknown)
}

/// Convert a `NodeError` to a raw-backed `napi::Error<Status>`.
pub fn node_error_to_napi_with_raw(env: Env, err: NodeError) -> NapiError {
    let coded: NapiError<&'static str> = err.into();
    coded_error_to_napi_raw(env, coded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::item::ItemId;
    use std::str::FromStr;

    // All tests check `napi_err.status.as_ref() == <code>` because in
    // napi::Error<S>, `status` becomes JS `.code` and `reason` becomes JS
    // `.message`.  The Display impl formats as `"{:?}, {reason}"` so
    // `to_string().contains("not found")` checks the human message.

    #[test]
    fn not_found_maps_to_code_not_found() {
        let id = ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap();
        let core_err = CoreError::NotFound { id };
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "NotFound");
        assert!(napi_err.reason.contains("not found"));
    }

    #[test]
    fn validation_maps_to_code_validation() {
        let core_err = CoreError::Validation {
            field: "content",
            reason: "empty".to_string(),
        };
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "Validation");
    }

    #[test]
    fn supersedes_not_found_maps_to_code_supersedes_not_found() {
        let id = ItemId::from_str("01HXBBBBBBBBBBBBBBBBBBBBB0").unwrap();
        let core_err = CoreError::SupersedesNotFound { id };
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "SupersedesNotFound");
    }

    #[test]
    fn read_only_maps_to_code_read_only() {
        let core_err = CoreError::ReadOnly {
            operation: "ingest",
        };
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "ReadOnly");
    }

    #[test]
    fn invalid_id_maps_to_code_invalid_id() {
        let core_err: CoreError = "not-a-ulid".parse::<ItemId>().unwrap_err().into();
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "InvalidId");
    }

    #[test]
    fn unsupported_format_version_maps_to_code() {
        let core_err = CoreError::UnsupportedFormatVersion {
            found: "99.0.0".to_string(),
            max_supported: "1.0.0",
        };
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "UnsupportedFormatVersion");
    }

    #[test]
    fn ambiguous_latest_maps_to_code() {
        let core_err = CoreError::AmbiguousLatest { candidates: vec![] };
        let napi_err: NapiError<&'static str> = NodeError::from(core_err).into();
        assert_eq!(napi_err.status, "AmbiguousLatest");
    }

    // ── search / retrieve error mapping tests ─────────────────────────────────

    #[test]
    fn no_indexes_maps_to_code() {
        let napi_err = super::from_search_error(singularmem_search::Error::NoIndexes);
        assert_eq!(napi_err.status, "NoIndexes");
    }

    #[test]
    fn hybrid_missing_index_maps_to_code() {
        let napi_err = super::from_search_error(singularmem_search::Error::HybridMissingIndex {
            missing: "tantivy",
            path: std::path::PathBuf::from("/tmp/x"),
        });
        assert_eq!(napi_err.status, "HybridMissingIndex");
        assert!(napi_err.reason.contains("tantivy"));
    }

    #[test]
    fn empty_query_maps_to_code() {
        let napi_err = super::from_retrieve_error(singularmem_retrieve::Error::EmptyQuery);
        assert_eq!(napi_err.status, "EmptyQuery");
    }

    #[test]
    fn wrapped_search_error_unwraps_to_inner_code() {
        let inner = singularmem_search::Error::NoIndexes;
        let wrapper = singularmem_retrieve::Error::Search(inner);
        let napi_err = super::from_retrieve_error(wrapper);
        assert_eq!(napi_err.status, "NoIndexes");
    }

    #[test]
    fn wrapped_core_error_unwraps_to_inner_code() {
        let id = ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap();
        let inner = CoreError::NotFound { id };
        let wrapper = singularmem_retrieve::Error::Core(inner);
        let napi_err = super::from_retrieve_error(wrapper);
        assert_eq!(napi_err.status, "NotFound");
    }

    #[test]
    fn query_parse_maps_to_code() {
        let napi_err =
            super::from_search_error(singularmem_search::Error::QueryParse("bad".to_string()));
        assert_eq!(napi_err.status, "QueryParse");
    }

    #[test]
    fn index_missing_maps_to_code() {
        let napi_err = super::from_search_error(singularmem_search::Error::IndexMissing {
            path: std::path::PathBuf::from("/tmp/x.tantivy"),
        });
        assert_eq!(napi_err.status, "IndexMissing");
    }

    #[test]
    fn io_in_search_maps_to_io_code() {
        let napi_err = super::from_search_error(singularmem_search::Error::Io(
            std::io::Error::other("disk"),
        ));
        assert_eq!(napi_err.status, "Io");
    }
}
