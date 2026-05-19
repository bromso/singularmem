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

use crate::error::{
    coded_error_to_napi_raw, invalid_store_path, node_error_to_napi_with_raw, NodeError,
};

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
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
        }
        match CoreStore::open_with_options(
            &self.path,
            CoreStoreOptions {
                read_only: self.read_only,
            },
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
        Ok(Store {
            inner: output,
            path: self.path.clone(),
            read_only: self.read_only,
        })
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
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
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
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
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
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "revisions failed",
                ))
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
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "format_version failed",
                ))
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
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "export failed",
                ))
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

// ── SearchMode ────────────────────────────────────────────────────────────────

/// Internal search mode — never exposed to JS.
#[derive(Debug, Clone, Copy)]
enum SearchMode {
    Auto,
    Lexical,
    Semantic,
    Hybrid,
}

impl SearchMode {
    fn from_optional_str(s: Option<&str>) -> Result<Self, NapiError<&'static str>> {
        match s.unwrap_or("auto") {
            "auto" => Ok(Self::Auto),
            "lexical" => Ok(Self::Lexical),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(NapiError::new(
                "Validation",
                format!(
                    "unknown search mode '{other}'; expected one of: auto, lexical, semantic, hybrid"
                ),
            )),
        }
    }
}

// ── Sidecar helpers ───────────────────────────────────────────────────────────

/// Open whatever sidecar indexes exist at the given store path. Returns
/// `(tantivy, vectors)` where either may be `None` if the corresponding
/// sidecar directory doesn't exist. Honors `SINGULARMEM_TEST_EMBEDDER=mock`
/// to mirror the MCP server pattern.
#[allow(clippy::unnecessary_wraps)] // Result kept for call-site ? compatibility (Task 3 may re-add early-exit paths)
fn open_sidecars(
    store_path: &std::path::Path,
) -> Result<
    (
        Option<singularmem_search::Index>,
        Option<singularmem_search::EmbedderIndex>,
    ),
    NapiError<&'static str>,
> {
    let tantivy_path = {
        let mut p = store_path.as_os_str().to_owned();
        p.push(".tantivy");
        std::path::PathBuf::from(p)
    };
    let vectors_path = {
        let mut p = store_path.as_os_str().to_owned();
        p.push(".vectors");
        std::path::PathBuf::from(p)
    };

    let tantivy = if tantivy_path.exists() {
        match singularmem_search::Index::open(&tantivy_path) {
            Ok(idx) => Some(idx),
            Err(e) => {
                tracing::warn!(
                    path = %tantivy_path.display(),
                    error = %e,
                    "Tantivy sidecar present but failed to open; skipping"
                );
                None
            }
        }
    } else {
        None
    };

    let vectors = if vectors_path.exists() {
        let embedder: Box<dyn singularmem_search::Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => match singularmem_search::FastembedEmbedder::new() {
                    Ok(e) => Box::new(e),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "FastembedEmbedder::new() failed; vectors sidecar will be skipped"
                        );
                        return Ok((tantivy, None));
                    }
                },
            };
        match singularmem_search::EmbedderIndex::open(&vectors_path, embedder) {
            Ok(idx) => Some(idx),
            Err(e) => {
                tracing::warn!(
                    path = %vectors_path.display(),
                    error = %e,
                    "Vectors sidecar present but failed to open; skipping"
                );
                None
            }
        }
    } else {
        None
    };

    Ok((tantivy, vectors))
}

