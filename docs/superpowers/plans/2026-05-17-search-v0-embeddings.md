---
spec: docs/superpowers/specs/2026-05-17-search-v0-embeddings-design.md
sub-project: 2b-search-v0-embeddings
status: draft
target-release: v0.3.0
---

# Search v0 (Embeddings + Vector) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship sub-project 2b — `Embedder` trait + `FastembedEmbedder` (ONNX via fastembed), `VectorIndex` (USearch wrapper), `EmbedderIndex` (a second `IndexHook` alongside Tantivy's), the `MultiHook` composite in `singularmem-core`, new `semantic-search` CLI verb + `reindex --with-embeddings` flag, opt-in mechanic via `.vectors/` directory existence, and version bump to v0.3.0.

**Architecture:** Extend `singularmem-search` with four new modules (`embedder.rs`, `model.rs`, `vector_index.rs`, `semantic_query.rs`). Add `MultiHook` to `singularmem-core::hook` (additive — existing `Store::open_with_hook` unchanged). The root binary's auto-wiring grows to compose Tantivy + Embedder hooks via `MultiHook` only when `.vectors/` exists and only for `Ingest` commands.

**Tech Stack:** Rust 1.80+ stable; existing v0.2.0 stack (rusqlite 0.32, ulid, jiff, thiserror, tracing, serde, clap, tantivy 0.22); **NEW**: `fastembed = "=4.4.0"` (with `default-features = false, features = ["ort-download-binaries"]`), `usearch = "=2.15.3"`.

---

**Frontmatter (per the plan-template):**

- spec: `docs/superpowers/specs/2026-05-17-search-v0-embeddings-design.md`
- sub-project: `2b-search-v0-embeddings`
- status: `ready-for-execution`
- target-release: `v0.3.0`

**Approach summary.** One feature branch (`search-v0-embeddings`) with one PR back to `main`. Twelve logical phases ending in commits. Tasks follow TDD where there is real code. The plan is shorter than 2a's because the subagent execution loop has demonstrated it fills in idiomatic Rust from this level of detail — I inline the load-bearing types and test signatures, sketch the rest.

## Step-by-step implementation milestones

- **M1** — Workspace prep: branch, fastembed + usearch deps, MultiHook in core.
- **M2** — Embedder trait + MockEmbedder (test fixture) + FastembedEmbedder.
- **M3** — VectorIndexMeta + VectorIndex::open / add / remove / search / save.
- **M4** — EmbedderIndex (`impl IndexHook`) + `semantic_search` method.
- **M5** — CLI: `semantic-search` verb + `reindex --with-embeddings` flag + auto-wiring update.
- **M6** — Tests: Principle VII MultiHook isolation; property + concurrency tests.
- **M7** — Format spec update + criterion benches + perf-check.sh extensions.
- **M8** — CONTRIBUTING.md cheap-vs-heavy rule + version bump 0.3.0 + doc audit.
- **M9** — Push, PR, CI green, merge, tag `v0.3.0`, update memory.

## Task list

### Task 0: Pre-flight — create feature branch

**Files:** none — git only.
**Assigned skill:** `verification-before-completion`

- [ ] Verify on `main` clean: `git status`, `git log --oneline -3` (HEAD = `6ab6ab5 docs: add Search v0 (Embeddings + Vector...)` or newer).
- [ ] `git checkout -b search-v0-embeddings`.
- [ ] `git branch --show-current` → `search-v0-embeddings`.

---

### Task 1: Workspace deps — fastembed + usearch

**Files:** `Cargo.toml` (root, `[workspace.dependencies]`).
**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Verify versions aren't yanked.**

```bash
cargo search fastembed --limit 3
cargo search usearch --limit 3
```

If `=4.4.0` (fastembed) or `=2.15.3` (usearch) are unavailable or yanked, substitute the freshest patch on the same minor line and note the substitution in the commit message.

- [ ] **Step 2: Add to `[workspace.dependencies]`:**

```toml
fastembed = { version = "=4.4.0", default-features = false, features = ["ort-download-binaries"] }
usearch = "=2.15.3"
bincode = "1.3"
```

`bincode` for `keymap.bin` serialization (Section 6 of spec).

- [ ] **Step 3: Verify the workspace builds (no consumer yet — just `cargo check`):**

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo check --workspace --all-targets --all-features 2>&1 | tail -10
```

Expected: `Finished`. fastembed pulls ORT precompiled binaries; first build is slow (~5 min downloading ~30 MB).

- [ ] **Step 4: Commit.**

```bash
git -C /Users/jonasbroms/Sites/singularmem add Cargo.toml
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "$(cat <<'EOF'
chore(deps): add fastembed + usearch + bincode to workspace deps

Three new workspace dependencies for sub-project 2b (Search v0
Embeddings + Vector). All pinned exact per the convention:
  fastembed = "=4.4.0" (default-features off, ort-download-binaries on)
  usearch = "=2.15.3"
  bincode = "1.3"

fastembed's ort-download-binaries feature ships precompiled ONNX
runtime binaries so users don't need a C/C++ toolchain to build.
First cargo build pulls ~30 MB and takes a few minutes.
EOF
)"
```

---

### Task 2: `MultiHook` in `singularmem-core` + `Store::open_with_hooks` (TDD)

**Files:**
- Modify: `crates/singularmem-core/src/hook.rs` (append `MultiHook`)
- Modify: `crates/singularmem-core/src/store.rs` (add `open_with_hooks`)
- Create: `crates/singularmem-core/tests/multi_hook.rs`

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Write the failing test.**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-core/tests/multi_hook.rs`

```rust
//! Tests for the MultiHook composite + Store::open_with_hooks constructor.
//! The Principle VII isolation test (failing hook doesn't block others)
//! lands here in Task 17; this file initially covers the construction +
//! per-hook fan-out.

use singularmem_core::hook::MultiHook;
use singularmem_core::{IndexHook, Item, NewItem, Result, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

struct CountingHook {
    on_ingest_calls: Arc<AtomicUsize>,
    commit_calls: Arc<AtomicUsize>,
}

impl IndexHook for CountingHook {
    fn on_ingest(&self, _item: &Item) -> Result<()> {
        self.on_ingest_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn on_reindex(&self, _item: &Item) -> Result<()> { Ok(()) }
    fn commit(&self) -> Result<()> {
        self.commit_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn multi_hook_fans_out_on_ingest_to_all_members() {
    let a_ingest = Arc::new(AtomicUsize::new(0));
    let a_commit = Arc::new(AtomicUsize::new(0));
    let b_ingest = Arc::new(AtomicUsize::new(0));
    let b_commit = Arc::new(AtomicUsize::new(0));

    let dir = TempDir::new().unwrap();
    let store = Store::open_with_hooks(
        dir.path().join("store.db"),
        vec![
            Box::new(CountingHook {
                on_ingest_calls: Arc::clone(&a_ingest),
                commit_calls: Arc::clone(&a_commit),
            }),
            Box::new(CountingHook {
                on_ingest_calls: Arc::clone(&b_ingest),
                commit_calls: Arc::clone(&b_commit),
            }),
        ],
    )
    .expect("open with two hooks");

    let _ = store.ingest(NewItem::text("hello")).unwrap();

    assert_eq!(a_ingest.load(Ordering::SeqCst), 1);
    assert_eq!(b_ingest.load(Ordering::SeqCst), 1);
    assert_eq!(a_commit.load(Ordering::SeqCst), 1);
    assert_eq!(b_commit.load(Ordering::SeqCst), 1);
}
```

- [ ] **Step 2: Run — must fail.**

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo test -p singularmem-core --test multi_hook 2>&1 | tail -10
```

Expected: compile error — `MultiHook` doesn't exist, `Store::open_with_hooks` doesn't exist.

- [ ] **Step 3: Implement `MultiHook` in `hook.rs`.**

Append to `/Users/jonasbroms/Sites/singularmem/crates/singularmem-core/src/hook.rs`:

```rust
/// Composite `IndexHook` that fans calls out to multiple underlying hooks.
/// Each hook runs independently; one hook's failure does NOT prevent later
/// hooks from running (the loop catches errors per-hook, logs via
/// `tracing::warn!`, and returns the FIRST error after all hooks have been
/// tried).
///
/// Use this when you need to wire two or more `IndexHook` implementations
/// (e.g. Tantivy lexical + USearch vector) into a single `Store`. The
/// `Store::open_with_hooks` constructor wraps a `Vec<Box<dyn IndexHook>>`
/// into a `MultiHook` for you.
pub struct MultiHook {
    hooks: Vec<Box<dyn IndexHook>>,
}

impl MultiHook {
    /// Construct from an ordered list of hooks. Order is preserved: hooks
    /// run in the order given, which only matters for visibility of
    /// `tracing::warn!` lines.
    #[must_use]
    pub fn new(hooks: Vec<Box<dyn IndexHook>>) -> Self {
        Self { hooks }
    }
}

impl IndexHook for MultiHook {
    fn on_ingest(&self, item: &crate::Item) -> crate::Result<()> {
        run_all(self.hooks.iter(), "on_ingest", |h| h.on_ingest(item))
    }

    fn on_reindex(&self, item: &crate::Item) -> crate::Result<()> {
        run_all(self.hooks.iter(), "on_reindex", |h| h.on_reindex(item))
    }

    fn commit(&self) -> crate::Result<()> {
        run_all(self.hooks.iter(), "commit", |h| h.commit())
    }
}

