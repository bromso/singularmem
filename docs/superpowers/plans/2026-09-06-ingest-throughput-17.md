# Ingest Throughput (sub-project 17) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Batch embeddings inside the existing synchronous hook contract and replace the full USearch rewrite on every commit with an append-only vector journal, so bulk ingest clears the Principle X floor with margin and single-item ingest cost stops growing with store size.

**Architecture:** `IndexHook` gains a defaulted `on_ingest_batch`; `Store::ingest_many` and the CLI reindex call it per batch. `EmbedderIndex` overrides it to embed in chunks of 64. `VectorIndex` keeps a pending buffer; `commit` appends pending vectors to `journal.bin` and compacts (full save via temp-and-rename, journal truncated) at a 1,000-record threshold or at batch end. `open` replays the journal. Commits are serialised across processes with an advisory file lock.

**Tech Stack:** Rust 2021 (MSRV 1.85), usearch 2.15.3, bincode, fs4 0.8 (already in the lock file via Tantivy), Criterion benches, existing `MockEmbedder`.

**Spec:** `docs/superpowers/specs/2026-09-06-ingest-throughput-17-design.md` — the contract. Where this plan and the spec differ, the spec governs; record deviations in the spec's "Deviations" section.

## Global Constraints

- Semantics stay synchronous: when an ingest returns, its vectors are on disk (journal or index) and searchable.
- `EMBED_CHUNK = 64`; `COMPACT_THRESHOLD = 1_000` records; both `const` in `crates/singularmem-search`, not configurable.
- Journal file `journal.bin` in `<store>.vectors/`: header magic `SMVJ`, `u16` version = 1, `u32 dim`, `u16` model-id length, model-id bytes; records `[16-byte ULID big-endian][dim × f32 little-endian]`; one `write_all` + `fsync` per commit's records; a trailing partial record is discarded on replay.
- Compaction writes `index.usearch` and `keymap.bin` to temp files in the same directory, `fsync`s, renames over the originals, then truncates the journal to its header.
- `.meta.json` `format_version` becomes `"2"`; a `"1"` directory opens unchanged and is rewritten as `"2"` on its first commit. `reindex` deletes the whole directory.
- Commit takes an exclusive advisory lock on `<store>.vectors/lock` with 5 attempts and a doubling delay from 50 ms; exhaustion is the existing "index busy" style error (`Error::Usearch { context: "acquiring vector index lock", .. }`).
- Journal header `dim`/model id must equal `.meta.json`; mismatch → `Error::ModelMismatch` / `Error::DimMismatch` (existing variants).
- All tests offline with `MockEmbedder`; clippy pedantic + nursery `-D warnings` workspace-wide; `cargo fmt --all`; every commit `git commit -s`; never stage `.superpowers/`, `.agents/`, `.claude/`, `skills-lock.json`, `*.proptest-regressions`, `*.node`.
- Perf gates (`.github/scripts/perf-check.sh`): `ingest_throughput/ingest_with_indexes` median ≥ 50 items/s (target ≥ 200); `ingest_throughput/ingest_single_with_indexes` median ≤ 20 ms at 20,000 pre-seeded vectors.

## File Structure

```
crates/singularmem-core/src/hook.rs            + on_ingest_batch default; MultiHook forwards   (Task 1)
crates/singularmem-core/src/ingest.rs          ingest_many calls on_ingest_batch once           (Task 1)
crates/singularmem-core/tests/hook_batch.rs    (Task 1)
crates/singularmem-search/src/vector_index.rs  EmbedderIndex::on_ingest_batch (Task 2); pending buffer, commit/compaction, replay, lock (Task 4)
crates/singularmem-search/src/vector_journal.rs  journal codec (Task 3)
crates/singularmem-search/src/lib.rs           pub(crate) mod vector_journal
crates/singularmem-search/Cargo.toml           + fs4 (Task 4)
src/commands/search.rs                         reindex embeds in batches of 500 (Task 2)
crates/singularmem-search/tests/{batch_hook,vector_journal,vector_index_journal,vector_index_concurrency}.rs
crates/singularmem-search/benches/search_perf.rs  + two ingest benches (Task 5)
.github/scripts/perf-check.sh                  + two gates (Task 5)
docs/formats/vectors-v2.md, docs/benchmarks/ingest.md, README.md, docs/hooks.md, spec Deviations (Task 5)
```

---

### Task 1: `IndexHook::on_ingest_batch` and `ingest_many`

**Files:**
- Modify: `crates/singularmem-core/src/hook.rs` (trait + `MultiHook`)
- Modify: `crates/singularmem-core/src/ingest.rs` (~lines 179–199)
- Test: `crates/singularmem-core/tests/hook_batch.rs`

**Interfaces:**
- Produces: `IndexHook::on_ingest_batch(&self, items: &[Item]) -> Result<()>` (default loops `on_ingest`); `MultiHook::on_ingest_batch` forwarding via `run_all`.

- [ ] **Step 1: Failing tests** — `crates/singularmem-core/tests/hook_batch.rs`