/// Construct a `HybridSearcher` given the requested mode and the sidecars
/// that were opened. Returns the appropriate coded `NapiError` if the mode
/// requires a sidecar that isn't present.
fn build_searcher<'a>(
    mode: SearchMode,
    tantivy: Option<&'a singularmem_search::Index>,
    vectors: Option<&'a singularmem_search::EmbedderIndex>,
    store_path: &std::path::Path,
) -> Result<singularmem_search::HybridSearcher<'a>, NapiError<&'static str>> {
    let tantivy_path = {
        let mut p = store_path.as_os_str().to_owned();
        p.push(".tantivy");
        std::path::PathBuf::from(p)
    };
    let vectors_path = {
        let mut p = store_path.as_os_str().to_owned();
        p.push(".vectors");
        std::path::PathBuf::from(p)
    };

    Ok(match mode {
        SearchMode::Lexical => {
            let t = tantivy.ok_or_else(|| {
                crate::error::from_search_error(singularmem_search::Error::IndexMissing {
                    path: tantivy_path,
                })
            })?;
            singularmem_search::HybridSearcher::lexical_only(t)
        }
        SearchMode::Semantic => {
            let v = vectors.ok_or_else(|| {
                crate::error::from_search_error(singularmem_search::Error::IndexMissing {
                    path: vectors_path,
                })
            })?;
            singularmem_search::HybridSearcher::semantic_only(v)
        }
        SearchMode::Hybrid => {
            let t = tantivy.ok_or_else(|| {
                crate::error::from_search_error(singularmem_search::Error::HybridMissingIndex {
                    missing: "lexical",
                    path: tantivy_path,
                })
            })?;
            let v = vectors.ok_or_else(|| {
                crate::error::from_search_error(singularmem_search::Error::HybridMissingIndex {
                    missing: "semantic",
                    path: vectors_path,
                })
            })?;
            singularmem_search::HybridSearcher::new(t, v)
        }
        SearchMode::Auto => match (tantivy, vectors) {
            (Some(t), Some(v)) => singularmem_search::HybridSearcher::new(t, v),
            (Some(t), None) => singularmem_search::HybridSearcher::lexical_only(t),
            (None, Some(v)) => singularmem_search::HybridSearcher::semantic_only(v),
            (None, None) => {
                return Err(crate::error::from_search_error(
                    singularmem_search::Error::NoIndexes,
                ));
            }
        },
    })
}

// ── SearchTask ────────────────────────────────────────────────────────────────

pub struct SearchTask {
    store: Arc<CoreStore>,
    store_path: PathBuf,
    query: String,
    mode: SearchMode,
    limit: usize,
    fetch_multiplier: usize,
    rrf_k: usize,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NapiError<&'static str>>,
}

#[napi]
impl Task for SearchTask {
    type Output = SearchOutput;
    type JsValue = crate::types::SearchResults;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        if self.pre_error.is_some() {
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
        }
        match run_search(
            &self.store,
            &self.store_path,
            &self.query,
            self.mode,
            self.limit,
            self.fetch_multiplier,
            self.rrf_k,
        ) {
            Ok(out) => Ok(out),
            Err(coded) => {
                self.failed = Some(coded);
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "search failed",
                ))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        let (query, pairs) = output;
        let hits = pairs
            .into_iter()
            .map(|(hit, item)| crate::types::SearchHit::from_parts(hit, item))
            .collect();
        Ok(crate::types::SearchResults { query, hits })
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        if let Some(coded) = self.pre_error.take() {
            return Err(crate::error::coded_error_to_napi_raw(env, coded));
        }
        if let Some(coded) = self.failed.take() {
            return Err(crate::error::coded_error_to_napi_raw(env, coded));
        }
        Err(NapiError::new(
            napi::Status::GenericFailure,
            "unknown search error",
        ))
    }
}

/// Return type of `run_search`: echoed query + hit/item pairs.
type SearchOutput = (
    String,
    Vec<(singularmem_search::HybridHit, singularmem_core::Item)>,
);

/// Probe sidecars, resolve mode, run hybrid search, enrich each hit with
/// its full `Item`. Returns `(echo_of_query, vec_of_(hit, item))`.
///
/// # Errors
///
/// Returns a `NapiError<&'static str>` with the appropriate code when index
/// probing, construction, or the search itself fails.
fn run_search(
    store: &Arc<CoreStore>,
    store_path: &std::path::Path,
    query: &str,
    mode: SearchMode,
    limit: usize,
    fetch_multiplier: usize,
    rrf_k: usize,
) -> Result<SearchOutput, NapiError<&'static str>> {
    let (tantivy, vectors) = open_sidecars(store_path)?;
    let searcher = build_searcher(mode, tantivy.as_ref(), vectors.as_ref(), store_path)?;

    let opts = singularmem_search::HybridSearchOptions {
        limit,
        fetch_multiplier,
        rrf_k,
        include_snippets: false, // snippets not exposed in 5b
    };
    let results = searcher
        .search(query, &opts)
        .map_err(crate::error::from_search_error)?;

    let mut pairs = Vec::with_capacity(results.hits.len());
    for hit in results.hits {
        let item = store.get(hit.id).map_err(|e| {
            let node_err: NapiError<&'static str> = NodeError::from(e).into();
            node_err
        })?;
        pairs.push((hit, item));
    }
    Ok((query.to_string(), pairs))
}

