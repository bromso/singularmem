//! The napi-exposed `Store` class — async wrappers around
//! `singularmem_core::Store` that run blocking `SQLite` work on the libuv
//! thread pool via `napi::Task`.
//!
//! # Design note — why `Task` instead of `async fn`
//!
//! napi-rs `#[napi] async fn` delegates to `execute_tokio_future`, whose
//! signature is `Fut: Future<Output = napi::Result<Data>>`.  `napi::Result`
//! defaults `S = napi::Status` (a fixed enum), so custom string `.code`
//! values produced by `Error<&'static str>` cannot propagate through that
//! machinery.
//!
//! The `Task` trait runs `compute` on the libuv thread pool and `resolve` /
//! `reject` back on the JS thread (with a live `Env`).  In `reject` we can
//! pre-build a JS `Error` object with the exact `.code` we want using raw
//! N-API calls, wrap it in `JsUnknown`, convert it to `napi::Error<Status>`
//! via `From<JsUnknown>` (which stores the pre-built JS value in
//! `maybe_raw`), and return that.  napi-rs then defers to `maybe_raw` when
//! calling `into_value` and uses our pre-built error untouched.

use std::path::PathBuf;
use std::ptr;
use std::sync::Arc;

use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error as NapiError, JsUnknown, NapiValue, Task};
use singularmem_core::{Store as CoreStore, StoreOptions as CoreStoreOptions};

use crate::error::{invalid_store_path, NodeError};

// ── low-level helper: build a JS Error with a custom string code ──────────────

/// Calls `napi_create_error(env, code, message)` via the N-API layer and
/// returns the resulting `napi_value`.  Safe to call from the JS thread only.
unsafe fn create_js_error(
    raw_env: napi::sys::napi_env,
    code: &str,
    message: &str,
) -> napi::sys::napi_value {
    let mut code_val = ptr::null_mut();
    let _ = unsafe {
        napi::sys::napi_create_string_utf8(
            raw_env,
            code.as_ptr().cast(),
            code.len(),
            &mut code_val,
        )
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
fn coded_error_to_napi_raw(env: Env, coded: NapiError<&'static str>) -> NapiError {
    let raw_js_err =
        unsafe { create_js_error(env.raw(), coded.status, &coded.reason) };
    // Wrap in JsUnknown so we can use the From<JsUnknown> impl which stores
    // the value in `maybe_raw`.
    let js_unknown = unsafe { JsUnknown::from_raw_unchecked(env.raw(), raw_js_err) };
    NapiError::from(js_unknown)
}

/// Convert a `NodeError` to a raw-backed `napi::Error<Status>`.
fn node_error_to_napi_with_raw(env: Env, err: NodeError) -> NapiError {
    let coded: NapiError<&'static str> = err.into();
    coded_error_to_napi_raw(env, coded)
}

// ── async task ────────────────────────────────────────────────────────────────

// `pub` so the `private_interfaces` lint is satisfied: `Store::open` is a
// `pub` fn and its return type references this struct.  Since the parent
// module `store` is itself not re-exported, external crates cannot name this
// type directly.
pub struct OpenStoreTask {
    path: PathBuf,
    read_only: bool,
    /// Pre-set validation error (e.g. empty path).  If `Some`, `compute`
    /// immediately returns `Err` so the task rejects the Promise — this
    /// ensures `Store.open('')` returns a *rejected Promise* rather than
    /// throwing synchronously, which is what `assert.rejects` expects.
    pre_error: Option<NapiError<&'static str>>,
    /// Populated by `compute` on error so `reject` can convert it with the
    /// correct string code.
    failed: Option<NodeError>,
}

#[napi]
impl Task for OpenStoreTask {
    type Output = Arc<CoreStore>;
    type JsValue = Store;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        // Surface any pre-validation error as a task failure.
        if self.pre_error.is_some() {
            return Err(NapiError::new(napi::Status::GenericFailure, "pre-validation failed"));
        }
        match CoreStore::open_with_options(
            &self.path,
            CoreStoreOptions { read_only: self.read_only },
        ) {
            Ok(store) => Ok(Arc::new(store)),
            Err(e) => {
                // Store the rich error for `reject`; return a dummy trigger.
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(napi::Status::GenericFailure, "open failed"))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(Store { inner: output })
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        // Pre-validation error takes priority.
        if let Some(coded) = self.pre_error.take() {
            return Err(coded_error_to_napi_raw(env, coded));
        }
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown open error",
            )))
        });
        Err(node_error_to_napi_with_raw(env, node_err))
    }
}

// ── Options ───────────────────────────────────────────────────────────────────

/// Options for `Store.open`.
#[napi(object)]
pub struct StoreOptions {
    /// If true, opens `SQLite` in read-only mode. Writes will error with
    /// `code: "ReadOnly"`.
    pub read_only: Option<bool>,
}

// ── Store class ───────────────────────────────────────────────────────────────

/// A handle to a Singularmem store on disk.
#[napi]
pub struct Store {
    #[allow(dead_code)] // used by later task bindings (get, list, revisions, …)
    pub(crate) inner: Arc<CoreStore>,
}

#[napi]
impl Store {
    /// Open a store at the given filesystem path.
    ///
    /// @param path Absolute or relative path to a `SQLite` file (will be
    ///   created if it does not exist).
    /// @param options Optional `{ readOnly?: boolean }`.
    /// @returns A `Store` instance.
    /// @throws `Error` with `.code` from the standard set
    ///   (`InvalidStorePath`, `Io`, `Sqlite`, `UnsupportedFormatVersion`, …).
    ///
    /// # Errors
    ///
    /// Returns a JS `Error` with `.code = "InvalidStorePath"` if `path` is
    /// empty, `"Sqlite"` on database errors, `"Io"` on filesystem errors, or
    /// `"UnsupportedFormatVersion"` if the file was written by a newer store.
    #[napi]
    pub fn open(
        path: String,
        options: Option<StoreOptions>,
    ) -> napi::Result<AsyncTask<OpenStoreTask>> {
        // Even path-validation errors are deferred into the Task so that
        // `Store.open('')` returns a *rejected Promise* rather than throwing
        // synchronously.  Callers using `await` or `.catch` would handle both,
        // but `assert.rejects` (Node's test helper) only handles async rejects.
        let pre_error = if path.is_empty() {
            Some(invalid_store_path(&path))
        } else {
            None
        };
        let read_only = options.and_then(|o| o.read_only).unwrap_or(false);
        Ok(AsyncTask::new(OpenStoreTask {
            path: PathBuf::from(path),
            read_only,
            pre_error,
            failed: None,
        }))
    }
}