```rust
use std::sync::{Arc, Mutex};

use singularmem_core::hook::MultiHook;
use singularmem_core::{IndexHook, Item, NewItem, Store};

/// Records every call so tests can assert the exact sequence.
#[derive(Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<String>>>,
    fail_on: Option<String>,
}

impl IndexHook for Recorder {
    fn on_ingest(&self, item: &Item) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push(format!("ingest:{}", item.content));
        if self.fail_on.as_deref() == Some(item.content.as_str()) {
            return Err(singularmem_core::Error::Validation { field: "hook", reason: "boom".into() });
        }
        Ok(())
    }
    fn on_reindex(&self, item: &Item) -> singularmem_core::Result<()> {
        self.on_ingest(item)
    }
    fn commit(&self) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push("commit".into());
        Ok(())
    }
}

/// Overrides the batch method so the test can see it was used.
struct BatchAware {
    calls: Arc<Mutex<Vec<String>>>,
}

impl IndexHook for BatchAware {
    fn on_ingest(&self, item: &Item) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push(format!("single:{}", item.content));
        Ok(())
    }
    fn on_reindex(&self, item: &Item) -> singularmem_core::Result<()> {
        self.on_ingest(item)
    }
    fn on_ingest_batch(&self, items: &[Item]) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push(format!("batch:{}", items.len()));
        Ok(())
    }
    fn commit(&self) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push("commit".into());
        Ok(())
    }
}

fn items(n: usize) -> Vec<NewItem> {
    (0..n).map(|i| NewItem::text(format!("item {i}"))).collect()
}

#[test]
fn default_batch_is_per_item_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let hook = Recorder { calls: calls.clone(), fail_on: None };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(hook)).unwrap();
    store.ingest_many(items(3)).unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["ingest:item 0", "ingest:item 1", "ingest:item 2", "commit"]
    );
}

#[test]
fn ingest_many_uses_the_batch_method_once_per_batch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(BatchAware { calls: calls.clone() })).unwrap();
    store.ingest_many(items(5)).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["batch:5", "commit"]);
    // Single-item ingest still uses the per-item path.
    store.ingest(NewItem::text("solo".into())).unwrap();
    assert_eq!(calls.lock().unwrap().last().unwrap(), "commit");
    assert!(calls.lock().unwrap().contains(&"single:solo".to_string()));
}

#[test]
fn multi_hook_forwards_the_batch_to_every_member() {
    let a = Arc::new(Mutex::new(Vec::new()));
    let b = Arc::new(Mutex::new(Vec::new()));
    let multi = MultiHook::new(vec![
        Box::new(BatchAware { calls: a.clone() }),
        Box::new(Recorder { calls: b.clone(), fail_on: None }),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(multi)).unwrap();
    store.ingest_many(items(2)).unwrap();
    assert_eq!(*a.lock().unwrap(), vec!["batch:2", "commit"]);
    assert_eq!(*b.lock().unwrap(), vec!["ingest:item 0", "ingest:item 1", "commit"]);
}

#[test]
fn a_failing_batch_does_not_fail_ingest_many() {
    // Items are durably stored; the hook failure is logged, not returned.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let hook = Recorder { calls, fail_on: Some("item 1".into()) };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(hook)).unwrap();
    let out = store.ingest_many(items(3)).unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(store.list().unwrap().count(), 3);
}
```

Check `Store::list()` returns an iterator of `Result<Item>` (`crates/singularmem-core/src/query.rs:137`); use `.count()` on it accordingly (`.filter_map(Result::ok).count()` if needed). Confirm the `Validation` error field names in `crates/singularmem-core/src/error.rs`.

Run: `cargo test -p singularmem-core --test hook_batch` — Expected: compile error (`on_ingest_batch` not in trait).

- [ ] **Step 2: Implement**

`hook.rs`, inside `pub trait IndexHook`:

```rust
    /// Index a batch of freshly ingested items. The default indexes each
    /// item with [`IndexHook::on_ingest`], in order, stopping at the first
    /// error. Implementations that can amortise work across items (batched
    /// embedding, one lock acquisition) override this.
    ///
    /// # Errors
    /// Whatever the per-item path returns; callers treat a failure as
    /// "stored but not searchable until `reindex`".
    fn on_ingest_batch(&self, items: &[Item]) -> Result<()> {
        items.iter().try_for_each(|item| self.on_ingest(item))
    }
```

`MultiHook`:

```rust
    fn on_ingest_batch(&self, items: &[crate::Item]) -> crate::Result<()> {
        run_all(self.hooks.iter(), "on_ingest_batch", |h| h.on_ingest_batch(items))
    }
```

`ingest.rs` `ingest_many`: replace the `for item in &out { hook.on_ingest(item) ... }` loop with one call:

```rust
            if let Err(e) = hook.on_ingest_batch(&out) {
                tracing::warn!(
                    items = out.len(),
                    error = %e,
                    "IndexHook::on_ingest_batch failed during bulk ingest; items are durably stored but some may be un-searchable. Run `singularmem reindex` to recover."
                );
            }
```

Keep the `commit` call and its warning as they are.

- [ ] **Step 3: Run, lint, commit**

Run: `cargo test -p singularmem-core` — Expected: all pass (existing hook tests unaffected). `cargo clippy -p singularmem-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`.

```bash
git add crates/singularmem-core/src/hook.rs crates/singularmem-core/src/ingest.rs crates/singularmem-core/tests/hook_batch.rs
git commit -s -m "feat(core): IndexHook::on_ingest_batch; ingest_many indexes per batch"
```

---

### Task 2: Batched embedding in `EmbedderIndex` and the CLI reindex

**Files:**
- Modify: `crates/singularmem-search/src/vector_index.rs` (the `impl singularmem_core::IndexHook for EmbedderIndex` block ~line 539)
- Modify: `src/commands/search.rs` (reindex embedding loop ~lines 291–301)
- Test: `crates/singularmem-search/tests/batch_hook.rs`; extend `tests/cli.rs` reindex test if one asserts vector counts

**Interfaces:**
- Consumes: Task 1's trait method; `Embedder::embed_batch(&[&str]) -> Result<Vec<Vec<f32>>>`; `VectorIndex::add(ItemId, &[f32])`.
- Produces: `pub const EMBED_CHUNK: usize = 64` in `vector_index.rs`.