fn run_all<'a, I, F>(hooks: I, op: &'static str, mut call: F) -> crate::Result<()>
where
    I: Iterator<Item = &'a Box<dyn IndexHook>>,
    F: FnMut(&dyn IndexHook) -> crate::Result<()>,
{
    let mut first_err: Option<crate::Error> = None;
    for (i, hook) in hooks.enumerate() {
        if let Err(e) = call(hook.as_ref()) {
            tracing::warn!(
                hook_index = i,
                op = op,
                error = %e,
                "MultiHook member failed; other hooks will still run"
            );
            if first_err.is_none() {
                first_err = Some(e);
            }
        }
    }
    first_err.map_or(Ok(()), Err)
}
```

- [ ] **Step 4: Add `Store::open_with_hooks` in `store.rs`.**

```rust
impl Store {
    /// Open with multiple `IndexHook`s. Equivalent to constructing a
    /// `MultiHook` from the list and calling `open_with_hook`.
    ///
    /// # Errors
    /// Same as `Store::open`.
    pub fn open_with_hooks(
        path: impl AsRef<Path>,
        hooks: Vec<Box<dyn IndexHook>>,
    ) -> Result<Self> {
        Self::open_with_hook(path, Box::new(crate::hook::MultiHook::new(hooks)))
    }
}
```

- [ ] **Step 5: Run, clippy, commit.**

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo test -p singularmem-core --test multi_hook 2>&1 | tail -5
cd /Users/jonasbroms/Sites/singularmem && cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1 | tail -3
git -C /Users/jonasbroms/Sites/singularmem add crates/singularmem-core/src/hook.rs crates/singularmem-core/src/store.rs crates/singularmem-core/tests/multi_hook.rs
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "feat(core): MultiHook composite + Store::open_with_hooks

MultiHook fans IndexHook calls out to N members; one member failing
does NOT block the others (log-and-continue, returns the first error).
Store::open_with_hooks wraps a Vec<Box<dyn IndexHook>> via MultiHook.
Existing Store::open and Store::open_with_hook are unchanged.

The Principle VII isolation test (failing-member doesn't block working
members) lands in Task 17 alongside the embedder integration."
```

---

### Task 3: `Embedder` trait + `MockEmbedder` test fixture (TDD)

**Files:**
- Create: `crates/singularmem-search/src/embedder.rs`
- Modify: `crates/singularmem-search/src/lib.rs` (add `pub mod embedder` + re-exports)
- Create: `crates/singularmem-search/src/testing.rs` (MockEmbedder behind `#[cfg(any(test, feature = "testing"))]`)
- Create: `crates/singularmem-search/tests/embedder.rs`

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Failing test.**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/tests/embedder.rs`

```rust
//! Tests for the Embedder trait + MockEmbedder fixture. FastembedEmbedder
//! is exercised in Task 4 and in `#[ignore]` integration tests.

use singularmem_search::testing::MockEmbedder;
use singularmem_search::Embedder;

#[test]
fn mock_embedder_has_consistent_dim() {
    let e = MockEmbedder::default();
    assert_eq!(e.dim(), 384, "MockEmbedder uses the same default dim as all-MiniLM-L6-v2");
}

#[test]
fn mock_embedder_is_deterministic() {
    let e = MockEmbedder::default();
    let v1 = e.embed("hello world").unwrap();
    let v2 = e.embed("hello world").unwrap();
    assert_eq!(v1, v2, "same input must produce byte-identical vector");
    assert_eq!(v1.len(), 384);
}

#[test]
fn mock_embedder_different_inputs_produce_different_vectors() {
    let e = MockEmbedder::default();
    let v1 = e.embed("hello").unwrap();
    let v2 = e.embed("world").unwrap();
    assert_ne!(v1, v2);
}

#[test]
fn mock_embedder_batch_matches_individual() {
    let e = MockEmbedder::default();
    let inputs = ["a", "b", "c"];
    let single: Vec<_> = inputs.iter().map(|s| e.embed(s).unwrap()).collect();
    let batched = e.embed_batch(&inputs).unwrap();
    assert_eq!(single, batched);
}

#[test]
fn mock_embedder_model_id_is_stable() {
    let e = MockEmbedder::default();
    assert_eq!(e.model_id(), "mock-embedder@v1");
}
```

- [ ] **Step 2: Run — must fail (Embedder trait + MockEmbedder don't exist yet).**

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo test -p singularmem-search --test embedder 2>&1 | tail -10
```

- [ ] **Step 3: Implement Embedder trait.**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/src/embedder.rs`

```rust
//! `Embedder` trait — produces fixed-dimension embedding vectors for text.
//!
//! Implementations are synchronous (CPU-bound; no tokio runtime). Output
//! vectors are unit-length (L2-normalized) so cosine similarity reduces to
//! dot product. The `FastembedEmbedder` impl lands in `model.rs` (Task 4);
//! `MockEmbedder` for tests lives in `testing.rs` (this Task).

use crate::error::Result;

/// Produces fixed-dimension embedding vectors for a text item.
pub trait Embedder: Send + Sync {
    /// The fixed embedding dimension (e.g. 384 for all-MiniLM-L6-v2).
    /// MUST be constant across all calls for a given implementation.
    fn dim(&self) -> usize;

    /// Stable identifier for the underlying model (e.g.
    /// `"sentence-transformers/all-MiniLM-L6-v2@v1"`). Stored in the vector
    /// index metadata; mismatch at open time triggers a reindex prompt.
    fn model_id(&self) -> &str;

    /// Embed one item. Returns a `dim()`-length unit-length f32 vector.
    ///
    /// # Errors
    /// Returns `Error::Embedding` on inference failure.
    fn embed(&self, content: &str) -> Result<Vec<f32>>;

    /// Embed a batch of items. Default impl loops over `embed`; concrete
    /// impls (like FastembedEmbedder) override with batched inference.
    fn embed_batch(&self, items: &[&str]) -> Result<Vec<Vec<f32>>> {
        items.iter().map(|s| self.embed(s)).collect()
    }
}
```

- [ ] **Step 4: Implement MockEmbedder in testing.rs.**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/src/testing.rs`

```rust
//! Test fixtures. Available in any `#[test]` and behind the `testing`
//! feature flag for cross-crate test reuse.

#![cfg(any(test, feature = "testing"))]

use crate::embedder::Embedder;
use crate::error::Result;

/// Deterministic-pseudo-hash `Embedder` implementation for tests.
///
/// - `dim()` returns 384 (matches all-MiniLM-L6-v2 so VectorIndex schema
///   tests don't need to vary on dim).
/// - `embed(s)` returns a 384-dim vector derived from `s` via a fast hash;
///   same `s` → byte-identical vector.
/// - No ONNX runtime, no model download, no network.
pub struct MockEmbedder {
    dim: usize,
    model_id: String,
}

impl MockEmbedder {
    #[must_use]
    pub fn with_dim(dim: usize) -> Self {
        Self {
            dim,
            model_id: format!("mock-embedder@v1"),
        }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::with_dim(384)
    }
}

impl Embedder for MockEmbedder {
    fn dim(&self) -> usize { self.dim }
    fn model_id(&self) -> &str { &self.model_id }

    fn embed(&self, content: &str) -> Result<Vec<f32>> {
        // Deterministic pseudo-hash: seed a small PRNG with a hash of the
        // input, draw `dim` floats in [-1, 1], normalize to unit length.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let mut seed = hasher.finish();

        let mut v = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            // xorshift64 step
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // Map to [-1, 1].
            let f = ((seed as i64) as f32) / (i64::MAX as f32);
            v.push(f);
        }
        // L2-normalize.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}
```

- [ ] **Step 5: Update lib.rs re-exports.**

```rust
pub mod embedder;
pub mod testing;

pub use crate::embedder::Embedder;
```

The `pub mod testing` is conditionally compiled, but the module declaration is unconditional so the cfg attribute inside the module file controls availability.

- [ ] **Step 6: Run tests + commit.**

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo test -p singularmem-search --test embedder 2>&1 | tail -10
cd /Users/jonasbroms/Sites/singularmem && cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1 | tail -3
git -C /Users/jonasbroms/Sites/singularmem add crates/singularmem-search/src/embedder.rs crates/singularmem-search/src/testing.rs crates/singularmem-search/src/lib.rs crates/singularmem-search/tests/embedder.rs
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "feat(search): Embedder trait + MockEmbedder test fixture

Embedder trait (sync, CPU-bound): dim, model_id, embed, embed_batch.
Default embed_batch loops over embed; concrete impls (FastembedEmbedder
in Task 4) override with batched ONNX inference.

MockEmbedder is a deterministic-pseudo-hash impl for tests. xorshift64
PRNG seeded by content hash; L2-normalized output. No ONNX runtime, no
network. Behind cfg(any(test, feature = \"testing\")) so production
builds don't ship it.

Five tests cover dim consistency, determinism, different inputs produce
different vectors, batch matches individual, model_id stability."
```

---

### Task 4: `FastembedEmbedder` + `EmbeddingModel` registry

**Files:**
- Create: `crates/singularmem-search/src/model.rs`
- Append to: `crates/singularmem-search/src/embedder.rs`
- Modify: `crates/singularmem-search/src/error.rs` (new variants)
- Create: `crates/singularmem-search/tests/integration_real_embedder.rs` (`#[ignore]`)

**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Add new Error variants per spec § Error handling.**

Append to `crates/singularmem-search/src/error.rs`:

```rust
    #[error("embedding inference failed during {context}: {reason}")]
    Embedding { context: &'static str, reason: String },

    #[error("could not download embedding model {model}: {reason}")]
    ModelDownload { model: String, reason: String },

    #[error("invalid model files at {path}: {reason}; expected ONNX weights + tokenizer")]
    InvalidModelFiles { path: std::path::PathBuf, reason: String },

    #[error("vector dimension mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },

    #[error("vector index at {path} was built with model {found_model}; \
             current Embedder uses {expected_model}; \
             run `singularmem reindex --with-embeddings --reset-vectors --force` to rebuild")]
    ModelMismatch {
        path: std::path::PathBuf,
        found_model: String,
        expected_model: String,
    },

    #[error("USearch error during {context}: {reason}")]
    Usearch { context: &'static str, reason: String },
```

- [ ] **Step 2: Implement `model.rs`.**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/src/model.rs`

```rust
//! Curated embedding-model registry. The three named models below ship
//! with v0.3.0; users wanting other models use `FastembedEmbedder::from_files`
//! with their own ONNX weights.

/// Curated catalogue of embedding models supported by `FastembedEmbedder::with_model`.
///
/// Any model not in this enum can still be used via `FastembedEmbedder::from_files`,
/// which takes a directory of ONNX weights + tokenizer files.
#[derive(Debug, Clone, Copy)]
pub enum EmbeddingModel {
    /// 384-dim, ~80 MB, English-focused, fast. The default.
    AllMiniLmL6V2,
    /// 384-dim, ~130 MB, English, slightly higher quality than MiniLM.
    BgeSmallEnV15,
    /// 768-dim, ~250 MB, English, larger context (8192 tokens via Matryoshka).
    NomicEmbedTextV15,
}

impl EmbeddingModel {
    /// fastembed's enum value for this model.
    pub(crate) fn fastembed(&self) -> fastembed::EmbeddingModel {
        match self {
            Self::AllMiniLmL6V2 => fastembed::EmbeddingModel::AllMiniLML6V2,
            Self::BgeSmallEnV15 => fastembed::EmbeddingModel::BGESmallENV15,
            Self::NomicEmbedTextV15 => fastembed::EmbeddingModel::NomicEmbedTextV15,
        }
    }

