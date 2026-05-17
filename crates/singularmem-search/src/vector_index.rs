//! `VectorIndex` — wraps `usearch::Index` with a sidecar `.meta.json` carrying
//! model + dimensionality + HNSW params. The on-disk layout is:
//!
//! ```text
//! <dir>/
//!   .meta.json      — VectorIndexMeta (serde_json)
//!   index.usearch   — USearch binary (written on first save)
//!   keymap.bin      — Keymap (bincode; u64 key ↔ ItemId)
//! ```
//!
//! Tasks 5-7 of the search-v0-embeddings plan implement open + add/remove/save +
//! search respectively.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use singularmem_core::ItemId;
use usearch::{IndexOptions, MetricKind, ScalarKind};

use crate::embedder::Embedder;
use crate::error::{Error, Result};

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
        Self { hnsw_m: 16, hnsw_ef_construction: 128, expansion_search: 64 }
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
    /// Monotonic format version. Always `"1"` for v0.3.0.
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
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Keymap {
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

// ── VectorIndex ───────────────────────────────────────────────────────────

/// `USearch`-backed approximate nearest-neighbour (ANN) index with model-identity
/// checking and bidirectional `ItemId` ↔ `u64` key mapping.
///
/// # Thread safety
///
/// Both the inner `usearch::Index` and the [`Keymap`] are guarded by [`Mutex`].
/// Multiple threads can call [`add`](VectorIndex::add),
/// [`remove`](VectorIndex::remove), and [`search`](VectorIndex::search)
/// concurrently; each operation acquires its own lock. [`save`](VectorIndex::save)
/// acquires both locks in sequence (inner first, then keymap) to avoid partial
/// writes.
pub struct VectorIndex {
    pub(crate) inner: Mutex<usearch::Index>,
    pub(crate) path: PathBuf,
    pub(crate) usearch_path: PathBuf,
    pub(crate) meta: VectorIndexMeta,
    pub(crate) keymap: Mutex<Keymap>,
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
        let meta_path = dir.join(".meta.json");
        let usearch_path = dir.join("index.usearch");
        let keymap_path = dir.join("keymap.bin");

        // ── Load or create meta ───────────────────────────────────────────
        let meta = if meta_path.exists() {
            let text = fs::read_to_string(&meta_path).map_err(Error::Io)?;
            let m: VectorIndexMeta =
                serde_json::from_str(&text).map_err(|e| Error::Embedding {
                    context: "parsing existing .meta.json",
                    reason: format!("{e}"),
                })?;
            if m.model_id != embedder.model_id() {
                return Err(Error::ModelMismatch {
                    path: dir.to_path_buf(),
                    found_model: m.model_id,
                    expected_model: embedder.model_id().to_string(),
                });
            }
            if m.dim != embedder.dim() {
                return Err(Error::DimMismatch { expected: m.dim, got: embedder.dim() });
            }
            m
        } else {
            VectorIndexMeta {
                format_version: "1".to_string(),
                model_id: embedder.model_id().to_string(),
                dim: embedder.dim(),
                distance: "cosine".to_string(),
                hnsw_m: options.hnsw_m,
                hnsw_ef_construction: options.hnsw_ef_construction,
                created_at: jiff::Timestamp::now(),
            }
        };

        // ── Persist meta on first open ────────────────────────────────────
        if !meta_path.exists() {
            let text =
                serde_json::to_string_pretty(&meta).map_err(|e| Error::Embedding {
                    context: "serializing .meta.json",
                    reason: format!("{e}"),
                })?;
            fs::write(&meta_path, text).map_err(Error::Io)?;
        }

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
        if usearch_path.exists() {
            inner.load(usearch_path.to_str().unwrap()).map_err(|e| Error::Usearch {
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
        let keymap = if keymap_path.exists() {
            let bytes = fs::read(&keymap_path).map_err(Error::Io)?;
            bincode::deserialize::<Keymap>(&bytes).map_err(|e| Error::Embedding {
                context: "deserializing keymap.bin",
                reason: format!("{e}"),
            })?
        } else {
            Keymap::default()
        };

        Ok(Self {
            inner: Mutex::new(inner),
            path: dir.to_path_buf(),
            usearch_path,
            meta,
            keymap: Mutex::new(keymap),
        })
    }

    /// Returns a reference to the index metadata.
    pub const fn meta(&self) -> &VectorIndexMeta {
        &self.meta
    }

    // ── Mutation operations ───────────────────────────────────────────────

    /// Add (or replace) one item's embedding vector. Assigns a sequential `u64`
    /// key internally and records the `ItemId` ↔ key mapping in the keymap.
    ///
    /// Call [`VectorIndex::save`] to persist changes to disk.
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
        if vector.len() != self.meta.dim {
            return Err(Error::DimMismatch { expected: self.meta.dim, got: vector.len() });
        }
        let key = {
            let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
            let key = keymap.next_key;
            keymap.next_key += 1;
            keymap.forward.insert(key, id);
            keymap.reverse.insert(id, key);
            key
        };
        self.inner
            .lock()
            .expect("usearch mutex poisoned")
            .add(key, vector)
            .map_err(|e| Error::Usearch { context: "usearch add", reason: format!("{e}") })
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
        Ok(())
    }

    /// Flush both the `USearch` binary (`index.usearch`) and the keymap
    /// (`keymap.bin`) to disk.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Usearch`] on `USearch` serialisation failure or
    /// [`Error::Io`] on filesystem write failure.
    ///
    /// # Panics
    ///
    /// Panics if the inner index or keymap mutex is poisoned.
    pub fn save(&self) -> Result<()> {
        self.inner
            .lock()
            .expect("usearch mutex poisoned")
            .save(self.usearch_path.to_str().unwrap())
            .map_err(|e| Error::Usearch { context: "usearch save", reason: format!("{e}") })?;

        let bytes = bincode::serialize(&*self.keymap.lock().expect("keymap mutex poisoned"))
            .map_err(|e| Error::Embedding {
                context: "serializing keymap",
                reason: format!("{e}"),
            })?;
        fs::write(self.path.join("keymap.bin"), bytes).map_err(Error::Io)
    }

    /// Returns the number of vectors currently indexed.
    ///
    /// # Errors
    ///
    /// Always succeeds in the current implementation; returns `Result<u64>` for
    /// forward-compatibility.
    ///
    /// # Panics
    ///
    /// Panics if the inner index mutex is poisoned.
    pub fn doc_count(&self) -> Result<u64> {
        Ok(self.inner.lock().expect("usearch mutex poisoned").size() as u64)
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
            .map_err(|e| Error::Usearch { context: "usearch search", reason: format!("{e}") })?;

        let keymap = self.keymap.lock().expect("keymap mutex poisoned");
        Ok(matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .filter_map(|(key, dist)| {
                // USearch cosine returns distance in [0, 2]; convert to similarity.
                let score = 1.0 - dist;
                keymap.forward.get(key).map(|id| VectorHit { id: *id, score })
            })
            .collect())
    }
}

// ── VectorHit ─────────────────────────────────────────────────────────────

/// A single result from [`VectorIndex::search`].
pub struct VectorHit {
    /// The item identifier.
    pub id: ItemId,
    /// Cosine similarity score in `[-1.0, 1.0]`. Higher = more similar.
    /// Self-similarity of an L2-normalised vector is 1.0.
    pub score: f32,
}