- [ ] **Step 1: Failing test** — `crates/singularmem-search/tests/batch_hook.rs`

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use singularmem_core::{IndexHook, Item, NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, Embedder, EMBED_CHUNK};

/// Wraps MockEmbedder and counts how many `embed_batch` calls it receives
/// and the largest batch it saw.
struct Counting {
    inner: MockEmbedder,
    calls: Arc<AtomicUsize>,
    max_batch: Arc<AtomicUsize>,
}

impl Embedder for Counting {
    fn dim(&self) -> usize { self.inner.dim() }
    fn model_id(&self) -> &str { self.inner.model_id() }
    fn embed(&self, c: &str) -> singularmem_search::Result<Vec<f32>> { self.inner.embed(c) }
    fn embed_batch(&self, items: &[&str]) -> singularmem_search::Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.max_batch.fetch_max(items.len(), Ordering::SeqCst);
        self.inner.embed_batch(items)
    }
}

fn seeded(n: usize) -> (tempfile::TempDir, Vec<Item>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let max_batch = Arc::new(AtomicUsize::new(0));
    let emb = Counting { inner: MockEmbedder::default(), calls: calls.clone(), max_batch: max_batch.clone() };
    let idx = EmbedderIndex::open(dir.path().join("v"), Box::new(emb)).unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(idx)).unwrap();
    let items = store.ingest_many((0..n).map(|i| NewItem::text(format!("text number {i}")))).unwrap();
    drop(store);
    (dir, items, calls, max_batch)
}

#[test]
fn batch_ingest_embeds_in_chunks_of_embed_chunk() {
    let (_d, _items, calls, max_batch) = seeded(150);
    // 150 items -> 64 + 64 + 22 = 3 embed_batch calls
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(max_batch.load(Ordering::SeqCst), EMBED_CHUNK);
}

#[test]
fn batch_vectors_equal_per_item_vectors() {
    let (dir, items, _c, _m) = seeded(10);
    let idx = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let mock = MockEmbedder::default();
    for item in &items {
        let expected = mock.embed(&item.content).unwrap();
        let hits = idx.vector_index().search(&expected, 1).unwrap();
        assert_eq!(hits[0].id, item.id, "nearest neighbour of an item's own vector is itself");
    }
}

#[test]
fn every_item_is_present_after_batch_ingest() {
    let (dir, items, _c, _m) = seeded(150);
    let idx = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    for item in &items {
        assert!(idx.vector_index().contains(item.id));
    }
}
```

Run: `cargo test -p singularmem-search --test batch_hook` — Expected: `EMBED_CHUNK` unresolved; the chunk-count test would fail with 150 calls.

- [ ] **Step 2: Implement**

In `vector_index.rs`, near the top:

```rust
/// Items per `Embedder::embed_batch` call in [`EmbedderIndex::on_ingest_batch`].
/// 64 sits on the flat part of the measured throughput curve for the bundled
/// ONNX models; larger chunks buy nothing and cost memory.
pub const EMBED_CHUNK: usize = 64;
```

Export it from `lib.rs` (`pub use crate::vector_index::{..., EMBED_CHUNK}`).

In the `IndexHook` impl for `EmbedderIndex`:

```rust
    fn on_ingest_batch(&self, items: &[singularmem_core::Item]) -> singularmem_core::Result<()> {
        for chunk in items.chunks(EMBED_CHUNK) {
            let texts: Vec<&str> = chunk.iter().map(|i| i.content.as_str()).collect();
            let vectors = self.embedder.embed_batch(&texts).map_err(|ref e| to_core_err(e))?;
            for (item, v) in chunk.iter().zip(vectors) {
                self.vector_index.add(item.id, &v).map_err(|ref e| to_core_err(e))?;
            }
        }
        Ok(())
    }
```

`src/commands/search.rs` reindex loop: collect items in chunks of 500 and call `on_ingest_batch`:

```rust
        let mut batch: Vec<singularmem_core::Item> = Vec::with_capacity(500);
        let mut done = 0usize;
        let flush = |batch: &mut Vec<singularmem_core::Item>, done: &mut usize| -> Result<(), CliError> {
            if batch.is_empty() { return Ok(()); }
            singularmem_core::IndexHook::on_ingest_batch(&embedder_idx, batch)
                .map_err(|e| CliError::IndexOpen(e.to_string()))?;
            *done += batch.len();
            batch.clear();
            if !args.quiet { tracing::info!("reindex (embeddings): {} items", done); }
            Ok(())
        };
        for item_r in store.list()? {
            batch.push(item_r?);
            if batch.len() == 500 { flush(&mut batch, &mut done)?; }
        }
        flush(&mut batch, &mut done)?;
```

If the closure borrows clash with `embedder_idx`, write it as a small `fn flush(idx: &EmbedderIndex, batch: &mut Vec<Item>, done: &mut usize, quiet: bool) -> Result<(), CliError>`.

- [ ] **Step 3: Run, lint, commit**

`cargo test -p singularmem-search --test batch_hook`, `cargo test -p singularmem --test cli reindex` (existing reindex tests must still pass), clippy, fmt.

```bash
git add crates/singularmem-search/src/vector_index.rs crates/singularmem-search/src/lib.rs crates/singularmem-search/tests/batch_hook.rs src/commands/search.rs
git commit -s -m "feat(search): batched embedding in EmbedderIndex and reindex"
```

---

### Task 3: Vector journal codec

**Files:**
- Create: `crates/singularmem-search/src/vector_journal.rs`
- Modify: `crates/singularmem-search/src/lib.rs` (`pub(crate) mod vector_journal;`)
- Test: `crates/singularmem-search/tests/vector_journal.rs` (make the module `pub` with `#[doc(hidden)]` so the integration test can reach it, or put the tests as `#[cfg(test)]` inside the module — prefer the latter to keep the API private)