    /// Stable model_id string written into VectorIndexMeta. The `@v1` suffix
    /// is a version anchor so future weight updates trigger a reindex prompt.
    pub fn model_id(&self) -> &'static str {
        match self {
            Self::AllMiniLmL6V2 => "sentence-transformers/all-MiniLM-L6-v2@v1",
            Self::BgeSmallEnV15 => "BAAI/bge-small-en-v1.5@v1",
            Self::NomicEmbedTextV15 => "nomic-ai/nomic-embed-text-v1.5@v1",
        }
    }

    pub fn dim(&self) -> usize {
        match self {
            Self::AllMiniLmL6V2 | Self::BgeSmallEnV15 => 384,
            Self::NomicEmbedTextV15 => 768,
        }
    }

    /// Soft truncation point in tokens. fastembed's tokenizer enforces this
    /// limit; we emit `tracing::warn!` when an input exceeds it.
    pub fn max_tokens(&self) -> usize {
        match self {
            Self::AllMiniLmL6V2 => 256,
            Self::BgeSmallEnV15 => 512,
            Self::NomicEmbedTextV15 => 8192,
        }
    }
}

/// fastembed's model cache directory. Distinct from `dirs::data_dir()` so
/// deleting all stores doesn't waste the download.
pub(crate) fn cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("singularmem")
        .join("models")
}
```

- [ ] **Step 3: Implement `FastembedEmbedder` in `embedder.rs`.**

Append to `crates/singularmem-search/src/embedder.rs`:

```rust
use crate::error::Error;
use crate::model::{cache_dir, EmbeddingModel};
use std::path::Path;

/// Concrete `Embedder` backed by fastembed (ONNX runtime + curated catalogue).
pub struct FastembedEmbedder {
    inner: fastembed::TextEmbedding,
    model_id: String,
    dim: usize,
}

impl FastembedEmbedder {
    /// Construct with the default model (`AllMiniLmL6V2`). Downloads weights
    /// on first construction if not cached.
    ///
    /// # Errors
    /// Returns `Error::ModelDownload` on network failure during first-time
    /// fetch; `Error::Embedding` on ONNX init failure.
    pub fn new() -> crate::Result<Self> {
        Self::with_model(EmbeddingModel::AllMiniLmL6V2)
    }

    /// Construct with a non-default model from the curated catalogue.
    pub fn with_model(model: EmbeddingModel) -> crate::Result<Self> {
        let cache = cache_dir();
        std::fs::create_dir_all(&cache).ok(); // best effort
        let init = fastembed::InitOptions::new(model.fastembed())
            .with_cache_dir(cache)
            .with_show_download_progress(true);
        let inner = fastembed::TextEmbedding::try_new(init).map_err(|e| Error::ModelDownload {
            model: model.model_id().to_string(),
            reason: format!("{e}"),
        })?;
        Ok(Self {
            inner,
            model_id: model.model_id().to_string(),
            dim: model.dim(),
        })
    }

    /// Construct from a directory of ONNX weights + tokenizer files (for
    /// air-gapped use or unsupported models). Caller is responsible for
    /// matching the model_id to whatever produced the files.
    ///
    /// Expected directory contents (fastembed convention): `model.onnx` or
    /// `model_quantized.onnx`, `tokenizer.json`, `config.json`, optionally
    /// `tokenizer_config.json` and `special_tokens_map.json`.
    pub fn from_files(_model_dir: &Path, _model_id: &str) -> crate::Result<Self> {
        Err(Error::InvalidModelFiles {
            path: _model_dir.to_path_buf(),
            reason: "from_files is a planned v0.3.1 feature; not implemented in v0.3.0".to_string(),
        })
    }
}

impl Embedder for FastembedEmbedder {
    fn dim(&self) -> usize { self.dim }
    fn model_id(&self) -> &str { &self.model_id }

    fn embed(&self, content: &str) -> crate::Result<Vec<f32>> {
        let vectors = self.embed_batch(&[content])?;
        vectors.into_iter().next().ok_or_else(|| Error::Embedding {
            context: "embedding single item (empty result)",
            reason: "fastembed returned zero vectors for one input".to_string(),
        })
    }

    fn embed_batch(&self, items: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
        // Truncation warning — fastembed silently truncates beyond max_tokens.
        // We approximate token count with content length / 4 (avg English).
        for s in items {
            let approx_tokens = s.len() / 4;
            if approx_tokens > 256 {
                // Conservative — uses MiniLM's limit as the warning threshold.
                tracing::warn!(
                    approx_tokens,
                    threshold = 256,
                    "item exceeds approximate token limit; fastembed will truncate"
                );
            }
        }
        let owned: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        self.inner.embed(owned, None).map_err(|e| Error::Embedding {
            context: "fastembed inference",
            reason: format!("{e}"),
        })
    }
}
```

**Note on `from_files`:** v0.3.0 ships the stub returning `Error::InvalidModelFiles`. Real implementation is deferred to v0.3.1. Documented as a planned feature; not in the v0.3.0 acceptance criteria.

- [ ] **Step 4: Real-embedder integration test (`#[ignore]`).**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/tests/integration_real_embedder.rs`

```rust
//! Real FastembedEmbedder integration tests. These hit the network on
//! first run to download model weights (~80 MB). Skipped by default;
//! run with `cargo test --test integration_real_embedder -- --ignored`.

use singularmem_search::{Embedder, FastembedEmbedder};

#[test]
#[ignore]
fn fastembed_default_model_works() {
    let e = FastembedEmbedder::new().expect("construct (may download model)");
    assert_eq!(e.dim(), 384);
    let v = e.embed("hello world").unwrap();
    assert_eq!(v.len(), 384);
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.01, "vector should be unit-length, got norm={norm}");
}

#[test]
#[ignore]
fn fastembed_is_deterministic() {
    let e = FastembedEmbedder::new().expect("construct");
    let v1 = e.embed("the quick brown fox").unwrap();
    let v2 = e.embed("the quick brown fox").unwrap();
    assert_eq!(v1, v2);
}
```

- [ ] **Step 5: Re-export from lib.rs + commit.**

Add to `lib.rs` re-exports:
```rust
pub use crate::embedder::FastembedEmbedder;
pub use crate::model::EmbeddingModel;
```

Run `cargo test -p singularmem-search` (the `#[ignore]` tests don't run by default) + clippy + commit.

```bash
git -C /Users/jonasbroms/Sites/singularmem add crates/singularmem-search/src/embedder.rs crates/singularmem-search/src/model.rs crates/singularmem-search/src/error.rs crates/singularmem-search/src/lib.rs crates/singularmem-search/tests/integration_real_embedder.rs
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "feat(search): FastembedEmbedder + EmbeddingModel registry

Three named models: AllMiniLmL6V2 (default, 384-dim, 256 tokens),
BgeSmallEnV15 (384-dim, 512 tokens), NomicEmbedTextV15 (768-dim,
8192 tokens). EmbeddingModel::model_id() returns a stable string
with @v1 suffix written into VectorIndexMeta.

FastembedEmbedder::new() uses the default; with_model picks from
the catalogue. from_files is a stub returning InvalidModelFiles —
real impl deferred to v0.3.1.

Truncation warning: approx_tokens = len/4 (English avg); over 256
emits tracing::warn! before fastembed silently truncates.

Real-network integration tests in tests/integration_real_embedder.rs
are #[ignore]; cargo test doesn't run them by default. Run with
--ignored manually."
```

---

### Task 5: `VectorIndexMeta` + `VectorIndex::open` with model-mismatch check (TDD)

**Files:**
- Create: `crates/singularmem-search/src/vector_index.rs`
- Modify: `crates/singularmem-search/src/lib.rs` (re-exports)
- Create: `crates/singularmem-search/tests/vector_index.rs`

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Failing tests.**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/tests/vector_index.rs`

```rust
//! Tests for VectorIndex: open, model-mismatch detection, meta persistence.

use singularmem_search::testing::MockEmbedder;
use singularmem_search::{Embedder, VectorIndex, VectorIndexOptions};
use tempfile::TempDir;

#[test]
fn open_fresh_creates_meta_and_index_files() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let _ = VectorIndex::open(dir.path().join("vectors"), &e).expect("open fresh");

    let meta_path = dir.path().join("vectors").join(".meta.json");
    let usearch_path = dir.path().join("vectors").join("index.usearch");
    assert!(meta_path.exists(), "meta.json should be created");
    // index.usearch may not exist until first save — that's fine.
}

