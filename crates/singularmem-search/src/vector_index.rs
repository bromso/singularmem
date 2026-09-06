//! `VectorIndex` — wraps `usearch::Index` with a sidecar `.meta.json` carrying
//! model + dimensionality + HNSW params. The on-disk layout is:
//!
//! ```text
//! <dir>/
//!   .meta.json      — VectorIndexMeta (serde_json), format_version "2"
//!   index.usearch   — USearch binary, rewritten only on compaction
//!   keymap.bin      — Keymap (bincode; u64 key ↔ ItemId), written with it
//!   journal.bin     — append-only vectors since the last compaction
//!   lock            — advisory lock file held across a commit
//! ```
//!
//! Tasks 5-7 of the search-v0-embeddings plan implement open + add/remove/save +
//! search respectively. Sub-project 17 adds the `journal.bin` write path: an
//! [`add`](VectorIndex::add) updates the in-memory graph immediately and queues
//! the vector; [`commit`](VectorIndex::commit) appends the queue to the journal
//! and only rewrites `index.usearch` when the journal outgrows
//! [`COMPACT_THRESHOLD`] or the commit ends a bulk batch. See
//! `docs/superpowers/specs/2026-09-06-ingest-throughput-17-design.md` § "Part 2".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use fs4::FileExt;
use serde::{Deserialize, Serialize};
use singularmem_core::ItemId;
use usearch::{IndexOptions, MetricKind, ScalarKind};

use crate::embedder::Embedder;
use crate::error::{Error, Result};
use crate::fsync::{sync_dir, sync_file};
use crate::vector_journal::Journal;

/// Items per `Embedder::embed_batch` call in [`EmbedderIndex::on_ingest_batch`].
///
/// 64 sits on the flat part of the measured throughput curve for the bundled
/// ONNX models; larger chunks buy nothing and cost memory.
pub const EMBED_CHUNK: usize = 64;

/// Journal records tolerated before [`VectorIndex::commit`] compacts, i.e.
/// rewrites `index.usearch` + `keymap.bin` and truncates `journal.bin`.
///
/// A record is `16 + 4 × dim` bytes, so 1 000 records of a 384-dimensional
/// model is ~1.5 MB of journal — cheap to replay on open, and far cheaper to
/// append to than re-serialising the whole HNSW graph per item.
pub const COMPACT_THRESHOLD: usize = 1_000;

/// Maximum records handed to one `Journal::append` call when draining the
/// pending queue. `append` buffers its whole argument before writing, so a
/// bulk drain of a large queue would otherwise materialise the entire corpus
/// as one contiguous byte buffer on top of the queue itself.
const JOURNAL_APPEND_CHUNK: usize = 1_024;

/// Attempts and base backoff used when taking the commit lock on
/// `<dir>/lock`. Mirrors the Tantivy sidecar's schedule in
/// `src/commands/index.rs`: five attempts sleeping `50, 100, 200, 400` ms.
const LOCK_ATTEMPTS: u32 = 5;
const LOCK_BASE_DELAY: Duration = Duration::from_millis(50);

/// On-disk `format_version` written by this build.
const FORMAT_VERSION: &str = "2";
/// The pre-journal `format_version`, upgraded in place on the first commit.
const FORMAT_VERSION_V1: &str = "1";

// ── VectorIndexOptions ────────────────────────────────────────────────────

/// HNSW tuning parameters for [`VectorIndex::open_with_options`].
///
/// These are written into `.meta.json` on the first open and cannot be changed
/// afterwards without rebuilding the index.
#[derive(Debug, Clone, Copy)]
pub struct VectorIndexOptions {
    /// `M` parameter: number of bi-directional links per graph node. Higher
    /// values improve recall at the cost of memory. Typical range: 8–64.
    pub hnsw_m: usize,
    /// `ef_construction`: dynamic candidate list size during graph construction.
    /// Higher values improve build quality but slow down indexing. Typical: 64–512.
    pub hnsw_ef_construction: usize,
    /// `ef_search`: dynamic candidate list during search. Larger → more recall,
    /// slower queries. Can be changed per-query without rebuilding.
    pub expansion_search: usize,
}

impl Default for VectorIndexOptions {
    fn default() -> Self {
        Self {
            hnsw_m: 16,
            hnsw_ef_construction: 128,
            expansion_search: 64,
        }
    }
}

// ── VectorIndexMeta ───────────────────────────────────────────────────────

/// Metadata persisted alongside the `USearch` binary in `.meta.json`.
///
/// This is the source of truth for validating that a loaded index matches the
/// current [`Embedder`]. If `model_id` or `dim` diverges from the embedder,
/// open returns [`Error::ModelMismatch`] or [`Error::DimMismatch`] respectively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorIndexMeta {
    /// Monotonic format version. `"1"` was the pre-journal layout; this build
    /// creates and upgrades to `"2"` (see [`COMPACT_THRESHOLD`]).
    pub format_version: String,
    /// Stable model identifier from [`Embedder::model_id()`].
    pub model_id: String,
    /// Embedding dimension; must match [`Embedder::dim()`].
    pub dim: usize,
    /// Distance function name. Always `"cosine"` for v0.3.0.
    pub distance: String,
    /// HNSW `M` connectivity parameter.
    pub hnsw_m: usize,
    /// HNSW `ef_construction` parameter.
    pub hnsw_ef_construction: usize,
    /// Wall-clock timestamp of first open.
    pub created_at: jiff::Timestamp,
}

// ── Keymap ────────────────────────────────────────────────────────────────

/// Bidirectional mapping between sequential `u64` `USearch` keys and [`ItemId`]s.
///
/// `USearch` requires `u64` integer keys. We assign them sequentially and record
/// the forward (`u64 → ItemId`) and reverse (`ItemId → u64`) mappings here.
/// The keymap is persisted as `keymap.bin` (bincode) alongside the `USearch`
/// binary so that keys survive process restarts.
///
/// # On-disk layout (`format_version "2"`)
///
/// bincode, fields in declaration order: `generation`, `next_key`, `forward`,
/// `reverse`. bincode is not self-describing and cannot default a missing
/// field, so the `"1"` layout — which had no `generation` — is read through
/// [`KeymapV1`] and selected by `.meta.json`'s `format_version`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Keymap {
    /// Bumped by one on every successful compaction. A handle records the
    /// generation it loaded; when the on-disk keymap has moved on, another
    /// handle has compacted and this handle's in-memory graph is stale.
    pub generation: u64,
    /// Next free sequential key. Monotonically increasing; never reused.
    pub next_key: u64,
    /// Forward direction: `USearch` key → `ItemId`. `BTreeMap` so iteration is
    /// deterministically ordered.
    pub forward: BTreeMap<u64, ItemId>,
    /// Reverse direction: `ItemId` → `USearch` key. `HashMap` because `ItemId`
    /// is `Hash + Eq` but not `Ord` (`Ulid` is `Ord`, but `ItemId` does not
    /// derive it).
    pub reverse: HashMap<ItemId, u64>,
}

/// `keymap.bin` as written by `format_version "1"` builds: the same fields
/// minus `generation`. Read-only — this build never writes it back.
#[derive(Debug, Deserialize)]
struct KeymapV1 {
    next_key: u64,
    forward: BTreeMap<u64, ItemId>,
    reverse: HashMap<ItemId, u64>,
}

impl From<KeymapV1> for Keymap {
    fn from(v1: KeymapV1) -> Self {
        Self {
            generation: 0,
            next_key: v1.next_key,
            forward: v1.forward,
            reverse: v1.reverse,
        }
    }
}

/// Deserialise `keymap.bin`, picking the struct that matches `format_version`.
///
/// The other layout is tried as a fallback: a crash between the `.meta.json`
/// upgrade and the first v2 keymap rename leaves a `"2"` meta beside a v1
/// keymap, and that directory must still open.
fn read_keymap(path: &Path, format_version: &str) -> Result<Keymap> {
    let bytes = fs::read(path).map_err(Error::Io)?;
    let corrupt = |e: bincode::Error| Error::Embedding {
        context: "deserializing keymap.bin",
        reason: format!("{e}"),
    };
    if format_version == FORMAT_VERSION_V1 {
        bincode::deserialize::<KeymapV1>(&bytes)
            .map(Keymap::from)
            .or_else(|e| bincode::deserialize::<Keymap>(&bytes).map_err(|_| corrupt(e)))
    } else {
        bincode::deserialize::<Keymap>(&bytes).or_else(|e| {
            bincode::deserialize::<KeymapV1>(&bytes)
                .map(Keymap::from)
                .map_err(|_| corrupt(e))
        })
    }
}