**Interfaces:**
- Produces:

```rust
pub(crate) const JOURNAL_MAGIC: &[u8; 4] = b"SMVJ";
pub(crate) const JOURNAL_VERSION: u16 = 1;

pub(crate) struct JournalHeader { pub dim: u32, pub model_id: String }

pub(crate) struct Journal { path: PathBuf, header: JournalHeader }

impl Journal {
    /// Open or create `journal.bin`. Creates the header when absent.
    pub(crate) fn open(path: &Path, dim: usize, model_id: &str) -> Result<Self>;
    /// Append records; one write_all then fsync.
    pub(crate) fn append(&self, records: &[(ItemId, Vec<f32>)]) -> Result<()>;
    /// Read every complete record; a trailing partial record is dropped.
    pub(crate) fn replay(&self) -> Result<Vec<(ItemId, Vec<f32>)>>;
    /// Number of complete records currently in the file.
    pub(crate) fn len(&self) -> Result<usize>;
    /// Truncate to the header.
    pub(crate) fn clear(&self) -> Result<()>;
}
```

- [ ] **Step 1: Failing unit tests** (inside `vector_journal.rs`, `#[cfg(test)] mod tests`)

```rust
    fn id(n: u128) -> ItemId { ulid::Ulid::from(n).to_string().parse().unwrap() }

    #[test]
    fn round_trip_records() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(&dir.path().join("journal.bin"), 4, "m@v1").unwrap();
        j.append(&[(id(1), vec![1.0, 2.0, 3.0, 4.0]), (id(2), vec![0.5; 4])]).unwrap();
        j.append(&[(id(3), vec![9.0; 4])]).unwrap();
        assert_eq!(j.len().unwrap(), 3);
        let got = j.replay().unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].0, id(1));
        assert_eq!(got[0].1, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(got[2].1, vec![9.0; 4]);
        j.clear().unwrap();
        assert_eq!(j.len().unwrap(), 0);
        assert!(j.replay().unwrap().is_empty());
    }

    #[test]
    fn truncated_tail_is_dropped_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        let j = Journal::open(&p, 4, "m@v1").unwrap();
        j.append(&[(id(1), vec![1.0; 4]), (id(2), vec![2.0; 4])]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() - 5]).unwrap(); // cut into the last record
        let j = Journal::open(&p, 4, "m@v1").unwrap();
        let got = j.replay().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, id(1));
    }

    #[test]
    fn header_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        Journal::open(&p, 4, "m@v1").unwrap();
        assert!(matches!(Journal::open(&p, 8, "m@v1"), Err(Error::DimMismatch { .. })));
        assert!(matches!(Journal::open(&p, 4, "other@v1"), Err(Error::ModelMismatch { .. })));
    }

    #[test]
    fn bad_magic_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        std::fs::write(&p, b"NOPE\0\0").unwrap();
        assert!(matches!(Journal::open(&p, 4, "m"), Err(Error::IndexCorrupted { .. })));
    }

    #[test]
    fn dim_mismatch_on_append_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(&dir.path().join("journal.bin"), 4, "m").unwrap();
        assert!(matches!(j.append(&[(id(1), vec![1.0; 3])]), Err(Error::DimMismatch { .. })));
    }
```

`ulid` is a dependency of core; add `ulid = { workspace = true }` to the search crate's `[dev-dependencies]` if not present (check `Cargo.toml`). `Error::IndexCorrupted`'s fields are in `crates/singularmem-search/src/error.rs:39`; `ModelMismatch` needs `path, found_model, expected_model`; `DimMismatch` needs `expected, got`.

- [ ] **Step 2: Implement `vector_journal.rs`**

