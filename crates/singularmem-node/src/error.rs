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

use napi::Error as NapiError;
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
                format!(
                    "ambiguous latest revision: {} candidates",
                    candidates.len()
                ),
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
            CoreError::Sqlite { context, source } => (
                "Sqlite",
                format!("SQLite error during {context}: {source}"),
            ),
            CoreError::Io(e) => ("Io", format!("I/O error: {e}")),
            CoreError::Json { context, source } => (
                "Json",
                format!("JSON error during {context}: {source}"),
            ),
        };
        NapiError::new(code, message)
    }
}

/// Build a napi error for invalid store paths surfaced by the binding layer
/// itself (not by the core).
#[allow(dead_code)] // used by binding functions added in later tasks
pub fn invalid_store_path(path: &str) -> NapiError<&'static str> {
    NapiError::new(
        "InvalidStorePath",
        format!("store path is not valid: {path}"),
    )
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
        let core_err = CoreError::ReadOnly { operation: "ingest" };
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
}