/// Just the `format_version` of `.meta.json`, without parsing (or requiring)
/// the rest of the document. `None` when the file does not exist.
fn disk_format_version(meta_path: &Path) -> Result<Option<String>> {
    #[derive(Deserialize)]
    struct Probe {
        format_version: String,
    }
    match fs::read_to_string(meta_path) {
        Ok(text) => serde_json::from_str::<Probe>(&text)
            .map(|p| Some(p.format_version))
            .map_err(|e| Error::Embedding {
                context: "parsing existing .meta.json",
                reason: format!("{e}"),
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

// ── VectorIndex ───────────────────────────────────────────────────────────

/// `USearch`-backed approximate nearest-neighbour (ANN) index with model-identity
/// checking and bidirectional `ItemId` ↔ `u64` key mapping.
///
/// # Thread safety
///
/// Both the inner `usearch::Index` and the keymap are guarded by [`Mutex`].
/// Multiple threads can call [`add`](VectorIndex::add),
/// [`remove`](VectorIndex::remove), and [`search`](VectorIndex::search)
/// concurrently; each operation acquires its own lock.
/// [`commit`](VectorIndex::commit) additionally takes an advisory *file* lock
/// on `<dir>/lock`, so handles in different processes (or two handles in this
/// one) serialise their journal appends and compactions against each other.
pub struct VectorIndex {
    pub(crate) inner: Mutex<usearch::Index>,
    pub(crate) path: PathBuf,
    pub(crate) usearch_path: PathBuf,
    pub(crate) meta: VectorIndexMeta,
    pub(crate) keymap: Mutex<Keymap>,
    /// `<dir>/keymap.bin`.
    keymap_path: PathBuf,
    /// `<dir>/.meta.json`.
    meta_path: PathBuf,
    /// `expansion_search` this handle was opened with. Kept so a stale-handle
    /// reload can rebuild the `usearch::Index` with the same options.
    expansion_search: usize,
    /// Append-only log of vectors added since the last compaction.
    journal: Journal,
    /// Vectors added but not yet appended to `journal`, in insertion order.
    pending: Mutex<Vec<(ItemId, Vec<f32>)>>,
    /// `<dir>/lock`, the advisory lock file taken for the whole of a commit.
    lock_path: PathBuf,
    /// `keymap.generation` this handle's in-memory graph corresponds to. When
    /// the on-disk keymap's generation has moved past this, another handle
    /// compacted and this handle must reload before it may save.
    loaded_generation: AtomicU64,
    /// Set when `.meta.json` on disk still says `format_version "1"`; the
    /// upgraded meta is written by the first commit, inside the lock.
    meta_upgrade_pending: AtomicBool,
    /// Ids removed since the last compaction. `journal.bin` has no tombstone
    /// record, so a removed id whose vector is still in the journal would be
    /// added straight back by the replay in `absorb_journal`. Holding the id
    /// here until the compaction that truncates the journal keeps the removal
    /// stable. In-memory only, exactly like the pre-journal behaviour where a
    /// `remove` was lost unless it was followed by a save.
    tombstones: Mutex<HashSet<ItemId>>,
}

/// Holds `<dir>/lock` for the duration of a commit or of a load. Releasing on
/// drop means an early `?` return inside a commit cannot strand the lock.
struct CommitLock(File);

impl Drop for CommitLock {
    fn drop(&mut self) {
        // Nothing useful to do on failure — the OS releases the flock when the
        // descriptor closes a moment later anyway.
        let _ = FileExt::unlock(&self.0);
    }
}

/// Which advisory lock mode [`acquire_lock_at`] should take. Writers need the
/// exclusive lock; a load only needs to exclude writers, so several readers
/// can share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockMode {
    Shared,
    Exclusive,
}

/// Open `<dir>/lock` without truncating it. `File::create` would truncate,
/// and on Windows `LockFileEx` then fails with a sharing violation that the
/// retry loop cannot tell apart from a hard error.
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Windows `ERROR_LOCK_VIOLATION`. `LockFileEx` reports a lock already held
/// by another process with this raw code, and Rust's `io::Error` has no
/// `ErrorKind` for it — it surfaces as `ErrorKind::Uncategorized`, which is
/// unstable and cannot be matched on. So the raw code is matched instead.
#[cfg(windows)]
const ERROR_LOCK_VIOLATION: i32 = 33;

/// `true` if `e` means "someone else holds this lock", the one condition
/// [`acquire_lock_at`] retries.
///
/// Unix `flock`/`fcntl` report contention as `EWOULDBLOCK`, which maps to
/// [`std::io::ErrorKind::WouldBlock`]. Windows `LockFileEx` reports it as raw
/// OS error 33 (`ERROR_LOCK_VIOLATION`), which maps to the unstable
/// `Uncategorized` kind — treating that as a hard error made every genuinely
/// contended commit on Windows fail immediately instead of backing off.
fn is_lock_contention(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        return e.raw_os_error() == Some(ERROR_LOCK_VIOLATION);
    }
    #[cfg(not(windows))]
    false
}

/// Take `<dir>/lock` in `mode`, retrying a busy lock on the schedule the
/// Tantivy sidecar uses: five attempts sleeping `50, 100, 200, 400` ms.
///
/// "Busy" is whatever [`is_lock_contention`] recognises — `WouldBlock`
/// everywhere, plus Windows' raw `ERROR_LOCK_VIOLATION` (33). Any other io
/// error is returned immediately as [`Error::Io`]; exhausting the five
/// attempts returns [`Error::Usearch`] with the context
/// `"acquiring vector index lock"`.
fn acquire_lock_at(lock_path: &Path, mode: LockMode) -> Result<CommitLock> {
    let mut delay = LOCK_BASE_DELAY;
    for attempt in 1..=LOCK_ATTEMPTS {
        let file = open_lock_file(lock_path).map_err(Error::Io)?;
        // UFCS: `File` grew inherent `try_lock_shared` in Rust 1.89, which
        // returns a different error type. We want fs4's `FileExt` on both
        // arms, so both stay on the `io::ErrorKind::WouldBlock` contract the
        // retry loop below is written against.
        let taken = match mode {
            LockMode::Shared => FileExt::try_lock_shared(&file),
            LockMode::Exclusive => FileExt::try_lock_exclusive(&file),
        };
        match taken {
            Ok(()) => return Ok(CommitLock(file)),
            Err(e) if is_lock_contention(&e) => {
                tracing::debug!(
                    path = %lock_path.display(),
                    ?mode,
                    attempt,
                    attempts = LOCK_ATTEMPTS,
                    "vector index lock busy; retrying",
                );
                if attempt < LOCK_ATTEMPTS {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Err(Error::Usearch {
        context: "acquiring vector index lock",
        reason: format!("busy after {LOCK_ATTEMPTS} attempts"),
    })
}

/// Take the shared lock for a load. A directory we may not write to has no
/// lock file we can create; loading it unsynchronised is the pre-lock
/// behaviour and is the only way a read-only vector directory can be opened
/// at all, so a permission failure downgrades to "no lock" rather than
/// failing the open.
fn acquire_load_lock(lock_path: &Path) -> Result<Option<CommitLock>> {
    match acquire_lock_at(lock_path, LockMode::Shared) {
        Ok(guard) => Ok(Some(guard)),
        Err(Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            tracing::debug!(
                path = %lock_path.display(),
                "vector index lock file is not writable; loading without the lock",
            );
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Delete leftover `*.tmp` files from a crashed compaction. Best effort: the
/// caller holds the shared load lock, so no compaction is in flight, but a
/// read-only directory simply keeps its litter.
fn sweep_temp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "tmp") {
            if let Err(e) = fs::remove_file(&path) {
                tracing::debug!(path = %path.display(), error = %e, "could not sweep stale temp file");
            }
        }
    }
}

/// `true` for the errors a torn compaction produces, i.e. the ones worth
/// retrying the load for once the lock has been re-taken.
fn is_retryable_load_error(e: &Error) -> bool {
    matches!(
        e,
        Error::Usearch { .. }
            | Error::Embedding {
                context: "deserializing keymap.bin",
                ..
            }
    )
}

impl std::fmt::Debug for VectorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorIndex")
            .field("path", &self.path)
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

impl VectorIndex {
    /// Open (or create) a vector index at `dir` using the given [`Embedder`].
    ///
    /// On a fresh directory: creates `.meta.json`, initialises an in-memory
    /// `USearch` graph, and reserves 1024 slots.
    ///
    /// On an existing directory: reads `.meta.json`, verifies that
    /// `model_id` matches the embedder, and loads `index.usearch` + `keymap.bin`
    /// if present.
    ///
    /// # Errors
    ///
    /// - [`Error::ModelMismatch`] if the persisted `model_id` ≠ `embedder.model_id()`.
    /// - [`Error::DimMismatch`] if the persisted `dim` ≠ `embedder.dim()`.
    /// - [`Error::Usearch`] on `USearch` initialisation or load failure.
    /// - [`Error::Io`] on filesystem errors.
    pub fn open(dir: impl AsRef<Path>, embedder: &dyn Embedder) -> Result<Self> {
        Self::open_with_options(dir, embedder, VectorIndexOptions::default())
    }

    /// Like [`VectorIndex::open`] but with explicit HNSW tuning parameters.
    ///
    /// `options` are written into `.meta.json` on the first open; subsequent
    /// opens ignore `options` (persisted values are used instead).
    ///
    /// # Errors
    ///
    /// Same as [`VectorIndex::open`].
    ///
    /// # Panics
    ///
    /// Panics if `dir` contains non-UTF-8 bytes (required by the `USearch` C FFI
    /// for the index file path).
    pub fn open_with_options(
        dir: impl AsRef<Path>,
        embedder: &dyn Embedder,
        options: VectorIndexOptions,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir).map_err(Error::Io)?;
        let lock_path = dir.join("lock");

        // The whole load runs under the SHARED advisory lock, so a concurrent
        // compaction's two renames can never be observed half-done: a reader
        // would otherwise see a new `index.usearch` beside an old `keymap.bin`
        // (replay then collides on keys the graph already holds) or the
        // reverse (vectors silently invisible). Several readers may hold it at
        // once; only a commit excludes them.
        for attempt in 0..2 {
            let _guard = acquire_load_lock(&lock_path)?;
            sweep_temp_files(dir);
            match Self::load(dir, embedder, options) {
                Ok(index) => return Ok(index),
                // Retry once: a load that raced a compaction started before we
                // held the lock can still see a half-written file.
                Err(e) if attempt == 0 && is_retryable_load_error(&e) => {
                    tracing::debug!(
                        path = %dir.display(),
                        error = %e,
                        "vector index load failed; retrying once under a fresh lock",
                    );
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("the second loop iteration either returns Ok or propagates the error")
    }

    /// Read `.meta.json`, or build the meta a fresh directory should have.
    ///
    /// Returns the meta this handle will use — always `format_version "2"` —
    /// alongside the version actually on disk, which selects the `keymap.bin`
    /// layout and says whether an upgrade is outstanding. `None` for a fresh
    /// directory, whose `.meta.json` this function has just written.
    fn load_meta(
        dir: &Path,
        meta_path: &Path,
        embedder: &dyn Embedder,
        options: VectorIndexOptions,
    ) -> Result<(VectorIndexMeta, Option<String>)> {
        if !meta_path.exists() {
            let meta = VectorIndexMeta {
                format_version: FORMAT_VERSION.to_string(),
                model_id: embedder.model_id().to_string(),
                dim: embedder.dim(),
                distance: "cosine".to_string(),
                hnsw_m: options.hnsw_m,
                hnsw_ef_construction: options.hnsw_ef_construction,
                created_at: jiff::Timestamp::now(),
            };
            let text = serde_json::to_string_pretty(&meta).map_err(|e| Error::Embedding {
                context: "serializing .meta.json",
                reason: format!("{e}"),
            })?;
            fs::write(meta_path, text).map_err(Error::Io)?;
            return Ok((meta, None));
        }

        let text = fs::read_to_string(meta_path).map_err(Error::Io)?;
        let mut meta: VectorIndexMeta =
            serde_json::from_str(&text).map_err(|e| Error::Embedding {
                context: "parsing existing .meta.json",
                reason: format!("{e}"),
            })?;
        // Refuse a directory written by a newer build by name rather than
        // misreading its `keymap.bin` as one of the layouts we know.
        if meta.format_version != FORMAT_VERSION && meta.format_version != FORMAT_VERSION_V1 {
            return Err(Error::IndexCorrupted {
                path: dir.to_path_buf(),
                reason: format!(
                    "unsupported vector index format_version {:?}; \
                     this build reads {FORMAT_VERSION_V1:?} and {FORMAT_VERSION:?}",
                    meta.format_version
                ),
            });
        }
        if meta.model_id != embedder.model_id() {
            return Err(Error::ModelMismatch {
                path: dir.to_path_buf(),
                found_model: meta.model_id,
                expected_model: embedder.model_id().to_string(),
            });
        }
        if meta.dim != embedder.dim() {
            return Err(Error::DimMismatch {
                expected: meta.dim,
                got: embedder.dim(),
            });
        }
        // A pre-journal directory opens unchanged: the in-memory meta is
        // promoted to v2 here and the first commit rewrites `.meta.json` under
        // the commit lock, so a read-only open never touches the file.
        let disk_format = std::mem::replace(&mut meta.format_version, FORMAT_VERSION.to_string());
        Ok((meta, Some(disk_format)))
    }

    /// Load meta, `index.usearch`, `keymap.bin` and the journal into a fresh
    /// handle. The caller holds the shared load lock.
    fn load(dir: &Path, embedder: &dyn Embedder, options: VectorIndexOptions) -> Result<Self> {
        let meta_path = dir.join(".meta.json");
        let usearch_path = dir.join("index.usearch");
        let keymap_path = dir.join("keymap.bin");

        let (meta, disk_format) = Self::load_meta(dir, &meta_path, embedder, options)?;
        let disk_format = disk_format.unwrap_or_else(|| FORMAT_VERSION.to_string());
        let upgrading = disk_format == FORMAT_VERSION_V1;

        // ── Construct usearch::Index ──────────────────────────────────────
        let usearch_opts = IndexOptions {
            dimensions: meta.dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: meta.hnsw_m,
            expansion_add: meta.hnsw_ef_construction,
            expansion_search: options.expansion_search,
            multi: false,
        };
        let inner = usearch::Index::new(&usearch_opts).map_err(|e| Error::Usearch {
            context: "constructing usearch::Index",
            reason: format!("{e}"),
        })?;

        // Load existing data if present; otherwise reserve initial capacity.
        let index_present = usearch_path.exists();
        if index_present {
            inner
                .load(usearch_path.to_str().unwrap())
                .map_err(|e| Error::Usearch {
                    context: "loading existing usearch index",
                    reason: format!("{e}"),
                })?;
        } else {
            inner.reserve(1024).map_err(|e| Error::Usearch {
                context: "reserving initial usearch capacity",
                reason: format!("{e}"),
            })?;
        }

        // ── Load or create keymap ─────────────────────────────────────────
        let mut keymap = if keymap_path.exists() {
            read_keymap(&keymap_path, &disk_format)?
        } else {
            Keymap::default()
        };
        // An absent `index.usearch` beside a non-empty keymap is not a state
        // any successful compaction produces: emptying the index removes the
        // graph file *and* renames an empty keymap into place. Seeing both
        // means the crash landed between those two steps (in either order),
        // so the keymap on disk describes vectors that no longer exist. The
        // keymap is the stale half — the graph file is gone and cannot be
        // recovered from it — so it is reset to empty. `generation` and
        // `next_key` are kept: the generation still identifies this state to
        // other handles, and keys are never reused.
        if !index_present && !keymap.forward.is_empty() {
            tracing::warn!(
                path = %dir.display(),
                stale_entries = keymap.forward.len(),
                generation = keymap.generation,
                "keymap.bin names vectors but index.usearch is absent (crash during an \
                 empty-index compaction); resetting the keymap and replaying the journal",
            );
            keymap.forward.clear();
            keymap.reverse.clear();
        }
        let generation = keymap.generation;

        // ── Open the journal and replay it into the in-memory graph ───────
        // A fully compacted directory has no `journal.bin`, and opening one
        // must not create it (see `Journal::open`).
        let journal = Journal::open(&dir.join("journal.bin"), meta.dim, &meta.model_id)?;
        let index = Self {
            inner: Mutex::new(inner),
            path: dir.to_path_buf(),
            usearch_path,
            meta,
            keymap: Mutex::new(keymap),
            keymap_path,
            meta_path,
            expansion_search: options.expansion_search,
            journal,
            pending: Mutex::new(Vec::new()),
            lock_path: dir.join("lock"),
            loaded_generation: AtomicU64::new(generation),
            meta_upgrade_pending: AtomicBool::new(upgrading),
            tombstones: Mutex::new(HashSet::new()),
        };
        index.absorb_journal()?;
        index.warn_if_graph_exceeds_keymap();
        Ok(index)
    }

    /// Warn when the loaded graph holds more vectors than the keymap can name.
    ///
    /// An end-of-batch commit skips the journal entirely (the compaction it
    /// runs makes the queued vectors durable inside the same locked section),
    /// so a crash between compaction's `index.usearch` rename and its
    /// `keymap.bin` rename leaves a *new* graph beside an *old* keymap with no
    /// journal to recover the difference from. The extra vectors are orphans:
    /// `search` filters its hits through `keymap.forward`, so they can never
    /// be returned, and no later compaction removes them — they are re-saved
    /// into every subsequent `index.usearch`.
    ///
    /// Nothing here can reconstruct the missing ids (`USearch` 2.15 cannot
    /// enumerate a graph's keys, and the ids only ever lived in the keymap
    /// that was lost), so this is a report, not a repair: the searchable
    /// count is the keymap's and that is what
    /// [`doc_count`](VectorIndex::doc_count) returns, while the operator is
    /// told how to rebuild.
    fn warn_if_graph_exceeds_keymap(&self) {
        let graph = self.inner.lock().expect("usearch mutex poisoned").size();
        let named = self
            .keymap
            .lock()
            .expect("keymap mutex poisoned")
            .forward
            .len();
        if graph > named {
            tracing::warn!(
                path = %self.path.display(),
                graph_vectors = graph,
                keymap_entries = named,
                orphans = graph - named,
                "index.usearch holds more vectors than keymap.bin names (torn compaction); \
                 the extra vectors are unsearchable — run `singularmem reindex \
                 --with-embeddings --reset-vectors --force` to rebuild the index",
            );
        }
    }

    /// Add every journal record whose id is not already in the keymap to the
    /// in-memory graph, without re-journalling it. Idempotent: a record whose
    /// id survived a compaction (a crash between the rename and the truncate)
    /// is skipped, so replaying the same journal twice is a no-op the second
    /// time.
    fn absorb_journal(&self) -> Result<()> {
        let records = self.journal.replay()?;
        if records.is_empty() {
            return Ok(());
        }
        let missing: Vec<(ItemId, &[f32])> = {
            let tombstones = self.tombstones.lock().expect("tombstone mutex poisoned");
            let keymap = self.keymap.lock().expect("keymap mutex poisoned");
            let missing = records
                .iter()
                .filter(|(id, _)| !keymap.reverse.contains_key(id) && !tombstones.contains(id))
                .map(|(id, vector)| (*id, vector.as_slice()))
                .collect();
            drop(keymap);
            drop(tombstones);
            missing
        };
        if missing.is_empty() {
            return Ok(());
        }
        self.add_entries(&missing, false)
    }

    /// Returns a reference to the index metadata.
    pub const fn meta(&self) -> &VectorIndexMeta {
        &self.meta
    }

    // ── Mutation operations ───────────────────────────────────────────────

    /// Add (or replace) one item's embedding vector. Assigns a sequential `u64`
    /// key internally and records the `ItemId` ↔ key mapping in the keymap.
    ///
    /// The in-memory graph is current immediately; the vector is queued for
    /// the next [`commit`](VectorIndex::commit), which writes it to
    /// `journal.bin`.
    ///
    /// # Errors
    ///
    /// - [`Error::DimMismatch`] if `vector.len() != meta.dim`.
    /// - [`Error::Usearch`] on `USearch` internal failure.
    ///
    /// # Panics
    ///
    /// Panics if the keymap or inner index mutex is poisoned (only possible if
    /// another thread panicked while holding the lock).
    pub fn add(&self, id: ItemId, vector: &[f32]) -> Result<()> {
        self.add_batch(&[(id, vector)])
    }

    /// Add (or replace) a batch of items' embedding vectors under a single
    /// keymap lock acquisition and a single inner-index lock acquisition.
    ///
    /// Every entry's dimension is validated before either lock is taken: if
    /// any `vector.len() != meta.dim`, the whole batch is rejected and
    /// neither the keymap nor the inner index is touched.
    ///
    /// The in-memory graph is current immediately; the vectors are queued for
    /// the next [`commit`](VectorIndex::commit).
    ///
    /// # Errors
    ///
    /// - [`Error::DimMismatch`] if any `vector.len() != meta.dim`.
    /// - [`Error::Usearch`] on `USearch` internal failure.
    ///
    /// # Panics
    ///
    /// Panics if the keymap or inner index mutex is poisoned (only possible if
    /// another thread panicked while holding the lock).
    pub fn add_batch(&self, entries: &[(ItemId, &[f32])]) -> Result<()> {
        self.add_entries(entries, true)
    }

    /// Shared body of [`add`](VectorIndex::add) /
    /// [`add_batch`](VectorIndex::add_batch) and journal replay.
    ///
    /// `queue` is `false` on the replay path: those vectors are already on
    /// disk, so re-queueing them would write them to the journal a second
    /// time on the next commit.
    fn add_entries(&self, entries: &[(ItemId, &[f32])], queue: bool) -> Result<()> {
        for (_, vector) in entries {
            if vector.len() != self.meta.dim {
                return Err(Error::DimMismatch {
                    expected: self.meta.dim,
                    got: vector.len(),
                });
            }
        }

        // Lock order throughout this type is inner → keymap → pending. The
        // queue is filled while the inner lock is still held so a compaction,
        // which snapshots the graph and takes the queue in one critical
        // section, can never see a vector that is in neither.
        let inner = self.inner.lock().expect("usearch mutex poisoned");
        let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
        let result = insert_entries(&inner, &mut keymap, entries);
        drop(keymap);
        result?;
        if queue {
            let mut pending = self.pending.lock().expect("pending mutex poisoned");
            pending.extend(entries.iter().map(|(id, vector)| (*id, (*vector).to_vec())));
            drop(pending);
        }
        drop(inner);
        Ok(())
    }

    /// Remove an item by [`ItemId`]. If the ID is not present, this is a no-op
    /// (returns `Ok(())`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Usearch`] if `USearch` reports an error during removal.
    ///
    /// # Panics
    ///
    /// Panics if the keymap or inner index mutex is poisoned.
    pub fn remove(&self, id: ItemId) -> Result<()> {
        let key_opt = {
            let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
            let key = keymap.reverse.remove(&id);
            if let Some(k) = key {
                keymap.forward.remove(&k);
            }
            key
        };
        if let Some(key) = key_opt {
            self.inner
                .lock()
                .expect("usearch mutex poisoned")
                .remove(key)
                .map_err(|e| Error::Usearch {
                    context: "usearch remove",
                    reason: format!("{e}"),
                })?;
        }
        // Drop the vector from the not-yet-journalled queue, and remember the
        // removal until the next compaction truncates the journal — otherwise
        // the replay in `absorb_journal` would add it straight back. Only an
        // id that was actually present needs suppressing; tombstoning an
        // absent id would suppress a *later* legitimate add of it.
        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        pending.retain(|(pending_id, _)| *pending_id != id);
        drop(pending);
        if key_opt.is_some() {
            let mut tombstones = self.tombstones.lock().expect("tombstone mutex poisoned");
            tombstones.insert(id);
            drop(tombstones);
        }
        Ok(())
    }

    // ── Durability: journal append + compaction ───────────────────────────

    /// Persist everything added since the last commit.
    ///
    /// Under the `<dir>/lock` advisory lock: rewrite `.meta.json` if this
    /// directory is still v1, append the queued vectors to `journal.bin`,
    /// and — when the journal now holds more than [`COMPACT_THRESHOLD`]
    /// records, or `end_of_batch` says this commit closes a bulk ingest —
    /// compact. A plain single-item commit therefore writes a couple of
    /// kilobytes instead of re-serialising the whole HNSW graph.
    ///
    /// # Errors
    ///
    /// - [`Error::Usearch`] if the commit lock is still busy after five
    ///   attempts, or on a `USearch` serialisation failure.
    /// - [`Error::Io`] on a filesystem failure.
    ///
    /// # Panics
    ///
    /// Panics if the pending, inner index, or keymap mutex is poisoned.
    pub fn commit(&self, end_of_batch: bool) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.commit_locked(end_of_batch)
    }

    /// Rewrite `index.usearch` + `keymap.bin` from the current in-memory state
    /// and truncate `journal.bin`, flushing any queued vectors first.
    ///
    /// Public because `reindex` and the tests drive compaction directly.
    ///
    /// # Errors
    ///
    /// Same as [`commit`](VectorIndex::commit).
    ///
    /// # Panics
    ///
    /// Panics if the pending, inner index, or keymap mutex is poisoned.
    pub fn compact(&self) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.commit_locked(true)
    }

    /// Compatibility alias for [`compact`](VectorIndex::compact): flushes the
    /// journal and rewrites `index.usearch` + `keymap.bin` unconditionally.
    ///
    /// Prefer [`commit`](VectorIndex::commit) (which compacts only when it
    /// needs to) or [`compact`](VectorIndex::compact) by name; this exists so
    /// pre-journal callers keep working.
    ///
    /// # Errors
    ///
    /// Same as [`compact`](VectorIndex::compact).
    pub fn save(&self) -> Result<()> {
        self.compact()
    }

    /// Number of records currently in `journal.bin` — vectors that are durable
    /// but not yet folded into `index.usearch`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the journal's metadata cannot be read.
    pub fn journal_len(&self) -> Result<usize> {
        self.journal.len()
    }

    /// Body of [`commit`](VectorIndex::commit), with the file lock already
    /// held by the caller.
    fn commit_locked(&self, end_of_batch: bool) -> Result<()> {
        self.write_meta_upgrade()?;

        if end_of_batch {
            // This commit compacts unconditionally, and the compaction makes
            // `pending` durable inside this same locked section. Journalling
            // it first would write every vector twice and buffer the whole
            // batch a second time; a crash before the compaction loses the
            // vectors either way, and the caller never saw `Ok`.
            return self.compact_locked();
        }

        let drained: Vec<(ItemId, Vec<f32>)> = {
            let mut pending = self.pending.lock().expect("pending mutex poisoned");
            std::mem::take(&mut *pending)
        };
        // Append in bounded chunks: `Journal::append` buffers its argument
        // before writing, so one giant call would hold the whole queue twice.
        let mut written = 0_usize;
        for chunk in drained.chunks(JOURNAL_APPEND_CHUNK) {
            if let Err(e) = self.journal.append(chunk) {
                // Put the unwritten tail back (ahead of anything added
                // meanwhile) so a later commit can still make it durable; the
                // chunks already appended are on disk and must not be
                // duplicated.
                self.requeue(drained[written..].to_vec());
                return Err(e);
            }
            written += chunk.len();
        }

        if self.journal.len()? > COMPACT_THRESHOLD {
            self.compact_locked()?;
        }
        Ok(())
    }

    /// Put drained-but-unwritten records back at the front of `pending`,
    /// ahead of anything added meanwhile, preserving insertion order.
    fn requeue(&self, mut records: Vec<(ItemId, Vec<f32>)>) {
        if records.is_empty() {
            return;
        }
        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        records.append(&mut pending);
        *pending = records;
        drop(pending);
    }

    /// Fold everything into `index.usearch` + `keymap.bin` and truncate the
    /// journal, with the exclusive file lock already held by the caller.
    ///
    /// Two things must happen before this handle may write the graph out, and
    /// both exist because another handle may have moved the directory on since
    /// this one loaded:
    ///
    /// 1. **Adopt a newer on-disk state.** If `keymap.bin`'s `generation` has
    ///    moved past the one this handle loaded, another handle compacted:
    ///    this handle's in-memory graph predates that work and the journal no
    ///    longer holds it (the other handle truncated it). Saving from here
    ///    would silently delete the other handle's vectors, so reload
    ///    `index.usearch` + `keymap.bin` first.
    /// 2. **Replay the journal.** Another handle may have appended vectors
    ///    that nobody has compacted yet. Truncating a journal this handle has
    ///    not absorbed would delete them, so a compaction always absorbs the
    ///    journal it is about to discard.
    fn compact_locked(&self) -> Result<()> {
        self.reload_if_stale()?;
        self.absorb_journal()?;

        // Snapshot the graph and take the queue in one critical section:
        // `add_entries` holds the inner lock while it pushes to `pending`, so
        // a concurrent add is either inside the file written below or still in
        // `pending` afterwards, never in neither.
        let tmp_usearch = self.path.join("index.usearch.tmp");
        let inner = self.inner.lock().expect("usearch mutex poisoned");
        let drained: Vec<(ItemId, Vec<f32>)> = {
            let mut pending = self.pending.lock().expect("pending mutex poisoned");
            std::mem::take(&mut *pending)
        };
        let count = inner.size();
        let saved = if count > 0 {
            // Only persist the USearch binary when there is at least one item.
            // usearch::Index::load on a file saved from an empty index may
            // segfault on some platforms; we avoid generating such files.
            inner
                .save(tmp_usearch.to_str().unwrap())
                .map_err(|e| Error::Usearch {
                    context: "usearch save",
                    reason: format!("{e}"),
                })
        } else {
            Ok(())
        };
        drop(inner);

        match saved.and_then(|()| self.publish_compaction(count, &tmp_usearch)) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Nothing was published, so the drained vectors are still only
                // in memory. Put them back so the next commit journals them.
                self.requeue(drained);
                Err(e)
            }
        }
    }

    /// Rename the freshly written pair into place, stamp the new generation,
    /// and drop the journal. Called by [`compact_locked`] with the exclusive
    /// lock held and `index.usearch.tmp` already written when `count > 0`.
    ///
    /// Order of the two on-disk mutations, and why:
    ///
    /// - `count > 0`: rename `index.usearch` first, then `keymap.bin`. A crash
    ///   in between is the *torn pair* — a graph ahead of its keymap — which
    ///   the journal replays over when there is a journal, and which
    ///   [`warn_if_graph_exceeds_keymap`](Self::warn_if_graph_exceeds_keymap)
    ///   reports when there is not.
    /// - `count == 0`: **remove** `index.usearch` first, then rename the empty
    ///   `keymap.bin`. A crash in between leaves (no index + the old keymap),
    ///   which `load` treats as a stale keymap and resets to empty. Renaming
    ///   the keymap first would instead leave (old index + empty keymap): the
    ///   removed vectors would still be in the graph, unnamed, and every
    ///   later compaction would write them out again.
    fn publish_compaction(&self, count: usize, tmp_usearch: &Path) -> Result<()> {
        if count > 0 {
            sync_file(tmp_usearch)?;
            fs::rename(tmp_usearch, &self.usearch_path).map_err(Error::Io)?;
        } else if self.usearch_path.exists() {
            // Empty graph: drop the stale `index.usearch` *before* the empty
            // keymap is renamed into place. A crash between the two then
            // leaves (no index + the old keymap), which `load` recognises as
            // stale and resets to empty. The other ordering leaves (old index
            // + empty keymap), which resurrects every removed vector as an
            // orphan in the graph the next compaction saves.
            fs::remove_file(&self.usearch_path).map_err(Error::Io)?;
        }

        let generation = self.loaded_generation.load(Ordering::SeqCst) + 1;
        let bytes = {
            let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
            keymap.generation = generation;
            let bytes = bincode::serialize(&*keymap).map_err(|e| Error::Embedding {
                context: "serializing keymap",
                reason: format!("{e}"),
            });
            drop(keymap);
            bytes?
        };
        let tmp_keymap = self.path.join("keymap.bin.tmp");
        fs::write(&tmp_keymap, bytes).map_err(Error::Io)?;
        sync_file(&tmp_keymap)?;
        fs::rename(&tmp_keymap, &self.keymap_path).map_err(Error::Io)?;

        // fsync the directory so the renames are durable in the order above.
        sync_dir(&self.path)?;

        // Only now is the journal redundant: on-disk state is either
        // (old index + full journal) or (new index + empty journal), and a
        // crash in between replays idempotently because every replayed id is
        // already in the freshly written keymap.
        self.journal.clear()?;

        // The journal no longer holds the removed vectors, so nothing is left
        // for the tombstones to suppress.
        let mut tombstones = self.tombstones.lock().expect("tombstone mutex poisoned");
        tombstones.clear();
        drop(tombstones);

        self.loaded_generation.store(generation, Ordering::SeqCst);
        Ok(())
    }

    /// If `keymap.bin`'s generation has moved past the one this handle loaded,
    /// replace the in-memory graph and keymap with what is on disk, then
    /// re-insert anything still queued in `pending`. Tombstones survive.
    ///
    /// Called at the top of every compaction, with the exclusive lock held.
    fn reload_if_stale(&self) -> Result<()> {
        if !self.keymap_path.exists() {
            return Ok(());
        }
        let version =
            disk_format_version(&self.meta_path)?.unwrap_or_else(|| FORMAT_VERSION.to_string());
        let disk_keymap = read_keymap(&self.keymap_path, &version)?;
        if disk_keymap.generation == self.loaded_generation.load(Ordering::SeqCst) {
            return Ok(());
        }
        tracing::debug!(
            path = %self.path.display(),
            loaded = self.loaded_generation.load(Ordering::SeqCst),
            on_disk = disk_keymap.generation,
            "another handle compacted; reloading before saving",
        );

        let fresh = usearch::Index::new(&self.usearch_options()).map_err(|e| Error::Usearch {
            context: "constructing usearch::Index for a stale-handle reload",
            reason: format!("{e}"),
        })?;
        if self.usearch_path.exists() {
            fresh
                .load(self.usearch_path.to_str().unwrap())
                .map_err(|e| Error::Usearch {
                    context: "reloading usearch index after another handle compacted",
                    reason: format!("{e}"),
                })?;
        } else {
            fresh.reserve(1024).map_err(|e| Error::Usearch {
                context: "reserving initial usearch capacity",
                reason: format!("{e}"),
            })?;
        }

        let mut disk_keymap = disk_keymap;
        let generation = disk_keymap.generation;
        let mut inner = self.inner.lock().expect("usearch mutex poisoned");
        let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
        // Anything queued here has never reached the journal, so the reloaded
        // graph does not have it; re-insert it under fresh keys.
        let queued: Vec<(ItemId, Vec<f32>)> = {
            let pending = self.pending.lock().expect("pending mutex poisoned");
            pending.clone()
        };
        let entries: Vec<(ItemId, &[f32])> = queued
            .iter()
            .map(|(id, vector)| (*id, vector.as_slice()))
            .collect();
        let inserted = insert_entries(&fresh, &mut disk_keymap, &entries);
        if inserted.is_ok() {
            *inner = fresh;
            *keymap = disk_keymap;
            self.loaded_generation.store(generation, Ordering::SeqCst);
        }
        drop(keymap);
        drop(inner);
        inserted
    }

    /// The `usearch::IndexOptions` this handle's graph was built with.
    /// `usearch::IndexOptions` is neither `Clone` nor `Copy`, so it is rebuilt
    /// from `meta` plus the `expansion_search` the handle was opened with.
    const fn usearch_options(&self) -> IndexOptions {
        IndexOptions {
            dimensions: self.meta.dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: self.meta.hnsw_m,
            expansion_add: self.meta.hnsw_ef_construction,
            expansion_search: self.expansion_search,
            multi: false,
        }
    }

    /// Rewrite `.meta.json` with `format_version "2"` the first time a v1
    /// directory commits. Called with the file lock held.
    fn write_meta_upgrade(&self) -> Result<()> {
        if !self.meta_upgrade_pending.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let write = || -> Result<()> {
            let text = serde_json::to_string_pretty(&self.meta).map_err(|e| Error::Embedding {
                context: "serializing .meta.json",
                reason: format!("{e}"),
            })?;
            fs::write(&self.meta_path, text).map_err(Error::Io)
        };
        write().inspect_err(|_| {
            // Leave the upgrade outstanding so the next commit retries it.
            self.meta_upgrade_pending.store(true, Ordering::SeqCst);
        })
    }

    /// Take the exclusive advisory lock on `<dir>/lock` for a commit.
    fn acquire_lock(&self) -> Result<CommitLock> {
        acquire_lock_at(&self.lock_path, LockMode::Exclusive)
    }

    /// Number of vectors currently *searchable*, journal replay included.
    ///
    /// The canonical count is [`doc_count`](VectorIndex::doc_count), which is
    /// what the search paths use; this is the same number as a plain `usize`,
    /// kept because `len`/`is_empty` read better at call sites that just want
    /// to know whether anything is indexed.
    ///
    /// # Panics
    ///
    /// Panics if the keymap mutex is poisoned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keymap
            .lock()
            .expect("keymap mutex poisoned")
            .forward
            .len()
    }

    /// Returns `true` if no vectors are indexed. See
    /// [`len`](VectorIndex::len).
    ///
    /// # Panics
    ///
    /// Panics if the inner index mutex is poisoned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The canonical count of vectors currently indexed, journal replay
    /// included. [`len`](VectorIndex::len) returns the same number as a
    /// `usize`.
    ///
    /// This is the number of entries in the keymap — the *searchable* count —
    /// not `usearch::Index::size()`. The two differ only after a torn
    /// compaction has left orphan vectors in the graph that no keymap entry
    /// names (see [`warn_if_graph_exceeds_keymap`](Self::warn_if_graph_exceeds_keymap)).
    /// `search` filters every hit through the keymap, so counting the graph
    /// would over-report what a query can actually return, permanently.
    ///
    /// # Errors
    ///
    /// Always succeeds in the current implementation; returns `Result<u64>` for
    /// forward-compatibility.
    ///
    /// # Panics
    ///
    /// Panics if the keymap mutex is poisoned.
    pub fn doc_count(&self) -> Result<u64> {
        Ok(self.len() as u64)
    }

    /// Returns `true` if the given [`ItemId`] is present in the index.
    ///
    /// This is an O(1) lookup via the in-memory keymap reverse index.
    ///
    /// # Panics
    ///
    /// Panics if the keymap mutex is poisoned.
    #[must_use]
    pub fn contains(&self, id: ItemId) -> bool {
        self.keymap
            .lock()
            .expect("keymap mutex poisoned")
            .reverse
            .contains_key(&id)
    }

    // ── Search ────────────────────────────────────────────────────────────

    /// Find the `k` nearest neighbours to `query_vector` by cosine similarity.
    ///
    /// Returns results sorted descending by `score` (1.0 = identical,
    /// −1.0 = opposite). `USearch` returns cosine *distance* (0 = identical,
    /// 2 = opposite); we convert via `score = 1.0 − distance`.
    ///
    /// Results are filtered to IDs present in the keymap; keys that have been
    /// removed are silently skipped.
    ///
    /// # Errors
    ///
    /// - [`Error::DimMismatch`] if `query_vector.len() != meta.dim`.
    /// - [`Error::Usearch`] on `USearch` search failure.
    ///
    /// # Panics
    ///
    /// Panics if the inner index or keymap mutex is poisoned.
    pub fn search(&self, query_vector: &[f32], k: usize) -> Result<Vec<VectorHit>> {
        if query_vector.len() != self.meta.dim {
            return Err(Error::DimMismatch {
                expected: self.meta.dim,
                got: query_vector.len(),
            });
        }
        let matches = self
            .inner
            .lock()
            .expect("usearch mutex poisoned")
            .search(query_vector, k)
            .map_err(|e| Error::Usearch {
                context: "usearch search",
                reason: format!("{e}"),
            })?;

        let keymap = self.keymap.lock().expect("keymap mutex poisoned");
        Ok(matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .filter_map(|(key, dist)| {
                // USearch cosine returns distance in [0, 2]; convert to similarity.
                let score = 1.0 - dist;
                keymap
                    .forward
                    .get(key)
                    .map(|id| VectorHit { id: *id, score })
            })
            .collect())
    }
}

// ── VectorHit ─────────────────────────────────────────────────────────────

/// A single result from [`VectorIndex::search`].
#[derive(Debug, Clone)]
pub struct VectorHit {
    /// The item identifier.
    pub id: ItemId,
    /// Cosine similarity score in `[-1.0, 1.0]`. Higher = more similar.
    /// Self-similarity of an L2-normalised vector is 1.0.
    pub score: f32,
}

// ── EmbedderIndex ─────────────────────────────────────────────────────────

/// Composite [`singularmem_core::IndexHook`] that embeds ingested items and
/// stores their vectors in a [`VectorIndex`].
///
/// `EmbedderIndex` bridges the core trait boundary: `on_ingest` embeds the
/// item's content via the [`Embedder`] and adds the vector to the
/// [`VectorIndex`]; `commit` flushes both the `USearch` binary and `keymap.bin`
/// to disk.
///
/// Wire into a [`singularmem_core::Store`] via
/// [`Store::open_with_hook`](singularmem_core::Store::open_with_hook) (single
/// hook) or [`Store::open_with_hooks`](singularmem_core::Store::open_with_hooks)
/// (alongside a Tantivy `Index` via `MultiHook`).
pub struct EmbedderIndex {
    embedder: Box<dyn Embedder>,
    vector_index: VectorIndex,
    /// Set by `on_ingest_batch` and consumed by the `commit` that follows it,
    /// so a bulk ingest compacts once at its end while single ingests only
    /// append to the journal.
    batch_end: AtomicBool,
}

impl EmbedderIndex {
    /// Open (or create) an `EmbedderIndex` at `dir` using the given `embedder`.
    ///
    /// Delegates to [`VectorIndex::open`] for the on-disk setup and
    /// model-identity check.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`VectorIndex::open`].
    pub fn open(dir: impl AsRef<Path>, embedder: Box<dyn Embedder>) -> Result<Self> {
        let vector_index = VectorIndex::open(dir, embedder.as_ref())?;
        Ok(Self {
            embedder,
            vector_index,
            batch_end: AtomicBool::new(false),
        })
    }

    /// Returns a reference to the underlying [`VectorIndex`].
    pub const fn vector_index(&self) -> &VectorIndex {
        &self.vector_index
    }

    /// Returns a reference to the underlying [`Embedder`].
    pub fn embedder(&self) -> &dyn Embedder {
        self.embedder.as_ref()
    }
}

impl singularmem_core::IndexHook for EmbedderIndex {
    /// Embed `item.content` and add the vector to the [`VectorIndex`].
    ///
    /// # Errors
    ///
    /// Returns [`singularmem_core::Error::Io`] wrapping the search error
    /// message if embedding or indexing fails.
    fn on_ingest(&self, item: &singularmem_core::Item) -> singularmem_core::Result<()> {
        let v = self
            .embedder
            .embed(&item.content)
            .map_err(|ref e| to_core_err(e))?;
        self.vector_index
            .add(item.id, &v)
            .map_err(|ref e| to_core_err(e))
    }

    /// Equivalent to `on_ingest`: re-embed and re-index.
    fn on_reindex(&self, item: &singularmem_core::Item) -> singularmem_core::Result<()> {
        self.on_ingest(item)
    }

    /// Embed `items` in chunks of [`EMBED_CHUNK`] via
    /// [`Embedder::embed_batch`], then add each vector to the
    /// [`VectorIndex`]. On an error mid-way, vectors already added stay
    /// added; the error propagates to the caller.
    ///
    /// # Errors
    ///
    /// Returns [`singularmem_core::Error::Io`] wrapping the search error
    /// message if embedding or indexing fails.
    fn on_ingest_batch(&self, items: &[singularmem_core::Item]) -> singularmem_core::Result<()> {
        for chunk in items.chunks(EMBED_CHUNK) {
            let texts: Vec<&str> = chunk.iter().map(|i| i.content.as_str()).collect();
            let vectors = self
                .embedder
                .embed_batch(&texts)
                .map_err(|ref e| to_core_err(e))?;
            if vectors.len() != chunk.len() {
                return Err(to_core_err(&Error::Embedding {
                    context: "embed_batch returned a short result",
                    reason: format!("expected {} vectors, got {}", chunk.len(), vectors.len()),
                }));
            }
            let entries: Vec<(ItemId, &[f32])> = chunk
                .iter()
                .zip(vectors.iter())
                .map(|(item, v)| (item.id, v.as_slice()))
                .collect();
            self.vector_index
                .add_batch(&entries)
                .map_err(|ref e| to_core_err(e))?;
        }
        // Tell the `commit` that follows this call to compact: a bulk ingest
        // is worth one graph rewrite, and it leaves the journal empty for the
        // single ingests that come after.
        self.batch_end.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Persist the [`VectorIndex`]: append the vectors added since the last
    /// commit to `journal.bin`, compacting into `index.usearch` +
    /// `keymap.bin` only when the journal outgrew
    /// [`COMPACT_THRESHOLD`] or this commit closes an
    /// [`on_ingest_batch`](singularmem_core::IndexHook::on_ingest_batch).
    ///
    /// # Errors
    ///
    /// Returns [`singularmem_core::Error::Io`] wrapping the search error
    /// message if the flush fails.
    fn commit(&self) -> singularmem_core::Result<()> {
        // Read, then clear only on success: a failed commit leaves the batch
        // still open, so the retry still compacts instead of degrading into a
        // journal append that leaves the bulk ingest un-compacted.
        let end_of_batch = self.batch_end.load(Ordering::SeqCst);
        self.vector_index
            .commit(end_of_batch)
            .map_err(|ref e| to_core_err(e))?;
        if end_of_batch {
            self.batch_end.store(false, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl EmbedderIndex {
    /// Embed `query`, run KNN against the [`VectorIndex`], and return
    /// filtered, timed results.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Embedding`] if the embedder fails or [`Error::Usearch`]
    /// if the KNN search fails.
    pub fn semantic_search(
        &self,
        query: &str,
        opts: &crate::semantic_query::SemanticSearchOptions,
    ) -> Result<crate::semantic_query::SemanticSearchResults> {
        use crate::semantic_query::{SemanticHit, SemanticSearchResults};
        let start = std::time::Instant::now();
        let qv = self.embedder.embed(query)?;
        let raw = self.vector_index.search(&qv, opts.limit)?;
        let total_indexed = self.vector_index.doc_count()?;
        let hits: Vec<SemanticHit> = raw
            .into_iter()
            .filter(|h| h.score >= opts.min_score)
            .map(|h| SemanticHit {
                id: h.id,
                score: h.score,
            })
            .collect();
        Ok(SemanticSearchResults {
            hits,
            elapsed: start.elapsed(),
            total_indexed,
        })
    }
}

/// Assign the next sequential keys to `entries`, insert the vectors into
/// `index`, and record the mappings in `keymap`.
///
/// # Re-adding an id that is already indexed
///
/// [`VectorIndex::add`] is documented as "add **or replace**", so an id that
/// the keymap already knows must end up with exactly one vector — the new
/// one. The id's previous key is therefore evicted from the graph and from
/// both keymap directions *before* the new key is issued. Keys are never
/// reused (`next_key` only ever moves forward, and the old key's slot stays
/// a hole), which keeps the numbering aligned with the sequential order
/// journal replay reproduces.
///
/// Without the eviction, `reindex --with-embeddings` without
/// `--reset-vectors` doubled every vector in the graph and `search` returned
/// the same id twice — once per key.
///
/// # Collision-proof replay
///
/// If the graph already holds the key about to be issued, `index.usearch` is
/// *ahead* of `keymap.bin`: a compaction renamed the graph and then crashed
/// before renaming the keymap. `USearch` refuses a duplicate key outright
/// ("Duplicate keys not allowed"), which used to make such a directory
/// permanently unopenable. `USearch` 2.15 cannot enumerate a loaded graph's
/// keys, so the highest key present cannot be computed up front; instead each
/// key is checked as it is issued and a stale occupant is evicted first. The
/// record being replayed is the authoritative value for that key — it is the
/// very record the crashed compaction folded in to produce it.
fn insert_entries(
    index: &usearch::Index,
    keymap: &mut Keymap,
    entries: &[(ItemId, &[f32])],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    // USearch requires explicit reservation before insertions when the index
    // was loaded from disk (capacity does not auto-grow). Reserve in doubling
    // increments to amortise the cost, computed once for the whole batch
    // rather than once per entry.
    let capacity = index.capacity();
    let needed = index.size() + entries.len();
    if needed > capacity {
        let new_cap = (capacity + 1).max(needed + 1024);
        index.reserve(new_cap).map_err(|e| Error::Usearch {
            context: "auto-reserving usearch capacity before add_batch",
            reason: format!("{e}"),
        })?;
    }
    for (id, vector) in entries {
        // "Add or replace": drop whatever key this id already holds, so the
        // graph never carries two vectors for one id.
        if let Some(old_key) = keymap.reverse.remove(id) {
            keymap.forward.remove(&old_key);
            if index.contains(old_key) {
                index.remove(old_key).map_err(|e| Error::Usearch {
                    context: "evicting the previous vector of a re-added id",
                    reason: format!("{e}"),
                })?;
            }
        }
        let key = keymap.next_key;
        keymap.next_key += 1;
        if index.contains(key) {
            tracing::debug!(
                key,
                "usearch graph is ahead of keymap.bin (torn compaction); replacing the orphan key",
            );
            index.remove(key).map_err(|e| Error::Usearch {
                context: "evicting an orphan usearch key during replay",
                reason: format!("{e}"),
            })?;
        }
        index.add(key, vector).map_err(|e| Error::Usearch {
            context: "usearch add",
            reason: format!("{e}"),
        })?;
        keymap.forward.insert(key, *id);
        keymap.reverse.insert(*id, key);
    }
    Ok(())
}

/// Convert a search crate [`Error`] into a [`singularmem_core::Error`].
///
/// The core trait cannot reference the search error type without creating a
/// circular dependency, so we wrap as `Error::Io` carrying the full message.
/// Type information is lost but the full string survives for logging.
fn to_core_err(e: &crate::Error) -> singularmem_core::Error {
    singularmem_core::Error::Io(std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use fs4::FileExt;
    use singularmem_core::ItemId;

    use super::{open_lock_file, Error, VectorIndex};
    use crate::embedder::Embedder;
    use crate::testing::MockEmbedder;

    /// Hold `<dir>/lock` exclusively from another thread for `hold`, using
    /// fs4 directly so the test does not depend on `VectorIndex`'s own
    /// acquisition path. Returns once the lock is definitely held.
    fn hold_lock_for(lock_path: std::path::PathBuf, hold: Duration) -> std::thread::JoinHandle<()> {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let file = open_lock_file(&lock_path).expect("lock file opens");
            FileExt::try_lock_exclusive(&file).expect("uncontended lock is free");
            tx.send(()).expect("receiver alive");
            std::thread::sleep(hold);
            FileExt::unlock(&file).expect("unlock");
        });
        rx.recv().expect("lock taken");
        handle
    }

    #[test]
    fn commit_waits_out_a_briefly_held_lock() {
        let dir = tempfile::tempdir().unwrap();
        let e = MockEmbedder::default();
        let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
        idx.add(fresh_id(), &e.embed("waited").unwrap()).unwrap();

        // Shorter than the 50+100+200+400 ms the retry schedule sleeps.
        let holder = hold_lock_for(dir.path().join("v/lock"), Duration::from_millis(300));
        let started = Instant::now();
        idx.commit(true)
            .expect("the commit must back off and then succeed");
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "the commit should have waited for the lock, not sailed past it"
        );
        holder.join().unwrap();
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn commit_reports_the_lock_context_once_the_backoff_is_exhausted() {
        let dir = tempfile::tempdir().unwrap();
        let e = MockEmbedder::default();
        let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
        idx.add(fresh_id(), &e.embed("blocked").unwrap()).unwrap();

        // Longer than the 750 ms total backoff, so all five attempts fail.
        let holder = hold_lock_for(dir.path().join("v/lock"), Duration::from_millis(1_500));
        match idx.commit(true) {
            Err(Error::Usearch { context, reason }) => {
                assert_eq!(context, "acquiring vector index lock");
                assert!(reason.contains("busy"), "unexpected reason: {reason}");
            }
            other => panic!("expected a lock-acquisition error, got {other:?}"),
        }
        holder.join().unwrap();
        // The vectors are still queued, so a later commit can persist them.
        idx.commit(true).unwrap();
        assert_eq!(idx.len(), 1);
    }

    fn fresh_id() -> ItemId {
        ItemId::from_str(&ulid::Ulid::new().to_string()).expect("freshly minted ulid parses")
    }

    #[test]
    fn add_batch_of_n_equals_n_add_calls() {
        let dir = tempfile::tempdir().unwrap();
        let e = MockEmbedder::default();
        let idx_batch = VectorIndex::open(dir.path().join("batch"), &e).unwrap();
        let idx_single = VectorIndex::open(dir.path().join("single"), &e).unwrap();

        let ids: Vec<ItemId> = (0..5).map(|_| fresh_id()).collect();
        let vectors: Vec<Vec<f32>> = (0..ids.len())
            .map(|i| e.embed(&format!("batch item {i}")).unwrap())
            .collect();

        let entries: Vec<(ItemId, &[f32])> = ids
            .iter()
            .zip(vectors.iter())
            .map(|(id, v)| (*id, v.as_slice()))
            .collect();
        idx_batch.add_batch(&entries).unwrap();

        for (id, v) in ids.iter().zip(vectors.iter()) {
            idx_single.add(*id, v).unwrap();
        }

        for id in &ids {
            assert!(idx_batch.contains(*id), "batch-added id should be present");
            assert!(
                idx_single.contains(*id),
                "singly-added id should be present"
            );
        }
        assert_eq!(
            idx_batch.doc_count().unwrap(),
            idx_single.doc_count().unwrap(),
            "add_batch of N entries should match N individual add calls"
        );
        assert_eq!(idx_batch.doc_count().unwrap(), ids.len() as u64);
    }

    #[test]
    fn add_batch_dimension_mismatch_in_middle_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let e = MockEmbedder::default(); // dim 384
        let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();

        let id1 = fresh_id();
        let id2 = fresh_id();
        let id3 = fresh_id();

        let v1 = e.embed("hello").unwrap();
        let bad = vec![0.0_f32; 128]; // wrong dim, in the middle of the batch
        let v3 = e.embed("world").unwrap();

        let entries: Vec<(ItemId, &[f32])> = vec![
            (id1, v1.as_slice()),
            (id2, bad.as_slice()),
            (id3, v3.as_slice()),
        ];

        let err = idx.add_batch(&entries).unwrap_err();
        assert!(matches!(
            err,
            Error::DimMismatch {
                expected: 384,
                got: 128
            }
        ));
        assert!(
            !idx.contains(id1),
            "a dimension mismatch anywhere in the batch must add nothing at all"
        );
        assert_eq!(idx.doc_count().unwrap(), 0);
    }
}