```rust
//! Append-only journal of vectors added since the last compaction of the
//! USearch index. Format documented in `docs/formats/vectors-v2.md`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use singularmem_core::ItemId;

use crate::error::{Error, Result};

pub(crate) const JOURNAL_MAGIC: &[u8; 4] = b"SMVJ";
pub(crate) const JOURNAL_VERSION: u16 = 1;
const ID_BYTES: usize = 16;

#[derive(Debug, Clone)]
pub(crate) struct JournalHeader {
    pub dim: u32,
    pub model_id: String,
}

impl JournalHeader {
    fn encoded(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 2 + 4 + 2 + self.model_id.len());
        out.extend_from_slice(JOURNAL_MAGIC);
        out.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
        out.extend_from_slice(&self.dim.to_le_bytes());
        let id_len = u16::try_from(self.model_id.len()).expect("model id fits u16");
        out.extend_from_slice(&id_len.to_le_bytes());
        out.extend_from_slice(self.model_id.as_bytes());
        out
    }

    fn len(&self) -> u64 { self.encoded().len() as u64 }

    fn decode(bytes: &[u8], path: &Path) -> Result<Self> {
        let corrupt = |reason: &str| Error::IndexCorrupted {
            path: path.to_path_buf(),
            reason: reason.to_string(),
        };
        if bytes.len() < 12 || &bytes[..4] != JOURNAL_MAGIC {
            return Err(corrupt("journal header magic missing"));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != JOURNAL_VERSION {
            return Err(corrupt(&format!("unsupported journal version {version}")));
        }
        let dim = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let id_len = usize::from(u16::from_le_bytes([bytes[10], bytes[11]]));
        let id = bytes.get(12..12 + id_len).ok_or_else(|| corrupt("journal header truncated"))?;
        let model_id = String::from_utf8(id.to_vec()).map_err(|_| corrupt("journal model id is not UTF-8"))?;
        Ok(Self { dim, model_id })
    }
}

#[derive(Debug)]
pub(crate) struct Journal {
    path: PathBuf,
    header: JournalHeader,
}

impl Journal {
    pub(crate) fn open(path: &Path, dim: usize, model_id: &str) -> Result<Self> {
        let dim32 = u32::try_from(dim).map_err(|_| Error::DimMismatch { expected: dim, got: usize::MAX })?;
        let wanted = JournalHeader { dim: dim32, model_id: model_id.to_string() };
        if path.exists() {
            let mut buf = vec![0u8; 12 + 512];
            let mut f = File::open(path).map_err(Error::Io)?;
            let n = f.read(&mut buf).map_err(Error::Io)?;
            let found = JournalHeader::decode(&buf[..n], path)?;
            if found.model_id != wanted.model_id {
                return Err(Error::ModelMismatch {
                    path: path.to_path_buf(),
                    found_model: found.model_id,
                    expected_model: wanted.model_id,
                });
            }
            if found.dim != wanted.dim {
                return Err(Error::DimMismatch { expected: dim, got: found.dim as usize });
            }
            Ok(Self { path: path.to_path_buf(), header: found })
        } else {
            let mut f = File::create(path).map_err(Error::Io)?;
            f.write_all(&wanted.encoded()).map_err(Error::Io)?;
            f.sync_all().map_err(Error::Io)?;
            Ok(Self { path: path.to_path_buf(), header: wanted })
        }
    }

    fn record_len(&self) -> usize { ID_BYTES + self.header.dim as usize * 4 }

    pub(crate) fn append(&self, records: &[(ItemId, Vec<f32>)]) -> Result<()> {
        if records.is_empty() { return Ok(()); }
        let dim = self.header.dim as usize;
        let mut buf = Vec::with_capacity(records.len() * self.record_len());
        for (id, v) in records {
            if v.len() != dim {
                return Err(Error::DimMismatch { expected: dim, got: v.len() });
            }
            buf.extend_from_slice(&id.to_bytes());
            for x in v { buf.extend_from_slice(&x.to_le_bytes()); }
        }
        let mut f = OpenOptions::new().append(true).open(&self.path).map_err(Error::Io)?;
        f.write_all(&buf).map_err(Error::Io)?;
        f.sync_all().map_err(Error::Io)
    }

    pub(crate) fn replay(&self) -> Result<Vec<(ItemId, Vec<f32>)>> {
        let mut f = File::open(&self.path).map_err(Error::Io)?;
        f.seek(SeekFrom::Start(self.header.len())).map_err(Error::Io)?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).map_err(Error::Io)?;
        let rl = self.record_len();
        let complete = bytes.len() / rl;
        if bytes.len() % rl != 0 {
            tracing::debug!(path = %self.path.display(), "dropping partial trailing journal record");
        }
        let dim = self.header.dim as usize;
        let mut out = Vec::with_capacity(complete);
        for rec in bytes[..complete * rl].chunks_exact(rl) {
            let mut idb = [0u8; ID_BYTES];
            idb.copy_from_slice(&rec[..ID_BYTES]);
            let id = ItemId::from_bytes(idb);
            let v: Vec<f32> = rec[ID_BYTES..].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            debug_assert_eq!(v.len(), dim);
            out.push((id, v));
        }
        Ok(out)
    }

    pub(crate) fn len(&self) -> Result<usize> {
        let size = std::fs::metadata(&self.path).map_err(Error::Io)?.len();
        Ok(((size.saturating_sub(self.header.len())) as usize) / self.record_len())
    }

    pub(crate) fn clear(&self) -> Result<()> {
        let f = OpenOptions::new().write(true).open(&self.path).map_err(Error::Io)?;
        f.set_len(self.header.len()).map_err(Error::Io)?;
        f.sync_all().map_err(Error::Io)
    }
}
```

`ItemId::to_bytes()` / `ItemId::from_bytes([u8; 16])`: check `crates/singularmem-core/src/id.rs` for the `ulid_id!` macro's API; if only `Ulid` access exists, add `pub fn to_bytes(self) -> [u8; 16]` and `pub fn from_bytes([u8; 16]) -> Self` to the macro (big-endian, via `Ulid::to_bytes`/`Ulid::from_bytes`) in core and note it in the report. Pedantic casts (`as usize`, `as u64`) need `#[allow(clippy::cast_possible_truncation)]` with a reason comment or `usize::try_from`; prefer `try_from` where cheap.

- [ ] **Step 3: Run, lint, commit**

`cargo test -p singularmem-search vector_journal`, clippy, fmt.

```bash
git add crates/singularmem-search/src/vector_journal.rs crates/singularmem-search/src/lib.rs crates/singularmem-search/Cargo.toml crates/singularmem-core/src/id.rs
git commit -s -m "feat(search): append-only vector journal codec"
```

---

### Task 4: Journal-backed commit, compaction, replay, and locking in `VectorIndex`

**Files:**
- Modify: `crates/singularmem-search/src/vector_index.rs` (struct fields, `open_with_options`, `add`, `save` → `commit`, `EmbedderIndex` hook impl)
- Modify: `crates/singularmem-search/Cargo.toml` (`fs4 = { version = "0.8", features = ["sync"] }`)
- Test: `crates/singularmem-search/tests/vector_index_journal.rs`, `crates/singularmem-search/tests/vector_index_concurrency.rs`

**Interfaces:**
- Consumes: Task 3 `Journal`; Task 2's batch method.
- Produces: `pub const COMPACT_THRESHOLD: usize = 1_000`; `VectorIndex::commit(&self, end_of_batch: bool) -> Result<()>` (journal append + conditional compaction); `VectorIndex::compact(&self) -> Result<()>` (public for tests and `reindex`); `VectorIndex::journal_len(&self) -> Result<usize>`; `VectorIndex::save` kept as an alias for `compact` (deprecated doc note) so existing callers compile.