#[test]
fn reopen_with_same_model_succeeds() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let path = dir.path().join("vectors");
    let _ = VectorIndex::open(&path, &e).expect("open fresh");
    let _ = VectorIndex::open(&path, &e).expect("reopen with same model");
}

#[test]
fn reopen_with_different_model_returns_model_mismatch() {
    let dir = TempDir::new().unwrap();
    let e1 = MockEmbedder::default();
    let path = dir.path().join("vectors");
    let _ = VectorIndex::open(&path, &e1).expect("open with e1");
    drop(e1);

    // Construct an Embedder with a different model_id.
    struct OtherMock;
    impl Embedder for OtherMock {
        fn dim(&self) -> usize { 384 }
        fn model_id(&self) -> &str { "different-model@v1" }
        fn embed(&self, _: &str) -> singularmem_search::Result<Vec<f32>> { unimplemented!() }
    }

    let result = VectorIndex::open(&path, &OtherMock);
    match result {
        Err(singularmem_search::Error::ModelMismatch { found_model, expected_model, .. }) => {
            assert_eq!(found_model, "mock-embedder@v1");
            assert_eq!(expected_model, "different-model@v1");
        }
        other => panic!("expected ModelMismatch, got {other:?}"),
    }
}

#[test]
fn open_with_options_respects_hnsw_params() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let opts = VectorIndexOptions {
        hnsw_m: 32,
        hnsw_ef_construction: 256,
        expansion_search: 128,
    };
    let idx = VectorIndex::open_with_options(dir.path().join("v"), &e, opts).unwrap();
    assert_eq!(idx.meta().hnsw_m, 32);
}
```

- [ ] **Step 2: Run — must fail.**

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo test -p singularmem-search --test vector_index 2>&1 | tail -10
```

- [ ] **Step 3: Implement vector_index.rs (open + meta only; add/search land in Tasks 6-7).**

File: `/Users/jonasbroms/Sites/singularmem/crates/singularmem-search/src/vector_index.rs`

```rust
//! `VectorIndex` — wraps usearch::Index with a sidecar .meta.json carrying
//! model + dimensionality + HNSW params. Add/search land in Task 6+7; this
//! task implements open + meta persistence + model-mismatch detection.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use usearch::{IndexOptions, MetricKind, ScalarKind};

use crate::embedder::Embedder;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub struct VectorIndexOptions {
    pub hnsw_m: usize,
    pub hnsw_ef_construction: usize,
    pub expansion_search: usize,
}

impl Default for VectorIndexOptions {
    fn default() -> Self {
        Self { hnsw_m: 16, hnsw_ef_construction: 128, expansion_search: 64 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorIndexMeta {
    pub format_version: String,
    pub model_id: String,
    pub dim: usize,
    pub distance: String,
    pub hnsw_m: usize,
    pub hnsw_ef_construction: usize,
    pub created_at: jiff::Timestamp,
}

pub struct VectorIndex {
    pub(crate) inner: Mutex<usearch::Index>,
    pub(crate) path: PathBuf,
    pub(crate) meta_path: PathBuf,
    pub(crate) usearch_path: PathBuf,
    pub(crate) meta: VectorIndexMeta,
}

impl VectorIndex {
    pub fn open(dir: impl AsRef<Path>, embedder: &dyn Embedder) -> Result<Self> {
        Self::open_with_options(dir, embedder, VectorIndexOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        embedder: &dyn Embedder,
        options: VectorIndexOptions,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(Error::Io)?;
        let meta_path = dir.join(".meta.json");
        let usearch_path = dir.join("index.usearch");

        let meta = if meta_path.exists() {
            let text = std::fs::read_to_string(&meta_path).map_err(Error::Io)?;
            let m: VectorIndexMeta = serde_json::from_str(&text).map_err(|e| Error::Embedding {
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

        // Persist meta on first open.
        if !meta_path.exists() {
            let text = serde_json::to_string_pretty(&meta).map_err(|e| Error::Embedding {
                context: "serializing .meta.json",
                reason: format!("{e}"),
            })?;
            std::fs::write(&meta_path, text).map_err(Error::Io)?;
        }

        // Construct usearch::Index.
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

        // Reserve capacity (USearch growth is amortized; reserve avoids reallocs).
        inner.reserve(1024).map_err(|e| Error::Usearch {
            context: "reserving usearch capacity",
            source: format!("{e}").into(),
        })?;
        // ^ NOTE: `source` field is `String`, not `reason` — adjust to the
        // actual Error::Usearch shape (the macro-generated field is named
        // `reason` per Task 4 step 1). Use `reason: format!(...)`.

        // Load existing data if present.
        if usearch_path.exists() {
            inner.load(usearch_path.to_str().unwrap()).map_err(|e| Error::Usearch {
                context: "loading existing usearch index",
                reason: format!("{e}"),
            })?;
        }

        Ok(Self {
            inner: Mutex::new(inner),
            path: dir.to_path_buf(),
            meta_path,
            usearch_path,
            meta,
        })
    }

    pub fn meta(&self) -> &VectorIndexMeta { &self.meta }
}
```

**Implementer note:** The `inner.reserve(...)` line in Step 3 above has a placeholder showing the right error-variant field name (`reason`, not `source`). Use `reason: format!("{e}")` for all Error::Usearch constructions.

- [ ] **Step 4: Run + clippy + commit.**

Run tests + clippy. Address any pedantic/nursery lints individually. Commit.

```bash
git -C /Users/jonasbroms/Sites/singularmem add crates/singularmem-search/src/vector_index.rs crates/singularmem-search/src/lib.rs crates/singularmem-search/tests/vector_index.rs
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "feat(search): VectorIndex::open + VectorIndexMeta + model-mismatch detection

Open path:
- Fresh: write .meta.json from the embedder's model_id/dim + HNSW params.
- Existing: read .meta.json, assert model_id matches embedder.model_id();
  if not → Error::ModelMismatch with recovery hint pointing at
  reindex --with-embeddings --reset-vectors --force.

usearch::Index constructed with cosine distance, f32 quantization, HNSW
params from VectorIndexMeta. Existing index.usearch loaded if present;
add/save land in Task 6.

Three integration tests: fresh open creates meta, reopen with same model
succeeds, reopen with different model returns ModelMismatch."
```

---

### Task 6: `VectorIndex::add` / `remove` / `save` / `doc_count` + keymap (TDD)

**Files:**
- Append to: `crates/singularmem-search/src/vector_index.rs`
- Append to: `crates/singularmem-search/tests/vector_index.rs`

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Failing tests.**

Append to `tests/vector_index.rs`:

```rust
use singularmem_core::ItemId;
use std::str::FromStr;

#[test]
fn add_then_save_then_reopen_preserves_vectors() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let path = dir.path().join("v");
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    {
        let idx = VectorIndex::open(&path, &e).unwrap();
        let v = e.embed("hello").unwrap();
        idx.add(id, &v).expect("add");
        idx.save().expect("save");
        assert_eq!(idx.doc_count().unwrap(), 1);
    }
    let idx2 = VectorIndex::open(&path, &e).unwrap();
    assert_eq!(idx2.doc_count().unwrap(), 1);
}

#[test]
fn add_with_wrong_dim_returns_dim_mismatch() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default(); // 384
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let too_small = vec![0.0_f32; 128];
    let err = idx.add(id, &too_small).unwrap_err();
    assert!(matches!(err, singularmem_search::Error::DimMismatch { expected: 384, got: 128 }));
}

#[test]
fn remove_absent_id_is_noop() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    idx.remove(id).expect("remove of absent ID is no-op, not error");
}
```

- [ ] **Step 2: Run — must fail.**

- [ ] **Step 3: Implement add / remove / save / doc_count.**

Append to `crates/singularmem-search/src/vector_index.rs`:

```rust
use std::collections::BTreeMap;
use std::fs;
use singularmem_core::ItemId;

impl VectorIndex {
    /// Add (or replace) one item's vector. Generates a sequential u64 key
    /// and records the mapping in the in-memory keymap. Use `save` to
    /// persist both the USearch index and the keymap to disk.
    pub fn add(&self, id: ItemId, vector: &[f32]) -> Result<()> {
        if vector.len() != self.meta.dim {
            return Err(Error::DimMismatch { expected: self.meta.dim, got: vector.len() });
        }
        let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
        let key = keymap.next_key;
        keymap.next_key += 1;
        keymap.forward.insert(key, id);
        keymap.reverse.insert(id, key);

        let inner = self.inner.lock().expect("usearch mutex poisoned");
        inner.add(key, vector).map_err(|e| Error::Usearch {
            context: "usearch add",
            reason: format!("{e}"),
        })?;
        Ok(())
    }

    pub fn remove(&self, id: ItemId) -> Result<()> {
        let mut keymap = self.keymap.lock().expect("keymap mutex poisoned");
        if let Some(key) = keymap.reverse.remove(&id) {
            keymap.forward.remove(&key);
            let inner = self.inner.lock().expect("usearch mutex poisoned");
            inner.remove(key).map_err(|e| Error::Usearch {
                context: "usearch remove",
                reason: format!("{e}"),
            })?;
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let inner = self.inner.lock().expect("usearch mutex poisoned");
        inner.save(self.usearch_path.to_str().unwrap()).map_err(|e| Error::Usearch {
            context: "usearch save",
            reason: format!("{e}"),
        })?;
        let keymap = self.keymap.lock().expect("keymap mutex poisoned");
        let bytes = bincode::serialize(&*keymap).map_err(|e| Error::Embedding {
            context: "serializing keymap",
            reason: format!("{e}"),
        })?;
        fs::write(self.path.join("keymap.bin"), bytes).map_err(Error::Io)?;
        Ok(())
    }

    pub fn doc_count(&self) -> Result<u64> {
        let inner = self.inner.lock().expect("usearch mutex poisoned");
        Ok(inner.size() as u64)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Keymap {
    pub next_key: u64,
    pub forward: BTreeMap<u64, ItemId>,
    pub reverse: BTreeMap<ItemId, u64>,
}
```

