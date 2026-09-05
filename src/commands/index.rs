//! Tantivy sidecar lifecycle: opening indexes with lock-retry, deriving
//! sidecar paths from the store path, resolving `--mode auto` against
//! whichever sidecars exist on disk, and rebuilding a schema-stale sidecar.

use std::path::{Path, PathBuf};

use singularmem_core::Store;

use crate::commands::SearchMode;
use crate::CliError;

/// Attempts and base backoff used when opening the Tantivy sidecar for a
/// write. Tantivy allows one writer per directory, so two hooks firing at
/// once (a `Stop` in two editor windows, say) contend for
/// `.tantivy-writer.lock`. The schedule sleeps `50, 100, 200, 400` ms
/// between five attempts — comfortably longer than a hook's own indexing
/// pass — before giving up.
const INDEX_LOCK_ATTEMPTS: u32 = 5;
const INDEX_LOCK_BASE_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

/// Whether `e` is Tantivy's writer-lockfile contention, as opposed to a
/// corrupt or schema-mismatched sidecar. Matched on the rendered message,
/// lowercased, against Tantivy's actual lock-failure wording (`"lockfile"`
/// or `"failed to acquire lock"`) because the lock failure arrives wrapped
/// in `tantivy::TantivyError` with no dedicated variant to match on. A bare
/// `contains("lock")` is too broad — it misfires on paths like
/// `~/dev/blockchain` appearing in an unrelated error message.
fn is_index_lock_error(e: &singularmem_search::Error) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("lockfile") || msg.contains("failed to acquire lock")
}

/// Open the Tantivy sidecar at `path`, retrying a writer-lock conflict with
/// bounded exponential backoff.
///
/// Makes at most `attempts` opens, sleeping `base_delay`, `2 × base_delay`,
/// … between them (so `attempts - 1` sleeps; no sleep follows the final
/// attempt, which would only add latency to the failure). Only a lock error
/// is retried — see [`is_index_lock_error`]; anything else fails fast, since
/// a corrupt sidecar will not fix itself by waiting.
///
/// # Errors
/// The last error returned by `Index::open`.
fn open_index_with_retry(
    path: &Path,
    attempts: u32,
    base_delay: std::time::Duration,
) -> Result<singularmem_search::Index, singularmem_search::Error> {
    let attempts = attempts.max(1);
    let mut delay = base_delay;
    let mut last: Option<singularmem_search::Error> = None;
    for attempt in 1..=attempts {
        match singularmem_search::Index::open(path) {
            Ok(idx) => return Ok(idx),
            Err(e) => {
                if !is_index_lock_error(&e) {
                    return Err(e);
                }
                tracing::debug!(
                    error = %e,
                    attempt,
                    attempts,
                    path = %path.display(),
                    "Tantivy writer lock is held; retrying"
                );
                last = Some(e);
                if attempt < attempts {
                    std::thread::sleep(delay);
                    delay = delay.saturating_mul(2);
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| unreachable!("at least one attempt always runs")))
}

/// Wire up the Tantivy (and, opt-in, vector) `IndexHook`s on `store` so live
/// writes populate the search sidecars. A no-op when `no_index` is set.
///
/// Shared by the ingest verbs (`run`) and `cmd_hook_entry`'s save-event path
/// — `session-start` is read-only and never needs this.
pub fn wire_index_hooks(store: &mut Store, store_path: &Path, no_index: bool) {
    if no_index {
        return;
    }

    let mut hooks: Vec<Box<dyn singularmem_core::IndexHook>> = Vec::new();

    // Tantivy lexical-search hook (sub-project 2a behaviour — always attempt).
    let index_path = derive_index_path(store_path);
    match open_index_with_retry(&index_path, INDEX_LOCK_ATTEMPTS, INDEX_LOCK_BASE_DELAY) {
        Ok(idx) => hooks.push(Box::new(idx)),
        Err(e) => tracing::warn!(
            error = %e,
            path = %index_path.display(),
            "could not open Tantivy index; lexical search will not work until reindex"
        ),
    }

    // Embedder / vector hook — opt-in: only when .vectors/ already exists.
    let vectors_path = derive_vectors_path(store_path);
    if vectors_path.exists() {
        let embedder: Option<Box<dyn singularmem_search::Embedder>> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Some(Box::new(
                    singularmem_search::testing::MockEmbedder::default(),
                )),
                _ => match singularmem_search::FastembedEmbedder::new() {
                    Ok(e) => Some(Box::new(e)),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "embedder construction failed; semantic search will not work"
                        );
                        None
                    }
                },
            };
        if let Some(embedder) = embedder {
            match singularmem_search::EmbedderIndex::open(&vectors_path, embedder) {
                Ok(idx) => hooks.push(Box::new(idx)),
                Err(e) => tracing::warn!(
                    error = %e,
                    "vector index open failed; semantic search will not work"
                ),
            }
        }
    }

    if !hooks.is_empty() {
        store.set_hook(Some(Box::new(singularmem_core::hook::MultiHook::new(
            hooks,
        ))));
    }
}

