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
use std::str::FromStr;
use std::sync::Arc;

use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error as NapiError, Task};
use singularmem_core::item::ItemId;
use singularmem_core::{Store as CoreStore, StoreOptions as CoreStoreOptions};

use crate::error::{coded_error_to_napi_raw, invalid_store_path, node_error_to_napi_with_raw, NodeError};

// (raw-backed error helpers live in crate::error)

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

// ── GetTask ──────────────────────────────────────────────────────────────────

pub struct GetTask {
    store: Arc<CoreStore>,
    /// `None` when the ID failed to parse; `compute` will immediately fail
    /// via `pre_error` in that case, so the value is never used.
    id: Option<ItemId>,
    /// Pre-set error (e.g. ULID parse failure).  When `Some`, `compute`
    /// immediately returns `Err` so the task rejects the Promise, ensuring
    /// `store.get('bad')` returns a *rejected Promise* rather than throwing
    /// synchronously (mirrors the `OpenStoreTask::pre_error` pattern).
    pre_error: Option<NapiError<&'static str>>,
    /// Populated by `compute` on core error so `reject` can convert it.
    failed: Option<NodeError>,
}

#[napi]
impl Task for GetTask {
    type Output = singularmem_core::Item;
    type JsValue = crate::types::Item;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        // Surface any pre-validation error (e.g. ULID parse failure) as a
        // task failure so the Promise rejects rather than the method throwing.
        if self.pre_error.is_some() {
            return Err(NapiError::new(napi::Status::GenericFailure, "pre-validation failed"));
        }
        let id = self.id.expect("id must be Some when pre_error is None");
        match self.store.get(id) {
            Ok(item) => Ok(item),
            Err(e) => {
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(napi::Status::GenericFailure, "get failed"))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        // Pre-validation error (InvalidId) takes priority.
        if let Some(coded) = self.pre_error.take() {
            return Err(coded_error_to_napi_raw(env, coded));
        }
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown get error",
            )))
        });
        Err(node_error_to_napi_with_raw(env, node_err))
    }
}

// ── ListTask ─────────────────────────────────────────────────────────────────

pub struct ListTask {
    store: Arc<CoreStore>,
    tags: Vec<String>,
    limit: Option<usize>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for ListTask {
    type Output = Vec<singularmem_core::Item>;
    type JsValue = Vec<crate::types::Item>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        let result: singularmem_core::Result<Vec<singularmem_core::Item>> = (|| {
            let iter = if self.tags.is_empty() {
                self.store.list()?
            } else {
                let refs: Vec<&str> = self.tags.iter().map(String::as_str).collect();
                self.store.list_by_tags(&refs)?
            };
            let mut out = Vec::new();
            for item in iter {
                out.push(item?);
                if let Some(n) = self.limit {
                    if out.len() >= n {
                        break;
                    }
                }
            }
            Ok(out)
        })();
        match result {
            Ok(items) => Ok(items),
            Err(e) => {
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(napi::Status::GenericFailure, "list failed"))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into_iter().map(Into::into).collect())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown list error",
            )))
        });
        Err(node_error_to_napi_with_raw(env, node_err))
    }
}

// ── RevisionsTask ────────────────────────────────────────────────────────────

pub struct RevisionsTask {
    store: Arc<CoreStore>,
    id: Option<ItemId>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for RevisionsTask {
    type Output = Vec<singularmem_core::Item>;
    type JsValue = Vec<crate::types::Item>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        if self.pre_error.is_some() {
            return Err(NapiError::new(napi::Status::GenericFailure, "pre-validation failed"));
        }
        let id = self.id.expect("id must be Some when pre_error is None");
        match self.store.revision_history(id) {
            Ok(mut items) => {
                // Core walks backward (newest → oldest); reverse to oldest → newest.
                items.reverse();
                Ok(items)
            }
            Err(e) => {
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(napi::Status::GenericFailure, "revisions failed"))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into_iter().map(Into::into).collect())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        if let Some(coded) = self.pre_error.take() {
            return Err(coded_error_to_napi_raw(env, coded));
        }
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown revisions error",
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

/// Options for `Store.list`.
#[napi(object)]
pub struct ListOptions {
    /// Only return items tagged with ALL of these tags (AND-semantics).
    pub tags: Option<Vec<String>>,
    /// Maximum number of items to return. Applied after tag filtering.
    pub limit: Option<u32>,
}

// ── Store class ───────────────────────────────────────────────────────────────

/// A handle to a Singularmem store on disk.
#[napi]
pub struct Store {
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
    // Error conditions are documented in the @throws JSDoc above; the
    // `missing_errors_doc` lint does not recognise @throws as a substitute.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
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

    /// Look up a single item by its ULID.
    ///
    /// @param id A 26-character Crockford base32 ULID string.
    /// @returns The matching item.
    /// @throws `Error` with `.code === "NotFound"` if the ID does not exist.
    /// @throws `Error` with `.code === "InvalidId"` if the string is not a valid ULID.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn get(&self, id: String) -> napi::Result<AsyncTask<GetTask>> {
        // Defer ULID parse errors into the Task so that `store.get('bad')`
        // returns a *rejected Promise* rather than throwing synchronously.
        // This mirrors the `Store::open('')` pattern.
        let (item_id, pre_error) = match ItemId::from_str(&id) {
            Ok(id) => (Some(id), None),
            Err(e) => {
                let core_err = singularmem_core::Error::from(e);
                let coded: NapiError<&'static str> = NodeError::from(core_err).into();
                (None, Some(coded))
            }
        };
        Ok(AsyncTask::new(GetTask {
            store: self.inner.clone(),
            id: item_id,
            pre_error,
            failed: None,
        }))
    }

    /// List items in the store, ordered oldest → newest by ingest time.
    ///
    /// @param options Optional `{ tags?: string[]; limit?: number }`.
    ///   When `tags` is given, items must contain ALL listed tags.
    ///   When `limit` is given, the returned array is capped at that length.
    /// @returns Array of items.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn list(&self, options: Option<ListOptions>) -> napi::Result<AsyncTask<ListTask>> {
        #[allow(clippy::cast_possible_truncation)]
        let (tags, limit) = options
            .map(|o| (o.tags.unwrap_or_default(), o.limit.map(|n| n as usize)))
            .unwrap_or_default();
        Ok(AsyncTask::new(ListTask {
            store: self.inner.clone(),
            tags,
            limit,
            failed: None,
        }))
    }

    /// Return the full revision history for an item, ordered oldest to newest.
    ///
    /// @param id The ULID of any item in the chain.
    /// @returns Array of items in chronological order.
    /// @throws `Error` with `.code === "NotFound"` if the ID does not exist.
    /// @throws `Error` with `.code === "InvalidId"` if the string is not a valid ULID.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn revisions(&self, id: String) -> napi::Result<AsyncTask<RevisionsTask>> {
        let (item_id, pre_error) = match ItemId::from_str(&id) {
            Ok(id) => (Some(id), None),
            Err(e) => {
                let core_err = singularmem_core::Error::from(e);
                let coded: NapiError<&'static str> = NodeError::from(core_err).into();
                (None, Some(coded))
            }
        };
        Ok(AsyncTask::new(RevisionsTask {
            store: self.inner.clone(),
            id: item_id,
            pre_error,
            failed: None,
        }))
    }
}