Design inside `VectorIndex`:
- new fields: `journal: Journal`, `pending: Mutex<Vec<(ItemId, Vec<f32>)>>`, `batch_end: AtomicBool`, `lock_path: PathBuf`.
- `add` pushes `(id, vector.to_vec())` onto `pending` after inserting into USearch (in-memory index is current immediately).
- `commit(end_of_batch)`: take the file lock (see below); drain `pending` → `journal.append`; if `journal.len() > COMPACT_THRESHOLD || end_of_batch` → `compact_locked()`; release.
- `compact_locked()`: `inner.save(tmp_usearch)`, write keymap to `tmp_keymap`, `fsync` both, `rename` both over the originals, `journal.clear()`. Empty index: remove `index.usearch` as today.
- `open_with_options`: after loading index + keymap, `Journal::open(dir/journal.bin, meta.dim, &meta.model_id)`, `replay()`, and for each record whose id is not in `keymap.reverse`, add via the same path as `add` but WITHOUT pushing to `pending` (they are already journaled). If `meta.format_version == "1"`, keep it in memory as `"2"` and rewrite `.meta.json` on the first `commit` (inside the lock).
- Lock: `fs4::fs_std::FileExt::try_lock_exclusive` on `File::create(lock_path)`; on `WouldBlock`, sleep `50ms << attempt` up to 5 attempts, then `Error::Usearch { context: "acquiring vector index lock", reason: "busy after 5 attempts" }`. Verify the exact fs4 0.8 API on docs.rs (`fs4::fs_std::FileExt` with the `sync` feature; method names `try_lock_exclusive`/`unlock`); if the module path differs, adapt and note it.
- `EmbedderIndex`: `on_ingest_batch` sets `batch_end = true` after adding; `commit()` calls `vector_index.commit(batch_end.swap(false))`. `on_ingest` (single) leaves the flag false, so a single ingest appends only.

- [ ] **Step 1: Failing tests** — `crates/singularmem-search/tests/vector_index_journal.rs`

```rust
use singularmem_core::{IndexHook, NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, VectorIndex, COMPACT_THRESHOLD};

fn open(dir: &std::path::Path) -> EmbedderIndex {
    EmbedderIndex::open(dir.join("v"), Box::new(MockEmbedder::default())).unwrap()
}

#[test]
fn single_ingest_appends_to_journal_and_does_not_rewrite_index() {
    let dir = tempfile::tempdir().unwrap();
    // Seed 200 items in one batch -> compaction at batch end, journal empty.
    {
        let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
        store.ingest_many((0..200).map(|i| NewItem::text(format!("seed {i}")))).unwrap();
    }
    let usearch = dir.path().join("v/index.usearch");
    let journal = dir.path().join("v/journal.bin");
    let before = std::fs::metadata(&usearch).unwrap().modified().unwrap();
    let jlen_before = std::fs::metadata(&journal).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(20));
    {
        let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
        store.ingest(NewItem::text("one more".into())).unwrap();
    }
    assert_eq!(std::fs::metadata(&usearch).unwrap().modified().unwrap(), before, "index.usearch must not be rewritten by a single ingest");
    assert!(std::fs::metadata(&journal).unwrap().len() > jlen_before, "journal grew");
    // And the item is searchable after reopen (journal replay).
    let idx = open(dir.path());
    let v = MockEmbedder::default().embed_for_test("one more");
    let hits = idx.vector_index().search(&v, 1).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn compaction_fires_at_threshold_and_at_batch_end() {
    let dir = tempfile::tempdir().unwrap();
    let idx = open(dir.path());
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(idx)).unwrap();
    for i in 0..COMPACT_THRESHOLD {
        store.ingest(NewItem::text(format!("s {i}"))).unwrap();
    }
    let journal = dir.path().join("v/journal.bin");
    let idx_probe = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx_probe.journal_len().unwrap(), COMPACT_THRESHOLD, "at the threshold, not yet compacted");
    drop(idx_probe);
    store.ingest(NewItem::text("over".into())).unwrap();
    let idx_probe = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx_probe.journal_len().unwrap(), 0, "one past the threshold compacts");
    drop(idx_probe);
    store.ingest(NewItem::text("after".into())).unwrap();
    store.ingest_many(vec![NewItem::text("b1".into()), NewItem::text("b2".into())]).unwrap();
    let idx_probe = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx_probe.journal_len().unwrap(), 0, "batch end compacts");
    let _ = journal;
}

#[test]
fn replay_skips_ids_already_in_keymap_and_search_is_identical_before_and_after_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
    let items = store.ingest_many((0..50).map(|i| NewItem::text(format!("corpus {i}")))).unwrap();
    for i in 0..30 { store.ingest(NewItem::text(format!("extra {i}"))).unwrap(); }
    drop(store);
    let q = MockEmbedder::default().embed_for_test("corpus 7");
    let before = open(dir.path()).vector_index().search(&q, 5).unwrap();
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    idx.compact().unwrap();
    assert_eq!(idx.journal_len().unwrap(), 0);
    drop(idx);
    let after = open(dir.path()).vector_index().search(&q, 5).unwrap();
    assert_eq!(before.iter().map(|h| h.id).collect::<Vec<_>>(), after.iter().map(|h| h.id).collect::<Vec<_>>());
    assert!(after.iter().any(|h| h.id == items[7].id));
    // Reopen again: replay of an empty journal, no duplicates.
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx.len(), 80);
}

#[test]
fn v1_directory_opens_and_becomes_v2_on_first_commit() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
        store.ingest_many(vec![NewItem::text("a".into())]).unwrap();
    }
    // Simulate v0.20.0 layout: no journal, format_version "1".
    std::fs::remove_file(dir.path().join("v/journal.bin")).unwrap();
    let meta_path = dir.path().join("v/.meta.json");
    let meta = std::fs::read_to_string(&meta_path).unwrap().replace("\"format_version\": \"2\"", "\"format_version\": \"1\"");
    std::fs::write(&meta_path, meta).unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
    store.ingest(NewItem::text("b".into())).unwrap();
    assert!(std::fs::read_to_string(&meta_path).unwrap().contains("\"format_version\": \"2\""));
    assert!(dir.path().join("v/journal.bin").exists());
}

#[test]
fn crash_between_rename_and_truncate_is_idempotent_on_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
    for i in 0..10 { store.ingest(NewItem::text(format!("x {i}"))).unwrap(); }
    drop(store);
    // Compact, then put the journal records back as if truncate never happened.
    let journal_bytes = std::fs::read(dir.path().join("v/journal.bin")).unwrap();
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    idx.compact().unwrap();
    drop(idx);
    std::fs::write(dir.path().join("v/journal.bin"), journal_bytes).unwrap();
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx.len(), 10, "replayed ids already in the keymap are skipped");
}
```