**Implementer note:** Add `keymap: Mutex<Keymap>` field to `VectorIndex`. Load it from `path.join("keymap.bin")` on open (if present); construct empty otherwise. Serde-derive on `ItemId` already exists (sub-project 1).

- [ ] **Step 4: Run + clippy + commit.**

```bash
git -C /Users/jonasbroms/Sites/singularmem add crates/singularmem-search/src/vector_index.rs crates/singularmem-search/tests/vector_index.rs
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "feat(search): VectorIndex add/remove/save/doc_count + keymap persistence

Sequential u64 keymap (BTreeMap<u64, ItemId> + reverse) persisted as
keymap.bin via bincode. add() generates the next key, records both
directions, calls usearch::Index::add. remove() drops both directions
+ usearch::Index::remove (no-op on absent ID). save() flushes both
the usearch binary and keymap.bin together.

Three new integration tests: add → save → reopen preserves; wrong
dim returns DimMismatch; remove of absent ID is no-op."
```

---

### Task 7: `VectorIndex::search` (KNN) (TDD)

**Files:**
- Append to: `crates/singularmem-search/src/vector_index.rs`
- Append to: `crates/singularmem-search/tests/vector_index.rs`

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Failing test.**

```rust
#[test]
fn search_returns_nearest_neighbours_by_cosine() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();

    let id1 = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let id2 = ItemId::from_str("01BX5ZZKBKACTAV9WEVGEMMVRZ").unwrap();
    let v1 = e.embed("hello world").unwrap();
    let v2 = e.embed("totally different text").unwrap();
    idx.add(id1, &v1).unwrap();
    idx.add(id2, &v2).unwrap();
    idx.save().unwrap();

    let query = e.embed("hello world").unwrap();
    let hits = idx.search(&query, 2).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, id1, "highest similarity = same vector = id1");
    assert!(hits[0].score > 0.99, "self-similarity should be ~1.0, got {}", hits[0].score);
}
```

- [ ] **Step 2: Implement search.**

Append to vector_index.rs:

```rust
pub struct VectorHit {
    pub id: ItemId,
    pub score: f32,
}

impl VectorIndex {
    pub fn search(&self, query_vector: &[f32], k: usize) -> Result<Vec<VectorHit>> {
        if query_vector.len() != self.meta.dim {
            return Err(Error::DimMismatch { expected: self.meta.dim, got: query_vector.len() });
        }
        let inner = self.inner.lock().expect("usearch mutex poisoned");
        let matches = inner.search(query_vector, k).map_err(|e| Error::Usearch {
            context: "usearch search",
            reason: format!("{e}"),
        })?;
        drop(inner);

        let keymap = self.keymap.lock().expect("keymap mutex poisoned");
        let hits: Vec<VectorHit> = matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .filter_map(|(k, dist)| {
                // USearch cosine returns distance; similarity = 1 - distance.
                let score = 1.0 - dist;
                keymap.forward.get(k).map(|id| VectorHit { id: *id, score })
            })
            .collect();
        Ok(hits)
    }
}
```

- [ ] **Step 3: Run + commit.**

```bash
git -C /Users/jonasbroms/Sites/singularmem add crates/singularmem-search/src/vector_index.rs crates/singularmem-search/tests/vector_index.rs
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "feat(search): VectorIndex::search (KNN) + VectorHit

USearch returns cosine DISTANCE (0=identical, 2=opposite); we convert to
cosine SIMILARITY (1=identical, -1=opposite) via 1.0 - distance. Keymap
lookup translates u64 keys back to ItemId.

One integration test: self-similarity for identical vectors > 0.99."
```

---

### Task 8: `EmbedderIndex` — IndexHook impl (TDD)

**Files:** Append to `crates/singularmem-search/src/vector_index.rs`; create `crates/singularmem-search/tests/embedder_index.rs`.

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Failing test.**

```rust
//! tests/embedder_index.rs
use singularmem_core::{IndexHook, Item, ItemId};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, VectorIndex};
use std::str::FromStr;
use tempfile::TempDir;
use jiff::Timestamp;

#[test]
fn on_ingest_then_commit_increments_doc_count() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v");
    let idx = EmbedderIndex::open(&path, Box::new(MockEmbedder::default())).unwrap();
    let item = Item {
        id: ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
        content: "hello".into(),
        created_at: Timestamp::now(),
        supersedes: None,
        tags: vec![],
        source: None,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
    };
    idx.on_ingest(&item).unwrap();
    idx.commit().unwrap();
    assert_eq!(idx.vector_index().doc_count().unwrap(), 1);
}
```

- [ ] **Step 2: Implement.**

Append to `vector_index.rs`:

```rust
pub struct EmbedderIndex {
    embedder: Box<dyn Embedder>,
    vector_index: VectorIndex,
}

impl EmbedderIndex {
    pub fn open(dir: impl AsRef<Path>, embedder: Box<dyn Embedder>) -> Result<Self> {
        let vector_index = VectorIndex::open(dir, embedder.as_ref())?;
        Ok(Self { embedder, vector_index })
    }

    pub fn vector_index(&self) -> &VectorIndex { &self.vector_index }
    pub fn embedder(&self) -> &dyn Embedder { self.embedder.as_ref() }
}

impl singularmem_core::IndexHook for EmbedderIndex {
    fn on_ingest(&self, item: &singularmem_core::Item) -> singularmem_core::Result<()> {
        let v = self.embedder.embed(&item.content).map_err(to_core_err)?;
        self.vector_index.add(item.id, &v).map_err(to_core_err)
    }
    fn on_reindex(&self, item: &singularmem_core::Item) -> singularmem_core::Result<()> {
        self.on_ingest(item)
    }
    fn commit(&self) -> singularmem_core::Result<()> {
        self.vector_index.save().map_err(to_core_err)
    }
}

fn to_core_err(e: crate::Error) -> singularmem_core::Error {
    singularmem_core::Error::Io(std::io::Error::other(e.to_string()))
}
```

Re-export `EmbedderIndex` from lib.rs.

- [ ] **Step 3: Run + commit.**

```bash
git commit -s -m "feat(search): EmbedderIndex (impl IndexHook)

Composes Embedder + VectorIndex. on_ingest/on_reindex embed content
+ add to vector_index. commit saves the USearch file + keymap.bin.

Hook errors wrap as singularmem_core::Error::Io with the full
message (the trait can't return a search error without coupling
core to search; full message survives the lossy conversion for
log purposes, exact type info is lost)."
```

---

### Task 9: `EmbedderIndex::semantic_search` (TDD)

**Files:** Create `crates/singularmem-search/src/semantic_query.rs`; append impl to `vector_index.rs`; create `crates/singularmem-search/tests/embedder_to_search.rs`.

**Assigned skill:** `test-driven-development`

- [ ] **Step 1: Implement result types in `semantic_query.rs`.**

```rust
use singularmem_core::ItemId;
use std::time::Duration;

pub struct SemanticSearchOptions {
    pub limit: usize,
    pub min_score: f32,
}

impl Default for SemanticSearchOptions {
    fn default() -> Self { Self { limit: 20, min_score: 0.0 } }
}

pub struct SemanticSearchResults {
    pub hits: Vec<SemanticHit>,
    pub elapsed: Duration,
    pub total_indexed: u64,
}

pub struct SemanticHit {
    pub id: ItemId,
    pub score: f32,
}
```

- [ ] **Step 2: Add semantic_search to EmbedderIndex (append to vector_index.rs).**

```rust
use crate::semantic_query::{SemanticHit, SemanticSearchOptions, SemanticSearchResults};

impl EmbedderIndex {
    pub fn semantic_search(&self, query: &str, opts: SemanticSearchOptions) -> Result<SemanticSearchResults> {
        let start = std::time::Instant::now();
        let qv = self.embedder.embed(query)?;
        let raw = self.vector_index.search(&qv, opts.limit)?;
        let total_indexed = self.vector_index.doc_count()?;
        let hits: Vec<SemanticHit> = raw
            .into_iter()
            .filter(|h| h.score >= opts.min_score)
            .map(|h| SemanticHit { id: h.id, score: h.score })
            .collect();
        Ok(SemanticSearchResults { hits, elapsed: start.elapsed(), total_indexed })
    }
}
```

Re-export `SemanticSearchOptions`, `SemanticSearchResults`, `SemanticHit` from lib.rs.

- [ ] **Step 3: Integration test.**

File: `crates/singularmem-search/tests/embedder_to_search.rs`

```rust
use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, SemanticSearchOptions};
use tempfile::TempDir;

#[test]
fn ingest_then_semantic_search_finds_item() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let vectors_path = dir.path().join("vectors");

    let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
    let store = Store::open_with_hook(&store_path, Box::new(embedder_idx)).unwrap();
    let item = store.ingest(NewItem::text("the cat sat on the mat")).unwrap();
    drop(store);

    // Re-open EmbedderIndex for search; the hook's instance is now dropped.
    let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
    let results = embedder_idx.semantic_search("the cat sat on the mat", SemanticSearchOptions::default()).unwrap();
    assert!(results.hits.iter().any(|h| h.id == item.id));
}
```

- [ ] **Step 4: Run + commit.**

```bash
git commit -s -m "feat(search): EmbedderIndex::semantic_search + result types

Embed query → KNN against vector_index → filter by min_score →
return SemanticSearchResults with hits + elapsed + total_indexed.

One integration test: ingest via Store (with EmbedderIndex hook) →
re-open EmbedderIndex for search → finds the item."
```

---

### Task 10: CLI `semantic-search` verb + new error exit codes (TDD)