// ── RetrieveTask ─────────────────────────────────────────────────────────────

pub struct RetrieveTask {
    store: Arc<CoreStore>,
    store_path: PathBuf,
    query: String,
    mode: SearchMode,
    limit: usize,
    fetch_multiplier: usize,
    rrf_k: usize,
    min_score: f32,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NapiError<&'static str>>,
}

#[napi]
impl Task for RetrieveTask {
    type Output = singularmem_retrieve::RetrievedContext;
    type JsValue = crate::types::RetrievedContext;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        if self.pre_error.is_some() {
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
        }
        match run_retrieve(
            &self.store,
            &self.store_path,
            &self.query,
            self.mode,
            self.limit,
            self.fetch_multiplier,
            self.rrf_k,
            self.min_score,
        ) {
            Ok(ctx) => Ok(ctx),
            Err(coded) => {
                self.failed = Some(coded);
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "retrieve failed",
                ))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        if let Some(coded) = self.pre_error.take() {
            return Err(coded_error_to_napi_raw(env, coded));
        }
        if let Some(coded) = self.failed.take() {
            return Err(coded_error_to_napi_raw(env, coded));
        }
        Err(NapiError::new(
            napi::Status::GenericFailure,
            "unknown retrieve error",
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn run_retrieve(
    store: &Arc<CoreStore>,
    store_path: &std::path::Path,
    query: &str,
    mode: SearchMode,
    limit: usize,
    fetch_multiplier: usize,
    rrf_k: usize,
    min_score: f32,
) -> Result<singularmem_retrieve::RetrievedContext, NapiError<&'static str>> {
    let (tantivy, vectors) = open_sidecars(store_path)?;
    let searcher = build_searcher(mode, tantivy.as_ref(), vectors.as_ref(), store_path)?;
    let retriever = singularmem_retrieve::Retriever::new(store, &searcher);

    let opts = singularmem_retrieve::RetrieveOptions {
        max_blocks: limit,
        min_score,
        search: singularmem_search::HybridSearchOptions {
            limit,
            fetch_multiplier,
            rrf_k,
            include_snippets: false,
        },
    };
    retriever
        .retrieve(query, &opts)
        .map_err(crate::error::from_retrieve_error)
}

// ── IngestTask ───────────────────────────────────────────────────────────────

pub struct IngestTask {
    store: Arc<CoreStore>,
    store_path: PathBuf,
    read_only: bool,
    new_item: Option<singularmem_core::NewItem>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for IngestTask {
    type Output = singularmem_core::Item;
    type JsValue = crate::types::Item;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        if self.pre_error.is_some() {
            return Err(NapiError::new(
                napi::Status::GenericFailure,
                "pre-validation failed",
            ));
        }
        let new_item = self
            .new_item
            .take()
            .expect("new_item must be Some when pre_error is None");

        // Per-call hook wiring: open sidecars, attach to a fresh owned store
        // for the duration of this single ingest, then detach so subsequent
        // search/retrieve per-call opens don't conflict with the writer lock.
        // Matches singularmem-mcp's open_store_with_hooks pattern.
        match run_ingest_with_hooks(&self.store, &self.store_path, self.read_only, new_item) {
            Ok(item) => Ok(item),
            Err(e) => {
                self.failed = Some(NodeError::from(e));
                Err(NapiError::new(
                    napi::Status::GenericFailure,
                    "ingest failed",
                ))
            }
        }
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.into())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        if let Some(coded) = self.pre_error.take() {
            return Err(coded_error_to_napi_raw(env, coded));
        }
        let node_err = self.failed.take().unwrap_or_else(|| {
            NodeError::from(singularmem_core::Error::Io(std::io::Error::other(
                "unknown ingest error",
            )))
        });
        Err(node_error_to_napi_with_raw(env, node_err))
    }
}

/// Open sidecar indexes, attach as hooks on a freshly-opened store, run ingest,
/// then drop the owned store (so the writer locks are released before any future
/// search/retrieve probe in the same process). Matches the MCP server's
/// `open_store_with_hooks` pattern documented in
/// `singularmem-mcp/src/tools/ingest.rs`.
///
/// This per-call wiring is intentional v0 duplication with the MCP code
/// (~30 lines). A future shared helper (e.g., `singularmem-core::with_hooks`)
/// can extract once a third consumer appears.
///
/// Errors from the core's ingest path propagate; hook write failures are
/// logged inside the core's ingest implementation (Principle VII).
fn run_ingest_with_hooks(
    _store: &Arc<CoreStore>,
    store_path: &std::path::Path,
    read_only: bool,
    new_item: singularmem_core::NewItem,
) -> singularmem_core::Result<singularmem_core::Item> {
    // `Arc<CoreStore>` doesn't give us `&mut` for set_hook, so we open a
    // fresh owned CoreStore here (option B from the Task spec). SQLite
    // serialises writes, so this is safe. The `_store` param is kept for
    // API symmetry with the other task helpers; it is otherwise unused.
    //
    // Read-only mode: open the store read-only so the core's ingest path
    // returns `Error::ReadOnly` immediately (no hooks opened; read-only
    // stores have no write lock to hold).

    if read_only {
        let ro_store =
            CoreStore::open_with_options(store_path, CoreStoreOptions { read_only: true })?;
        return ro_store.ingest(new_item);
    }

    let (tantivy, vectors) = open_sidecars(store_path).unwrap_or_else(|e| {
        tracing::warn!(
            error = ?e,
            "sidecar probe failed during ingest; this ingest will write SQLite only"
        );
        (None, None)
    });

    let mut hooks: Vec<Box<dyn singularmem_core::IndexHook>> = Vec::new();
    if let Some(idx) = tantivy {
        hooks.push(Box::new(idx));
    }
    if let Some(idx) = vectors {
        hooks.push(Box::new(idx));
    }

    // Open a fresh owned store, wire hooks if any, ingest, then drop.
    let owned_store = if hooks.is_empty() {
        CoreStore::open_with_options(store_path, CoreStoreOptions { read_only: false })?
    } else {
        CoreStore::open_with_hooks(store_path, hooks)?
    };

    // Ingest. Hooks fire here (or are skipped if absent). The owned_store
    // drops at end of scope, releasing the writer locks and SQLite handle.
    let result = owned_store.ingest(new_item);

    drop(owned_store);

    result
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

/// Options for `Store.search`.
#[napi(object)]
pub struct SearchOptions {
    /// Search mode. One of: `"auto"` | `"lexical"` | `"semantic"` | `"hybrid"`.
    ///
    /// - `"auto"` (default) — probe which sidecars exist; run hybrid if both are
    ///   present, fall back to whichever single ranker is available.
    /// - `"lexical"` — Tantivy BM25 only; rejects with `IndexMissing` if absent.
    /// - `"semantic"` — `USearch` cosine only; rejects with `IndexMissing` if absent.
    /// - `"hybrid"` — both rankers, RRF-fused; rejects with `HybridMissingIndex`
    ///   if either sidecar is missing.
    pub mode: Option<String>,
    /// Maximum number of hits to return. Default: `10`.
    pub limit: Option<u32>,
    /// Per-ranker overfetch factor used to build candidates before fusion or
    /// trimming. Effective candidates = `limit × fetchMultiplier`. Default: `3`.
    pub fetch_multiplier: Option<u32>,
    /// Reciprocal Rank Fusion damping constant. Higher values reduce the impact
    /// of high-ranked documents. Default: `60`.
    pub rrf_k: Option<u32>,
}

/// Options for `Store.retrieve`.
#[napi(object)]
pub struct RetrieveOptions {
    /// Search mode. One of: `"auto"` | `"lexical"` | `"semantic"` | `"hybrid"`.
    /// Default: `"auto"`. Semantics are identical to `SearchOptions.mode`.
    pub mode: Option<String>,
    /// Maximum number of blocks to return (after score filtering). Default: `10`.
    pub limit: Option<u32>,
    /// Per-ranker overfetch factor. Default: `3`. Same meaning as in
    /// `SearchOptions.fetchMultiplier`.
    pub fetch_multiplier: Option<u32>,
    /// Reciprocal Rank Fusion damping constant. Default: `60`. Same meaning as
    /// in `SearchOptions.rrfK`.
    pub rrf_k: Option<u32>,
    /// Drop blocks whose score is strictly below this threshold. Default: `0.0`
    /// (keep all blocks). Useful for filtering out low-relevance results before
    /// passing to an adapter.
    pub min_score: Option<f64>,
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
    /// Retained so search/retrieve methods can compute sidecar paths.
    pub(crate) path: PathBuf,
    /// Mirror of the `read_only` flag from `Store.open` options. Used by
    /// `IngestTask` to open a fresh per-call store with the correct mode.
    pub(crate) read_only: bool,
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

    /// Run a hybrid search over the store's indexes.
    ///
    /// Probes for Tantivy (`.tantivy`) and vector (`.vectors`) sidecars next to
    /// the store file on each call (no caching). The `mode` option controls
    /// which rankers run; defaults to `"auto"` (use whatever's available).
    ///
    /// Build indexes first via the CLI:
    /// `singularmem reindex --with-embeddings --store ./memory.db`
    ///
    /// @param query The natural-language search query string.
    /// @param options Optional `{ mode?, limit?, fetchMultiplier?, rrfK? }`.
    /// @returns `SearchResults` with the query echoed and per-hit `Item` content.
    /// @throws `{ code: "NoIndexes" }` — `mode: "auto"` but no sidecar directories exist.
    /// @throws `{ code: "HybridMissingIndex" }` — `mode: "hybrid"` and one of the two sidecars is absent.
    /// @throws `{ code: "IndexMissing" }` — explicit `mode: "lexical"` or `"semantic"` requires a sidecar that is absent.
    /// @throws `{ code: "IndexCorrupted" }` — a sidecar directory exists but cannot be opened.
    /// @throws `{ code: "Validation" }` — the `mode` string is not one of the four recognised values.
    /// @throws `{ code: "QueryParse" }` — Tantivy could not parse the query string.
    /// @throws `{ code: "Tantivy" }` — Tantivy runtime error during search.
    /// @throws `{ code: "Usearch" }` — `USearch` runtime error during search.
    /// @throws `{ code: "Embedding" }` — embedder runtime error while encoding the query.
    /// @throws `{ code: "ModelDownload" }` — fastembed model files could not be downloaded.
    /// @throws `{ code: "InvalidModelFiles" }` — embedder model files on disk are malformed.
    /// @throws `{ code: "DimMismatch" }` — the vector index was built with a different embedding dimension.
    /// @throws `{ code: "ModelMismatch" }` — the vector index was built with a different embedder model.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn search(
        &self,
        query: String,
        options: Option<SearchOptions>,
    ) -> napi::Result<AsyncTask<SearchTask>> {
        let mode_str = options.as_ref().and_then(|o| o.mode.as_deref());
        let (mode, pre_error) = match SearchMode::from_optional_str(mode_str) {
            Ok(m) => (m, None),
            Err(coded) => (SearchMode::Auto, Some(coded)),
        };

        #[allow(clippy::cast_possible_truncation)]
        let limit = options.as_ref().and_then(|o| o.limit).unwrap_or(10) as usize;
        #[allow(clippy::cast_possible_truncation)]
        let fetch_multiplier = options
            .as_ref()
            .and_then(|o| o.fetch_multiplier)
            .unwrap_or(3) as usize;
        #[allow(clippy::cast_possible_truncation)]
        let rrf_k = options.and_then(|o| o.rrf_k).unwrap_or(60) as usize;

        Ok(AsyncTask::new(SearchTask {
            store: self.inner.clone(),
            store_path: self.path.clone(),
            query,
            mode,
            limit,
            fetch_multiplier,
            rrf_k,
            pre_error,
            failed: None,
        }))
    }

    /// Retrieve a structured context for the given query.
    ///
    /// Runs hybrid search, fetches the full Item for each hit, drops blocks
    /// below `minScore`, and returns the resulting `RetrievedContext`. Pass
    /// the result to one of `adapters.{plain,claude,openai,gemini}.format()`
    /// for prompt-ready output.
    ///
    /// @param query The natural-language query string. Must be non-empty and
    ///   not purely whitespace; rejects immediately with `EmptyQuery` otherwise.
    /// @param options Optional `{ mode?, limit?, fetchMultiplier?, rrfK?, minScore? }`.
    /// @returns `RetrievedContext` with `query` echoed and `blocks` populated.
    /// @throws `{ code: "EmptyQuery" }` — the query is empty or whitespace-only.
    /// @throws `{ code: "NoIndexes" }` — `mode: "auto"` but no sidecar directories exist.
    /// @throws `{ code: "HybridMissingIndex" }` — `mode: "hybrid"` and one sidecar is absent.
    /// @throws `{ code: "IndexMissing" }` — explicit mode requires a sidecar that is absent.
    /// @throws `{ code: "IndexCorrupted" }` — a sidecar directory exists but cannot be opened.
    /// @throws `{ code: "Validation" }` — the `mode` string is unrecognised.
    /// @throws `{ code: "QueryParse" }` — Tantivy could not parse the query string.
    /// @throws `{ code: "Tantivy" }` — Tantivy runtime error during search.
    /// @throws `{ code: "Usearch" }` — `USearch` runtime error during search.
    /// @throws `{ code: "Embedding" }` — embedder runtime error while encoding the query.
    /// @throws `{ code: "ModelDownload" }` — fastembed model files could not be downloaded.
    /// @throws `{ code: "InvalidModelFiles" }` — embedder model files on disk are malformed.
    /// @throws `{ code: "DimMismatch" }` — vector index built with a different embedding dimension.
    /// @throws `{ code: "ModelMismatch" }` — vector index built with a different embedder model.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn retrieve(
        &self,
        query: String,
        options: Option<RetrieveOptions>,
    ) -> napi::Result<AsyncTask<RetrieveTask>> {
        let mode_str = options.as_ref().and_then(|o| o.mode.as_deref());
        let (mode, pre_error) = match SearchMode::from_optional_str(mode_str) {
            Ok(m) => (m, None),
            Err(coded) => (SearchMode::Auto, Some(coded)),
        };

        #[allow(clippy::cast_possible_truncation)]
        let limit = options.as_ref().and_then(|o| o.limit).unwrap_or(10) as usize;
        #[allow(clippy::cast_possible_truncation)]
        let fetch_multiplier = options
            .as_ref()
            .and_then(|o| o.fetch_multiplier)
            .unwrap_or(3) as usize;
        #[allow(clippy::cast_possible_truncation)]
        let rrf_k = options.as_ref().and_then(|o| o.rrf_k).unwrap_or(60) as usize;
        #[allow(clippy::cast_possible_truncation)]
        let min_score = options.and_then(|o| o.min_score).unwrap_or(0.0) as f32;

        Ok(AsyncTask::new(RetrieveTask {
            store: self.inner.clone(),
            store_path: self.path.clone(),
            query,
            mode,
            limit,
            fetch_multiplier,
            rrf_k,
            min_score,
            pre_error,
            failed: None,
        }))
    }

