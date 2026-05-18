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

// ── FormatVersionTask ────────────────────────────────────────────────────────

pub struct FormatVersionTask {
    store: Arc<CoreStore>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for FormatVersionTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        match self.store.format_version() {
            Ok(v) => Ok(v),
            Err(e) => {
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(napi::Status::GenericFailure, "format_version failed"))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output)
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown format_version error",
            )))
        });
        Err(node_error_to_napi_with_raw(env, node_err))
    }
}

// ── ExportTask ───────────────────────────────────────────────────────────────

pub struct ExportTask {
    store: Arc<CoreStore>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for ExportTask {
    type Output = Vec<u8>;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        let mut buf: Vec<u8> = Vec::new();
        match self.store.export(&mut buf) {
            Ok(()) => Ok(buf),
            Err(e) => {
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(napi::Status::GenericFailure, "export failed"))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        String::from_utf8(output).map_err(|e| {
            NapiError::new(
                napi::Status::GenericFailure,
                format!("export produced non-UTF-8 bytes: {e}"),
            )
        })
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown export error",
            )))
        });
        Err(node_error_to_napi_with_raw(env, node_err))
    }
}

// ── Options ───────────────────────────────────────────────────────────────────

/// Options passed to `Store.open`.
#[napi(object)]
pub struct StoreOptions {
    /// When `true`, the underlying `SQLite` database is opened in read-only
    /// mode. Any attempt to write (insert, update, delete) will reject with
    /// `code: "ReadOnly"`.
    ///
    /// Defaults to `false` (read-write) when omitted.
    pub read_only: Option<bool>,
}

/// Options passed to `Store.list`.
#[napi(object)]
pub struct ListOptions {
    /// Restrict results to items that have ALL of these tags (AND-semantics).
    ///
    /// An item is included only if every tag in this array appears in the
    /// item's `tags` field. An empty array (or omitting `tags` entirely)
    /// returns all items without tag filtering.
    pub tags: Option<Vec<String>>,
    /// Cap the number of items returned at this value.
    ///
    /// The limit is applied after tag filtering, on the oldest-first ordered
    /// result set. Omit or pass `undefined` to return all matching items.
    pub limit: Option<u32>,
}

// ── Store class ───────────────────────────────────────────────────────────────

/// A handle to a Singularmem store on disk.
///
/// Obtain an instance via the async static factory `Store.open`. All methods
/// are async and resolve on the libuv thread pool so the JS event loop is
/// never blocked.
///
/// Errors are always thrown as `Error` objects with a structured `.code`
/// string property. See the README for the full list of possible codes.
#[napi]
pub struct Store {
    pub(crate) inner: Arc<CoreStore>,
}

#[napi]
impl Store {
    /// Open (or create) a Singularmem store at the given filesystem path.
    ///
    /// The `SQLite` database file is created automatically if it does not exist.
    /// Schema migrations run at open time; if the on-disk format is newer than
    /// this binding supports the promise rejects with `UnsupportedFormatVersion`.
    ///
    /// @param path Absolute or relative path to the `SQLite` database file.
    ///   Must be a non-empty string; rejects with `InvalidStorePath` otherwise.
    /// @param options Optional open options (see `StoreOptions`).
    /// @returns A ready-to-use `Store` instance.
    /// @throws `{ code: "InvalidStorePath" }` — path is empty or otherwise invalid.
    /// @throws `{ code: "UnsupportedFormatVersion" }` — store file is newer than this binding supports.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error (e.g. permissions, corrupt file).
    /// @throws `{ code: "Io" }` — filesystem or I/O error.
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

    /// Retrieve a single item by its ULID string.
    ///
    /// @param id A 26-character Crockford base32 ULID string identifying the item.
    /// @returns The matching `Item`.
    /// @throws `{ code: "NotFound" }` — no item with that ID exists in the store.
    /// @throws `{ code: "InvalidId" }` — `id` is not a valid 26-character ULID string.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
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

    /// List items in the store, ordered oldest to newest by ingest time.
    ///
    /// **Tag filtering (AND-semantics):** when `options.tags` is provided,
    /// only items that carry *every* listed tag are returned. An empty `tags`
    /// array is equivalent to omitting the field (no filtering).
    ///
    /// **Limit:** when `options.limit` is provided, the result array is
    /// truncated to at most that many items after tag filtering is applied.
    ///
    /// @param options Optional filtering and pagination options (see `ListOptions`).
    /// @returns Array of matching `Item` objects, oldest first.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
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

    /// Return the full revision history for a logical memory entry.
    ///
    /// Pass the ULID of *any* item in a supersession chain (not necessarily
    /// the oldest or newest). The method walks the chain and returns every
    /// revision ordered oldest to newest (i.e. the first element was ingested
    /// first and each subsequent element supersedes the previous one).
    ///
    /// @param id A 26-character Crockford base32 ULID of any item in the chain.
    /// @returns Array of `Item` objects in chronological order (oldest first).
    /// @throws `{ code: "NotFound" }` — no item with that ID exists in the store.
    /// @throws `{ code: "InvalidId" }` — `id` is not a valid 26-character ULID string.
    /// @throws `{ code: "AmbiguousLatest" }` — the revision chain forks (data integrity error).
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
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

    /// Return the on-disk format version recorded in this store file.
    ///
    /// The version is a semver string (e.g. `"1.0.0"`) that identifies the
    /// schema and serialisation format used by the store. This value is written
    /// once when the store is first created and is validated at open time; if
    /// the version is newer than what this binding understands, `Store.open`
    /// rejects with `UnsupportedFormatVersion`.
    ///
    /// @returns A semver version string such as `"1.0.0"`.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn format_version(&self) -> napi::Result<AsyncTask<FormatVersionTask>> {
        Ok(AsyncTask::new(FormatVersionTask {
            store: self.inner.clone(),
            failed: None,
        }))
    }

    /// Export the entire store as a JSONL (newline-delimited JSON) string.
    ///
    /// The output format is one JSON object per line:
    /// - **Line 1** — a metadata header object, e.g.
    ///   `{"type":"meta","formatVersion":"1.0.0"}`.
    /// - **Remaining lines** — one `Item`-shaped JSON object per stored item,
    ///   in oldest-first order.
    ///
    /// The returned string is UTF-8 encoded. It can be written directly to a
    /// `.jsonl` file for backup or migration purposes.
    ///
    /// @returns The full JSONL payload as a UTF-8 string.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    /// @throws `{ code: "Json" }` — JSON serialisation error (should not occur in normal use).
    /// @throws `{ code: "Io" }` — I/O error writing to the internal buffer.
    #[napi(js_name = "export")]
    #[allow(clippy::missing_errors_doc)]
    pub fn export_jsonl(&self) -> napi::Result<AsyncTask<ExportTask>> {
        Ok(AsyncTask::new(ExportTask {
            store: self.inner.clone(),
            failed: None,
        }))
    }
}