pub fn derive_index_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".tantivy");
    PathBuf::from(s)
}

pub fn derive_vectors_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    PathBuf::from(s)
}

/// Result of resolving a `SearchMode` for a given store path. Returned by
/// `resolve_search_mode`.
pub struct ResolvedSearchMode {
    /// The concrete search mode (never `Auto` after resolution).
    pub mode: SearchMode,
    /// Tantivy sidecar path.
    pub tantivy_path: PathBuf,
    /// Vectors sidecar path.
    pub vectors_path: PathBuf,
}

/// Probe the store's sidecar directories and resolve `requested_mode`
/// (which may be `Auto`) into a concrete mode (`Lexical`, `Semantic`,
/// or `Hybrid`). Surfaces the same set of errors `cmd_search` does:
/// `NoIndexes` for auto + neither sidecar, `HybridMissingIndex` for
/// explicit hybrid + one missing, `IndexMissing` for explicit
/// lexical/semantic + that sidecar missing.
pub fn resolve_search_mode(
    store_path: &Path,
    requested_mode: SearchMode,
) -> Result<ResolvedSearchMode, CliError> {
    let tantivy_path = derive_index_path(store_path);
    let vectors_path = derive_vectors_path(store_path);
    let has_lexical = tantivy_path.exists();
    let has_vectors = vectors_path.exists();

    // Resolve --mode auto → concrete mode (or NoIndexes error).
    let resolved = match requested_mode {
        SearchMode::Auto => match (has_lexical, has_vectors) {
            (true, true) => SearchMode::Hybrid,
            (true, false) => {
                tracing::info!(
                    path = %vectors_path.display(),
                    "no vector index; using lexical-only search"
                );
                SearchMode::Lexical
            }
            (false, true) => {
                tracing::info!(
                    path = %tantivy_path.display(),
                    "no lexical index; using semantic-only search"
                );
                SearchMode::Semantic
            }
            (false, false) => return Err(CliError::Search(singularmem_search::Error::NoIndexes)),
        },
        m => m,
    };

    // Explicit-mode pre-flight checks (Auto bypassed via the degradation above).
    match resolved {
        SearchMode::Hybrid => {
            if !has_lexical {
                return Err(CliError::Search(
                    singularmem_search::Error::HybridMissingIndex {
                        missing: "lexical",
                        path: tantivy_path,
                    },
                ));
            }
            if !has_vectors {
                return Err(CliError::Search(
                    singularmem_search::Error::HybridMissingIndex {
                        missing: "semantic",
                        path: vectors_path,
                    },
                ));
            }
        }
        SearchMode::Lexical if !has_lexical => {
            return Err(CliError::Search(singularmem_search::Error::IndexMissing {
                path: tantivy_path,
            }));
        }
        SearchMode::Semantic if !has_vectors => {
            return Err(CliError::Search(singularmem_search::Error::IndexMissing {
                path: vectors_path,
            }));
        }
        _ => {}
    }

    Ok(ResolvedSearchMode {
        mode: resolved,
        tantivy_path,
        vectors_path,
    })
}