    /// Persist a new item to the store.
    ///
    /// If Tantivy + `USearch` sidecars exist at the store path, the new item
    /// is written to those indexes too (via per-call hook attachment). Hook
    /// write failures during ingest are logged via `tracing::warn!` but do
    /// NOT roll back the `SQLite` insert — the returned Item is durable
    /// regardless of hook outcomes (Principle VII).
    ///
    /// @param item The item to persist. Only `content` is required; other
    ///   fields apply sensible defaults when omitted.
    /// @returns The newly-persisted Item with assigned `id` and `createdAt`.
    /// @throws Error with `.code === "Validation"` if any field fails validation.
    /// @throws Error with `.code === "InvalidId"` if `supersedes` is malformed.
    /// @throws Error with `.code === "SupersedesNotFound"` if `supersedes`
    ///   references a non-existent ID.
    /// @throws Error with `.code === "ReadOnly"` if the store was opened with
    ///   `{ readOnly: true }`.
    /// @throws Error with `.code === "Sqlite"` on database errors.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn ingest(&self, item: crate::types::NewItem) -> napi::Result<AsyncTask<IngestTask>> {
        // Synchronously validate supersedes ULID format. Other validation
        // happens inside the core's ingest (deferred to the libuv thread).
        match crate::types::js_new_item_to_core(item) {
            Ok(new_item) => Ok(AsyncTask::new(IngestTask {
                store: self.inner.clone(),
                store_path: self.path.clone(),
                read_only: self.read_only,
                new_item: Some(new_item),
                pre_error: None,
                failed: None,
            })),
            Err(coded) => Ok(AsyncTask::new(IngestTask {
                store: self.inner.clone(),
                store_path: self.path.clone(),
                read_only: self.read_only,
                new_item: None,
                pre_error: Some(coded),
                failed: None,
            })),
        }
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