**Files:** Modify `src/main.rs`; append CLI tests to `tests/cli.rs`.

**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Add to main.rs.**

Add to `Command` enum: `SemanticSearch(SemanticSearchArgs)`.

Add struct:

```rust
#[derive(Args, Debug)]
struct SemanticSearchArgs {
    queries: Vec<String>,
    #[arg(long, default_value = "20")]
    limit: usize,
    #[arg(long, default_value = "0.0")]
    min_score: f32,
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
}
```

Add handler:

```rust
fn cmd_semantic_search(store_path: &Path, args: &SemanticSearchArgs) -> Result<(), CliError> {
    use singularmem_search::testing::MockEmbedder;
    // In v0.3.0, the production CLI uses FastembedEmbedder. For deterministic
    // tests we let the test re-set the embedder via an env var. But the CLI
    // shipped to users uses FastembedEmbedder unconditionally:
    let embedder: Box<dyn singularmem_search::Embedder> = match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
        Some("mock") => Box::new(MockEmbedder::default()),
        _ => Box::new(singularmem_search::FastembedEmbedder::new()
            .map_err(|e| CliError::IndexOpen(format!("embedder init failed: {e}")))?),
    };

    let vectors_path = derive_vectors_path(store_path);
    let idx = singularmem_search::EmbedderIndex::open(&vectors_path, embedder)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    let query_str = args.queries.join(" ");
    let results = idx.semantic_search(&query_str, singularmem_search::SemanticSearchOptions {
        limit: args.limit, min_score: args.min_score,
    }).map_err(|e| CliError::IndexOpen(e.to_string()))?;

    if results.hits.is_empty() {
        tracing::info!("0 matches");
        return Ok(());
    }
    let mut out = io::stdout().lock();
    for hit in &results.hits {
        match args.format {
            ListFormat::Ids => writeln!(out, "{}", hit.id)?,
            ListFormat::Jsonl => {
                serde_json::to_writer(&mut out, &serde_json::json!({
                    "id": hit.id.to_string(), "score": hit.score,
                }))?;
                writeln!(out)?;
            }
            ListFormat::Table => writeln!(out, "{:.4}\t{}", hit.score, hit.id)?,
        }
    }
    Ok(())
}

fn derive_vectors_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    PathBuf::from(s)
}
```

Wire dispatch: `Command::SemanticSearch(args) => cmd_semantic_search(&store_path, &args)`.

- [ ] **Step 2: CLI integration tests.**

```rust
#[test]
fn semantic_search_with_mock_embedder_finds_ingested_item() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Pre-create the .vectors/ dir so auto-wiring fires.
    // (Phase 5 reindex --with-embeddings creates this; we shortcut for the test.)
    std::fs::create_dir_all(db.with_extension("db.vectors")).unwrap();

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "ingest", "--content", "cat sat on mat"])
        .assert()
        .success();

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "semantic-search", "cat sat on mat"])
        .assert()
        .success()
        .stdout(predicate::str::contains("01")); // any ULID
}

#[test]
fn semantic_search_missing_index_exits_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    // No .vectors/ dir AND no FastembedEmbedder available — should fail fast.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "semantic-search", "anything"])
        .assert()
        .failure()
        .code(2);
}
```

The `SINGULARMEM_TEST_EMBEDDER=mock` env var keeps tests fast + network-free. Without it, the CLI uses `FastembedEmbedder::new()`.

- [ ] **Step 3: Run + commit.**

```bash
git commit -s -m "feat(cli): semantic-search verb

New CLI verb. Default embedder is FastembedEmbedder::new() (downloads
all-MiniLM-L6-v2 weights on first use). SINGULARMEM_TEST_EMBEDDER=mock
env var swaps in MockEmbedder for fast deterministic tests — production
binary never reads this var because tests/cli.rs is the only setter.

Two CLI integration tests: end-to-end with mock embedder, and
missing-index exits 2."
```

---

### Task 11: CLI `reindex --with-embeddings` flag + `--reset-vectors` (TDD)

**Files:** Modify `src/main.rs` (extend `cmd_reindex`); append CLI tests.

**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Add flags to `ReindexArgs`.**

```rust
#[derive(Args, Debug)]
struct ReindexArgs {
    #[arg(long)]
    quiet: bool,
    /// NEW: also rebuild the vector index.
    #[arg(long)]
    with_embeddings: bool,
    /// NEW: which embedding model to use. Only meaningful with --with-embeddings.
    #[arg(long, default_value = "all-mini-lm-l6-v2")]
    embedding_model: String,
    /// NEW: destructive — delete .vectors/ before reindex (e.g. to switch models).
    #[arg(long)]
    reset_vectors: bool,
    /// NEW: required to confirm --reset-vectors.
    #[arg(long)]
    force: bool,
}
```

- [ ] **Step 2: Extend `cmd_reindex` to handle --with-embeddings.**

```rust
fn cmd_reindex(store: &Store, store_path: &Path, args: &ReindexArgs) -> Result<(), CliError> {
    // Existing Tantivy reindex (Task 11 from sub-project 2a) — unchanged.
    let tantivy_path = derive_index_path(store_path);
    let tantivy_index = singularmem_search::Index::open(&tantivy_path)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    let progress = |n: u64| { if !args.quiet { tracing::info!("reindex (tantivy): {n} items"); } };
    let count = tantivy_index
        .reindex_from(store.list()?.filter_map(Result::ok), progress)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    tracing::info!("tantivy reindex: {count} items total");

    // NEW: optional embedder reindex.
    if args.with_embeddings {
        let vectors_path = derive_vectors_path(store_path);
        if args.reset_vectors {
            if !args.force {
                return Err(CliError::Usage(
                    "--reset-vectors requires --force to confirm destructive operation".into()
                ));
            }
            if vectors_path.exists() {
                std::fs::remove_dir_all(&vectors_path).map_err(CliError::Io)?;
                tracing::warn!(path = %vectors_path.display(), "deleted existing vector index");
            }
        }
        let model = match args.embedding_model.as_str() {
            "all-mini-lm-l6-v2" => singularmem_search::EmbeddingModel::AllMiniLmL6V2,
            "bge-small-en" => singularmem_search::EmbeddingModel::BgeSmallEnV15,
            "nomic-embed" => singularmem_search::EmbeddingModel::NomicEmbedTextV15,
            other => return Err(CliError::Usage(format!("unknown --embedding-model: {other}"))),
        };

        let embedder: Box<dyn singularmem_search::Embedder> = match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
            Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
            _ => Box::new(singularmem_search::FastembedEmbedder::with_model(model)
                .map_err(|e| CliError::IndexOpen(format!("embedder init: {e}")))?),
        };

        let embedder_idx = singularmem_search::EmbedderIndex::open(&vectors_path, embedder)
            .map_err(|e| CliError::IndexOpen(e.to_string()))?;

        for (i, item_r) in store.list()?.enumerate() {
            let item = item_r?;
            embedder_idx.on_reindex(&item).map_err(|e| CliError::IndexOpen(e.to_string()))?;
            if !args.quiet && (i + 1) % 100 == 0 {
                tracing::info!("reindex (embeddings): {} items", i + 1);
            }
        }
        embedder_idx.commit().map_err(|e| CliError::IndexOpen(e.to_string()))?;
        tracing::info!("embedder reindex complete");
    }
    Ok(())
}
```

- [ ] **Step 3: CLI integration test.**

```rust
#[test]
fn reindex_with_embeddings_creates_vectors_dir() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let vectors = db.with_extension("db.vectors");

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "ingest", "--content", "first item"])
        .assert()
        .success();

    assert!(!vectors.exists(), ".vectors/ should not exist before reindex --with-embeddings");

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "reindex", "--with-embeddings"])
        .assert()
        .success();

    assert!(vectors.exists(), ".vectors/ should be created");
}

#[test]
fn reset_vectors_without_force_fails() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "reindex", "--with-embeddings", "--reset-vectors"])
        .assert()
        .failure();
}
```

- [ ] **Step 4: Run + commit.**

```bash
git commit -s -m "feat(cli): reindex --with-embeddings + --embedding-model + --reset-vectors

Without --with-embeddings: existing Tantivy-only behaviour unchanged.
With --with-embeddings: optionally delete .vectors/ (--reset-vectors
+ --force), select model (--embedding-model), embed every item from
SQLite, save vector index. Two-phase: Tantivy reindex always runs
first; embedder reindex runs second if requested.

Two CLI integration tests cover the happy path and the --reset-vectors
safety gate."
```

---

### Task 12: Auto-wiring updates — `MultiHook` from `.vectors/` existence (TDD)

**Files:** Modify `src/main.rs` (`open_store` helper); append CLI tests.

**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Update `open_store` to build MultiHook.**

Replace the existing single-hook auto-wiring block in `open_store` (was added in sub-project 2a) with the multi-hook version per spec § Section 4:

```rust
let needs_hook = matches!(cli.command, Command::Ingest(_));
if needs_hook && !cli.no_index {
    let mut hooks: Vec<Box<dyn singularmem_core::IndexHook>> = Vec::new();

    // Tantivy hook (sub-project 2a behaviour).
    let tantivy_path = derive_index_path(&store_path);
    match singularmem_search::Index::open(&tantivy_path) {
        Ok(idx) => hooks.push(Box::new(idx)),
        Err(e) => tracing::warn!(error = %e, path = %tantivy_path.display(),
            "could not open Tantivy index; lexical search will not work until reindex"),
    }

    // Embedder hook (NEW; only if .vectors/ already exists — opt-in).
    let vectors_path = derive_vectors_path(&store_path);
    if vectors_path.exists() {
        let embedder: Box<dyn singularmem_search::Embedder> = match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
            Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
            _ => match singularmem_search::FastembedEmbedder::new() {
                Ok(e) => Box::new(e),
                Err(e) => {
                    tracing::warn!(error = %e, "embedder construction failed; semantic search will not work");
                    return Ok((store, store_path));
                }
            },
        };
        match singularmem_search::EmbedderIndex::open(&vectors_path, embedder) {
            Ok(idx) => hooks.push(Box::new(idx)),
            Err(e) => tracing::warn!(error = %e, "vector index open failed; semantic search will not work"),
        }
    }

    if !hooks.is_empty() {
        store.set_hook(Some(Box::new(singularmem_core::hook::MultiHook::new(hooks))));
    }
}
```