/// Open the Tantivy sidecar at `index_path` for reindexing, recreating it
/// from scratch when it was built with an older schema.
pub fn open_or_rebuild_index(index_path: &Path) -> Result<singularmem_search::Index, CliError> {
    use singularmem_search::Index;

    match Index::open(index_path) {
        Ok(index) => Ok(index),
        Err(singularmem_search::Error::IndexSchemaMismatch { .. }) => {
            // Destructive action: always announce it, even with --quiet.
            eprintln!("rebuilding Tantivy sidecar with the current schema");
            std::fs::remove_dir_all(index_path).map_err(|e| {
                CliError::IndexOpen(format!(
                    "removing stale sidecar {}: {e}",
                    index_path.display()
                ))
            })?;
            Index::open(index_path).map_err(|e| CliError::IndexOpen(e.to_string()))
        }
        Err(e) => Err(CliError::IndexOpen(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_index_lock_error, open_index_with_retry};

    /// Tantivy allows a single writer per directory, so a second
    /// `Index::open` while one is live fails with "Failed to acquire
    /// Lockfile". `open_index_with_retry` must recognise that as transient
    /// contention, back off, and still surface the error once the attempts
    /// run out — then succeed as soon as the first writer is dropped.
    #[test]
    fn open_index_with_retry_backs_off_on_a_held_writer_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("index");
        let held = singularmem_search::Index::open(&path).expect("first open takes the lock");

        let base = std::time::Duration::from_millis(10);
        let start = std::time::Instant::now();
        let err = open_index_with_retry(&path, 2, base)
            .err()
            .expect("the writer lock is held, so every attempt must fail");
        let elapsed = start.elapsed();
        assert!(
            is_index_lock_error(&err),
            "expected a lockfile error, got: {err}"
        );
        assert!(
            elapsed >= base,
            "two attempts must sleep at least one base delay, slept {elapsed:?}"
        );

        drop(held);
        open_index_with_retry(&path, 2, base).expect("the lock is free again");
    }

    /// A non-lock failure (here: a sidecar path that is a regular file, so
    /// the directory cannot be created) must fail fast rather than burn the
    /// whole backoff schedule — waiting cannot fix it.
    #[test]
    fn open_index_with_retry_does_not_retry_non_lock_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("not-a-dir");
        std::fs::write(&path, b"regular file").unwrap();

        let start = std::time::Instant::now();
        let err = open_index_with_retry(&path, 5, std::time::Duration::from_millis(200))
            .err()
            .expect("a file where the sidecar directory should be cannot open");
        assert!(!is_index_lock_error(&err), "not a lock error: {err}");
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "must not have slept"
        );
    }

    /// A bare `contains("lock")` would misfire on an unrelated error whose
    /// message happens to mention a path like `~/dev/blockchain`. Matching
    /// must require Tantivy's actual lock-failure wording.
    #[test]
    fn is_index_lock_error_does_not_misfire_on_unrelated_lock_substring() {
        let err = singularmem_search::Error::Tantivy {
            context: "opening index",
            source: tantivy::TantivyError::SystemError(
                "no such directory: ~/dev/blockchain".to_string(),
            ),
        };
        assert!(!is_index_lock_error(&err), "not a lock error: {err}");
    }

    /// Tantivy's real wording ("Failed to acquire Lockfile") must still be
    /// recognised regardless of case.
    #[test]
    fn is_index_lock_error_recognises_tantivys_actual_wording() {
        let err = singularmem_search::Error::Tantivy {
            context: "opening index",
            source: tantivy::TantivyError::SystemError(
                "Failed to acquire Lockfile: try again".to_string(),
            ),
        };
        assert!(is_index_lock_error(&err), "expected a lock error: {err}");
    }
}