`MockEmbedder::embed_for_test` does not exist; use `Embedder::embed(&MockEmbedder::default(), "…")` with the trait in scope. `VectorIndex::len()` — add `pub fn len(&self) -> usize` returning `inner.size()` if absent (and `is_empty` to satisfy clippy). `format_version` JSON spacing must match `serde_json::to_string_pretty` output (`"format_version": "2"`).

`crates/singularmem-search/tests/vector_index_concurrency.rs`:

```rust
use std::thread;
use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, VectorIndex};

#[test]
fn concurrent_single_ingests_from_two_handles_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join("s.db");
    let vdir = dir.path().join("v");
    let mut handles = Vec::new();
    for t in 0..2 {
        let (sp, vd) = (store_path.clone(), vdir.clone());
        handles.push(thread::spawn(move || {
            let idx = EmbedderIndex::open(&vd, Box::new(MockEmbedder::default())).unwrap();
            let store = Store::open_with_hook(&sp, Box::new(idx)).unwrap();
            for i in 0..40 { store.ingest(NewItem::text(format!("t{t} item {i}"))).unwrap(); }
        }));
    }
    for h in handles { h.join().unwrap(); }
    let idx = VectorIndex::open(&vdir, &MockEmbedder::default()).unwrap();
    assert_eq!(idx.len(), 80, "every vector from both writers is present after replay");
}
```

Two handles in one process share the OS file lock semantics per file description; that is what the advisory lock serialises. Note: two `EmbedderIndex` instances each hold their own in-memory USearch; the second writer's compaction could drop the first's journal-only vectors if it compacts from a stale in-memory view. Design rule to implement: **compaction is only performed by a handle that replayed the journal under the lock immediately before saving** — i.e. `compact_locked` first replays any journal records not in its keymap (same skip-by-id rule as open), then saves. Add that to the implementation and to `docs/formats/vectors-v2.md`.

Run: `cargo test -p singularmem-search --test vector_index_journal --test vector_index_concurrency` — Expected: compile errors (`COMPACT_THRESHOLD`, `journal_len`, `compact`).

- [ ] **Step 2: Implement** per the design list above. Keep `save()` as `pub fn save(&self) -> Result<()> { self.compact() }` with a doc note "compat alias; prefer `commit`/`compact`". `EmbedderIndex::commit` → `self.vector_index.commit(self.batch_end.swap(false, Ordering::SeqCst))`.

- [ ] **Step 3: Run everything, lint, commit**

`cargo test -p singularmem-search`, `cargo test -p singularmem` (CLI reindex + hooks tests), `cargo test -p singularmem-mcp` (ingest tool), clippy, fmt.

```bash
git add crates/singularmem-search/src/vector_index.rs crates/singularmem-search/Cargo.toml Cargo.lock crates/singularmem-search/tests/vector_index_journal.rs crates/singularmem-search/tests/vector_index_concurrency.rs
git commit -s -m "feat(search): journal-backed vector commits with threshold compaction and file lock"
```

---

### Task 5: Benches, perf gates, docs, real-model numbers

**Files:**
- Modify: `crates/singularmem-search/benches/search_perf.rs` (+ `bench_ingest_with_indexes`, `bench_ingest_single_with_indexes`, register in `criterion_group!`)
- Modify: `.github/scripts/perf-check.sh` (two gates after the existing ingest gate)
- Create: `docs/formats/vectors-v2.md`, `docs/benchmarks/ingest.md`
- Modify: `README.md` (search section, one line), `docs/hooks.md` (note), spec Deviations

- [ ] **Step 1: Benches**