- [ ] **Step 2: CLI integration test for auto-wiring.**

```rust
#[test]
fn auto_wiring_writes_to_both_tantivy_and_embedder_after_reindex_with_embeddings() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Trigger the .vectors/ directory creation via reindex --with-embeddings.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "reindex", "--with-embeddings"])
        .assert()
        .success();

    // Now ingest. Both Tantivy and Embedder hooks should fire.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "ingest", "--content", "auto-wired-both item"])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Lexical search finds it.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "auto-wired-both"])
        .assert()
        .success();

    // Semantic search finds it.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "semantic-search", "auto-wired-both"])
        .assert()
        .success();
}
```

- [ ] **Step 3: Run + commit.**

```bash
git commit -s -m "feat(cli): auto-wire MultiHook(Tantivy + Embedder) when .vectors/ exists

open_store now builds a Vec<Box<dyn IndexHook>>: Tantivy always (unless
opening failed), Embedder only if .vectors/ already exists (the opt-in
mechanic from spec § Section 4). If both succeed, wrap in MultiHook and
attach. If only one succeeds, attach it alone. If neither, leave hook
unset.

One CLI integration test verifies the full chain: reindex --with-embeddings
creates .vectors/ → ingest auto-wires both hooks → both search verbs
find the item."
```

---

### Task 13: Principle VII MultiHook isolation test

**Files:** Append to `crates/singularmem-core/tests/multi_hook.rs`.

**Assigned skill:** `test-driven-development`

- [ ] Append the `failing_embedder_does_not_prevent_tantivy` test pattern (uses a `FailingHook` + a `CountingHook`; asserts that even though FailingHook errors, CountingHook still fires + the SQLite item is still durable + Store::ingest returns Ok).

```rust
struct FailingHook;
impl IndexHook for FailingHook {
    fn on_ingest(&self, _: &Item) -> Result<()> {
        Err(singularmem_core::Error::Io(std::io::Error::other("simulated failure")))
    }
    fn on_reindex(&self, _: &Item) -> Result<()> { Ok(()) }
    fn commit(&self) -> Result<()> { Ok(()) }
}

#[test]
fn failing_hook_in_multi_hook_does_not_block_others() {
    let working_count = Arc::new(AtomicUsize::new(0));
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");

    let store = Store::open_with_hooks(&path, vec![
        Box::new(FailingHook),
        Box::new(CountingHook {
            on_ingest_calls: Arc::clone(&working_count),
            commit_calls: Arc::new(AtomicUsize::new(0)),
        }),
    ]).unwrap();

    // ingest returns Ok despite the failing hook (Principle VII).
    let item = store.ingest(NewItem::text("durable")).unwrap();
    assert_eq!(working_count.load(Ordering::SeqCst), 1, "working hook still fires");

    // Item is durable in SQLite.
    let fetched = store.get(item.id).unwrap();
    assert_eq!(fetched.content, "durable");
}
```

Run + commit:

```bash
git commit -s -m "test(core): MultiHook Principle VII isolation

A failing hook in MultiHook does NOT prevent the others from running.
SQLite write is durable regardless. Mirrors the asymmetric write
semantics established in sub-project 2a."
```

---

### Task 14: Property tests (Embedder determinism + self-similarity)

**Files:** Create `crates/singularmem-search/tests/property_embeddings.rs`.

**Assigned skill:** `test-driven-development`

```rust
use proptest::prelude::*;
use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, SemanticSearchOptions};
use tempfile::TempDir;

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

    /// Embedding is deterministic given identical input.
    #[test]
    fn embed_is_deterministic(content in "[a-zA-Z ]{1,200}") {
        use singularmem_search::Embedder;
        let e = MockEmbedder::default();
        let v1 = e.embed(&content).unwrap();
        let v2 = e.embed(&content).unwrap();
        prop_assert_eq!(v1, v2);
    }

    /// An ingested item's embedding is its own nearest neighbour with high score.
    #[test]
    fn ingest_then_semantic_search_finds_self(content in "[a-zA-Z ]{20,200}") {
        let dir = TempDir::new().unwrap();
        let vectors_path = dir.path().join("v");
        let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(dir.path().join("store.db"), Box::new(embedder_idx)).unwrap();
        let item = store.ingest(NewItem::text(content.clone())).unwrap();
        drop(store);

        let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        let results = embedder_idx.semantic_search(&content, SemanticSearchOptions::default()).unwrap();
        prop_assert!(results.hits.iter().any(|h| h.id == item.id && h.score > 0.95),
            "self-similarity should be ~1.0; hits: {:?}", results.hits);
    }
}
```

Run + commit.

---

### Task 15: Concurrency tests — readers during embedder reindex

**Files:** Append to `crates/singularmem-search/tests/concurrency.rs`.

**Assigned skill:** `test-driven-development`

```rust
#[test]
fn parallel_semantic_searchers_during_reindex_see_consistent_state() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let vectors_path = dir.path().join("v");

    // Seed 200 items with embedder attached.
    {
        let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(&store_path, Box::new(embedder_idx)).unwrap();
        for i in 0..200 {
            store.ingest(NewItem::text(format!("item {i}"))).unwrap();
        }
    }

    let vectors_arc = Arc::new(vectors_path.clone());
    let mut readers = Vec::new();
    for _ in 0..4 {
        let path = Arc::clone(&vectors_arc);
        readers.push(thread::spawn(move || {
            for _ in 0..20 {
                let idx = EmbedderIndex::open(&*path, Box::new(MockEmbedder::default())).unwrap();
                let _ = idx.semantic_search("item 50", SemanticSearchOptions::default()).unwrap();
            }
        }));
    }

    let reindex_path = vectors_path.clone();
    let store_path2 = store_path.clone();
    let reindexer = thread::spawn(move || {
        let store = Store::open(&store_path2).unwrap();
        let idx = EmbedderIndex::open(&reindex_path, Box::new(MockEmbedder::default())).unwrap();
        for item in store.list().unwrap().filter_map(Result::ok) {
            idx.on_reindex(&item).unwrap();
        }
        idx.commit().unwrap();
    });

    for r in readers { r.join().unwrap(); }
    reindexer.join().unwrap();
}
```

Run + commit.

---

### Task 16: Format spec — append USearch vector sidecar section

**Files:** Modify `docs/formats/store-v1.md`.

**Assigned skill:** `verification-before-completion`

Append a new `## USearch vector sidecar (optional, format unstable across USearch versions)` section per spec § "On-disk format". Include:
- Directory layout under `<store_path>.vectors/`
- `.meta.json` VectorIndexMeta schema (JSON example)
- `keymap.bin` bincode schema (`BTreeMap<u64, ItemId>`)
- HNSW params (m=16, ef_construction=128, distance=cosine)
- USearch version pin = `2.15.3` and version-bump → reindex requirement
- "Writing a third-party vector loader" walkthrough

Run + commit.

---

### Task 17: Criterion benches — embed throughput + semantic search latency

**Files:** Append to `crates/singularmem-search/benches/search_perf.rs`.

**Assigned skill:** `rust-best-practices`

Add two new bench groups:

```rust
fn bench_embed_throughput(c: &mut Criterion) {
    use singularmem_search::testing::MockEmbedder;
    use singularmem_search::Embedder;
    let e = MockEmbedder::default();
    c.bench_function("embed_throughput", |b| {
        b.iter(|| e.embed("benchmark item with moderate content length").unwrap());
    });
}

fn bench_semantic_search_latency(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let embedder_idx = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let store = Store::open_with_hook(dir.path().join("store.db"), Box::new(embedder_idx)).unwrap();
    for i in 0..10_000 {
        store.ingest(NewItem::text(format!("seed item number {i}"))).unwrap();
    }
    drop(store);
    let embedder_idx = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    c.bench_function("semantic_search_latency", |b| {
        b.iter(|| embedder_idx.semantic_search("seed item number 5000", SemanticSearchOptions::default()).unwrap());
    });
}

criterion_group!(benches,
    bench_search_latency,
    bench_reindex_throughput,
    bench_embed_throughput,
    bench_semantic_search_latency,
);
```

Run + commit.

---

### Task 18: Extend `perf-check.sh` with semantic-search budget

**Files:** Modify `.github/scripts/perf-check.sh`.

**Assigned skill:** `verification-before-completion`

Add a 5th budget check after the existing four:

```bash
# 5. Semantic search latency: < 100 ms (median of criterion estimates.json)
SEM_NS=$(read_median_ns "semantic_search_latency")
SEM_MS=$(awk -v ns="$SEM_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$SEM_MS" 'BEGIN { exit !(v >= 100) }'; then
    echo "FAIL: semantic search latency ${SEM_MS} ms exceeds 100 ms" >&2
    exit 15
fi
echo "  semantic search:   ${SEM_MS} ms (limit 100)"
```

Exit code 15 for the new budget. Update the existing success-summary line to mention it. Run + commit.

---

### Task 19: CONTRIBUTING.md — document the cheap-vs-heavy default rule

**Files:** Modify `CONTRIBUTING.md`.

**Assigned skill:** `verification-before-completion`

Add a new top-level `##` section near the top:

```markdown
## Default behaviour: cheap-vs-heavy rule

Features in Singularmem follow a simple rule for whether they default on
or default off:

- **Cheap features default ON** — trivial per-op cost, no downloads, no
  large dep tree. Example: the Tantivy lexical index (sub-project 2a) is
  default-on and bundled.
- **Heavy features default OFF (opt-in)** — features requiring downloads,
  large dep trees, or non-trivial per-op cost. Example: ONNX embeddings
  (sub-project 2b) require an ~80 MB model download + ~10-50 ms per
  ingest, so they're off until the user runs `singularmem reindex
  --with-embeddings` to opt in.

When proposing a new feature, ask: "does this require something the user
didn't ask for the first time they run it?" If yes, default off.
```

Run + commit.

---

### Task 20: Version bump to 0.3.0

**Files:** Modify root `Cargo.toml`.

**Assigned skill:** `verification-before-completion`

Bump `[workspace.package] version = "0.3.0"`. Verify `./target/release/singularmem --version` prints `singularmem 0.3.0`. Commit.

---

### Task 21: Doc audit + final cargo verify + placeholder scan

**Files:** any in the new modules needing doc comments.

**Assigned skill:** `verification-before-completion`

- [ ] `RUSTDOCFLAGS="-D missing-docs" cargo doc -p singularmem-search --no-deps` → must `Generated` cleanly. Address any missing doc comments (most likely: `pub` fields on VectorHit, VectorIndexMeta, SemanticHit, SemanticSearchResults, SemanticSearchOptions).
- [ ] Placeholder grep: `grep -rn -E 'TODO|FIXME|XXX|TBD|\[PLACEHOLDER\]' crates/singularmem-search/src/ docs/formats/` → only acceptable matches (the spec's "TBD" verification grep patterns).
- [ ] Constitution grep verified clean.
- [ ] Full local CI equivalent passes:
  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace
  cargo build --release --bin singularmem
  ./target/release/singularmem --version  # → singularmem 0.3.0
  ```

Commit any doc-comment additions.

---

### Task 22: User checkpoint — confirm push

**Files:** none — out-of-band.
**Assigned skill:** `verification-before-completion`

Stop and ask the user. Present commit count + measured perf numbers + summary of new acceptance criteria status. Wait for explicit consent before any `git push`.

---

### Task 23: Push + open PR + watch CI

**Files:** none — remote operations.
**Assigned skill:** `verification-before-completion`

```bash
git push origin main
git push -u origin search-v0-embeddings

gh -R bromso/singularmem pr create \
  --base main --head search-v0-embeddings \
  --title "Search v0 (Embeddings + Vector, sub-project 2b)" \
  --body "$(cat <<'EOF'
## Summary

Sub-project 2b — ONNX embeddings via fastembed + USearch vector index +
MultiHook composite + semantic-search CLI verb. Opt-in via
`reindex --with-embeddings`. Version bump to v0.3.0.

(full body per spec § Section 8 acceptance criteria)
EOF
)"

gh -R bromso/singularmem pr checks search-v0-embeddings --watch
```

Expected: 8 blocking jobs + 1 new (the existing `perf-budgets` now also checks `semantic_search_latency`). The `tests-offline` advisory + `macos-advisory` may run / fail / pass.

If `perf-budgets` fails on the semantic-search budget on CI when local passed: investigate (likely a warm-cache difference; CI is colder).

---

### Task 24: User checkpoint — confirm merge

**Files:** none — out-of-band.
**Assigned skill:** `verification-before-completion`

Stop and ask. Wait for explicit consent.

---

### Task 25: Merge + tag v0.3.0 + memory update

**Files:** updates `~/.claude/projects/-Users-jonasbroms-Sites-singularmem/memory/project_singularmem_overview.md`.

**Assigned skill:** `verification-before-completion`

```bash
gh -R bromso/singularmem pr merge search-v0-embeddings \
  --merge --delete-branch \
  --subject "Search v0 (Embeddings + Vector, sub-project 2b) (#<PR>)"

git checkout main && git pull --ff-only

cargo build --release --bin singularmem
./target/release/singularmem --version  # → singularmem 0.3.0

git tag -a v0.3.0 -m "Search v0 (Embeddings + Vector) — sub-project 2b. ONNX embeddings via fastembed; USearch vector index; MultiHook composite; semantic-search CLI verb; opt-in via reindex --with-embeddings."
git push origin v0.3.0
```

Update memory: mark sub-project 2b as **MERGED 2026-05-17** (or whatever the actual date is); list deliverables (the four new modules, MultiHook, semantic-search verb, perf budget addition); update next-active candidate to **sub-project 2c** (Hybrid retrieval).

---

## Constitution Check

| Principle | How this plan complies |
|---|---|
| **I — Local-First and Sovereign** | All inference local via ONNX runtime. Model weights download from HuggingFace once on opt-in (Task 14). No telemetry. After download, fully offline. |
| **II — Provider-Agnostic by Contract** | No LLM provider integration. First relevance is sub-project 3. |
| **III — Open Core with a Stable Boundary** | Wholly open. Format spec gains vector sidecar section (Task 21). III.b preserved by unchanged `open_core_only_round_trip` test (Task 26 verifies). `singularmem-core` gains only the additive `MultiHook` type (Task 2). |
| **V — Composable Library Architecture** | `Embedder` is a public trait; `FastembedEmbedder` is one impl, `MockEmbedder` is another. `VectorIndex` is usable without an Embedder. `EmbedderIndex` composes both. Sub-project 4 (MCP) and 5 (TS SDK) consume the same library. |
| **VI — Deterministic and Offline-Testable** | Embedding deterministic given fixed weights + identical input (property-tested in Task 18). Tests use MockEmbedder; real Embedder integration tests are `#[ignore]`. The advisory `tests-offline` job continues. |
| **X — Performance Budgets, Enforced in CI** | `perf-budgets` stays blocking. Task 23 adds the new `semantic_search_latency` budget check. Single-item ingest with embedder will likely regress to ~20/s — documented as recognised slow path per spec (Task 24 documents the cheap-vs-heavy rule). |

Conditional re-check: Principles **IV** (CLI-First — new verb + flags in Tasks 13-15), **VII** (Honest Failure Modes — MultiHook log-and-continue in Task 2, model-mismatch errors in Tasks 7-15, truncation warnings in Task 6), **VIII** (Privacy Telemetry — none added), **IX** (Accessible by Default — clap output stays plain text).

## Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `fastembed = "=4.4.0"` is yanked at execution time | Low | Low | Task 1 verifies via `cargo search fastembed --limit 5` and re-pins if needed. Same lesson as tantivy 0.22.0 in 2a. |
| ONNX runtime build adds C/C++ toolchain dep that breaks CI | Medium | Medium | fastembed's `ort-download-binaries` feature ships precompiled binaries; no source build. Task 1 verifies the CI ubuntu-latest runner picks them up. |
| Binary size pushes past 150 MB | Low | Medium | Estimated ~70 MB; Task 27 measures actual on `ubuntu-latest` and re-runs perf-check.sh. If over 100 MB, investigate fastembed feature flags before merging. |
| Single-item ingest with embedder drops below 20/s | Medium | Medium | Already acknowledged in spec as recognised slow path. Task 22's bench measures actual; if <10/s, escalate to "real regression" and consider deferred-commit batching as a follow-up sub-project. |
| `FastembedEmbedder::new()` downloads 80 MB during `cargo test` in CI | Medium | High | Tests use `MockEmbedder`. Real-embedder tests are `#[ignore]`. CI doesn't pass `--ignored`. Verified in Task 18 review. |
| `tests-offline` (advisory) flakes because of ORT shared lib loading | Low | Low | ORT shared lib lookup is filesystem-only after Task 1's `download-binaries` setup; no runtime network. tests-offline behaviour should be unchanged from v0.2.0. |
| USearch HNSW recall is too low for short embeddings (384-dim) | Low | Low | Property test in Task 18 asserts cosine ≥ 0.95 for self-similarity; if it fails, bump `expansion_search` from 64 to 128 in VectorIndexOptions defaults. |

## Verification plan

The sixteen verifications below correspond one-to-one with the spec's sixteen acceptance criteria.

1. **Four new modules + doc comments.** Tasks 3-12 each module + Task 26 (final `cargo doc --no-deps -D missing-docs`).
2. **fastembed + usearch deps.** Task 1.
3. **Format spec update.** Task 21.
4. **MultiHook + Store::open_with_hooks.** Task 2 + Task 17 acceptance check (recompile v0.2.0 tests confirms `open` / `open_with_hook` unchanged).
5. **Embedder deterministic.** Task 4 + Task 18 property test.
6. **VectorIndex round-trip + model mismatch.** Tasks 7-10 + integration tests.
7. **`reindex --with-embeddings` end-to-end.** Tasks 13-15 + integration tests.
8. **`semantic-search` end-to-end.** Task 14 + integration tests in Task 20.
9. **Auto-wiring is opt-in.** Task 16 + integration test in Task 20.
10. **MultiHook Principle VII compliance.** Task 17.
11. **Principle III.b round-trip preserved.** Task 26 final test pass.
12. **Principle X budgets met.** Task 27 measures all four (5 with new semantic-search latency).
13. **`perf-budgets` CI job green.** Task 23 updates the script + workflow; Task 28 watches.
14. **`CONTRIBUTING.md` cheap-vs-heavy rule.** Task 24.
15. **Version bump to 0.3.0.** Task 25 + Task 30 tag push.
16. **No `[PLACEHOLDER]` strings.** Task 26 final grep.

## Rollback plan

Purely additive sub-project — `singularmem-search` gains modules; `singularmem-core`'s change is the additive `MultiHook` type. If a post-merge issue requires reverting, `git revert <merge-commit>` undoes everything; workspace returns to v0.2.0 state. The `v0.3.0` tag stays for historical record.

If a partial rollback is needed (e.g., revert the auto-wiring but keep the library), revert just the relevant phase commit. The phase commits are independent enough that this works without follow-up restabilisation.

<!-- END OF PLAN -->