```rust
fn realistic(i: usize) -> String {
    format!("assistant: {i} ") + &"We discussed the migration plan for the store format and agreed to keep append-only revisions; the reviewer asked for a doc-count guard after ingest. ".repeat(9)
}

fn bench_ingest_with_indexes(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_throughput");
    group.throughput(criterion::Throughput::Elements(100));
    group.bench_function("ingest_with_indexes", |b| {
        b.iter_batched(
            || {
                let dir = TempDir::new().unwrap();
                let lex = Index::open(dir.path().join("lex")).unwrap();
                let sem = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
                let multi = singularmem_core::hook::MultiHook::new(vec![Box::new(lex), Box::new(sem)]);
                let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(multi)).unwrap();
                (dir, store)
            },
            |(_dir, store)| {
                store.ingest_many((0..100).map(|i| NewItem::text(realistic(i)))).unwrap();
            },
            criterion::BatchSize::PerIteration,
        );
    });
    group.finish();
}

fn bench_ingest_single_with_indexes(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    {
        let sem = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(sem)).unwrap();
        for chunk in (0..20_000).collect::<Vec<_>>().chunks(500) {
            store.ingest_many(chunk.iter().map(|i| NewItem::text(format!("seed {i}")))).unwrap();
        }
    }
    let lex = Index::open(dir.path().join("lex")).unwrap();
    let sem = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let multi = singularmem_core::hook::MultiHook::new(vec![Box::new(lex), Box::new(sem)]);
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(multi)).unwrap();
    let mut group = c.benchmark_group("ingest_throughput");
    let mut n = 0usize;
    group.bench_function("ingest_single_with_indexes", |b| {
        b.iter(|| { n += 1; store.ingest(NewItem::text(realistic(n))).unwrap(); });
    });
    group.finish();
}
```

Note the single bench crosses the compaction threshold every 1,000 iterations; that is intended (the gate is a median, and the amortised cost is what users see).

- [ ] **Step 2: Perf gates** in `.github/scripts/perf-check.sh`, after the existing `ingest_one` block:

```bash
WITH_IDX_NS=$(read_median_ns "ingest_throughput/ingest_with_indexes")
WITH_IDX_RATE=$(awk -v ns="$WITH_IDX_NS" 'BEGIN { printf "%.2f", 100 * 1e9 / ns }')
if awk -v v="$WITH_IDX_RATE" 'BEGIN { exit !(v < 50) }'; then
    echo "FAIL: ingest with indexes $WITH_IDX_RATE items/s below 50 items/s" >&2
    exit 15
fi
SINGLE_NS=$(read_median_ns "ingest_throughput/ingest_single_with_indexes")
SINGLE_MS=$(awk -v ns="$SINGLE_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$SINGLE_MS" 'BEGIN { exit !(v > 20) }'; then
    echo "FAIL: single ingest with indexes ${SINGLE_MS} ms exceeds 20 ms" >&2
    exit 16
fi
echo "ingest with indexes: $WITH_IDX_RATE items/s; single ingest: $SINGLE_MS ms"
```

(The 100-item batch is one iteration, hence `100 * 1e9 / ns`.) Run the script locally once (`.github/scripts/perf-check.sh`; on macOS `stat -c` is GNU-only — run the bench part manually if the script's `stat` fails, and note it).

- [ ] **Step 3: Real-model numbers** — `docs/benchmarks/ingest.md`

Run before (checkout `v0.20.0` in a temporary worktree: `git worktree add ../sm-v0.20 v0.20.0`) and after (this branch) with the real model, using the LongMemEval harness's ingest rate as the end-to-end figure (`cargo run --release -p singularmem-bench -- longmemeval <file> --limit 50 --seed 1` prints `ingest N items/s` in its header) and a small ad-hoc example for the three text lengths (short / realistic / long) through `Store::ingest_many` with `FastembedEmbedder::new()`. Record commit, CPU (Apple M2 Max), model id, and the numbers in a table; state the constitution floor and the CI-gated mock figure alongside. Remove the worktree afterwards.

- [ ] **Step 4: Docs**

`docs/formats/vectors-v2.md`: directory listing, `.meta.json` fields, `keymap.bin` (bincode, note the struct), `journal.bin` byte layout with a worked example, replay rules (skip ids in keymap; drop partial tail), commit and compaction rules (threshold 1,000; batch end; compaction replays first), lock file, crash guarantees, v1 → v2 upgrade, and "what a third-party loader must do". README search section: one sentence on the journal and what it means for hook-driven saves. `docs/hooks.md`: note that stop/session-end saves append to the journal and no longer rewrite the index. Spec Deviations: anything changed (e.g. the "compaction replays first" rule, `save()` alias, `ItemId::{to_bytes,from_bytes}` if added).

- [ ] **Step 5: Verify and commit**

`cargo test --workspace --all-targets`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, `cargo bench -p singularmem-search --bench search_perf -- ingest_ --warm-up-time 1 --measurement-time 5` (numbers in the report).

```bash
git add crates/singularmem-search/benches/search_perf.rs .github/scripts/perf-check.sh docs/formats/vectors-v2.md docs/benchmarks/ingest.md README.md docs/hooks.md docs/superpowers/specs/2026-09-06-ingest-throughput-17-design.md
git commit -s -m "perf(ci): gate ingest with indexes; document vectors v2 and ingest numbers"
```

---

## Self-review

- Spec coverage: Part 1 → Tasks 1–2; Part 2 (journal, commit, open, concurrency, crash safety, v1 upgrade, reindex) → Tasks 3–4; Part 3 (benches, gates, docs) → Task 5; error table → Tasks 3–4 tests; acceptance criteria 1–2 → Task 5 gates, 3 → Task 5 docs, 4 → Task 4 tests, 5 → Task 5 verification.
- Type consistency: `Journal::{open, append, replay, len, clear}` used identically in Tasks 3–4; `EMBED_CHUNK` (Task 2) and `COMPACT_THRESHOLD`, `commit(end_of_batch)`, `compact`, `journal_len`, `len` (Task 4) used by tests and benches under those names.
- Known uncertainties flagged in-task: fs4 0.8 API path; `ItemId` byte accessors; `stat -c` on macOS; borrow shape of the reindex flush closure. Each has a fallback and a report note.
- One design rule added beyond the spec text and to be recorded in Deviations: compaction replays the journal under the lock before saving, so two writer handles cannot drop each other's journal-only vectors.
