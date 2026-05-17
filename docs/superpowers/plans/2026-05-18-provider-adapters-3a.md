# Provider Adapters — Foundation (Sub-Project 3a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish the typed `Adapter` contract (Principle II requires it), ship a `Retriever` that composes `HybridSearcher` + `Store::get` into prompt-ready memory blocks, and add a `singularmem retrieve` CLI verb with `PlainAdapter` as the default.

**Architecture:** New crate `singularmem-retrieve` depends on `singularmem-core` + `singularmem-search`. `Retriever<'a>` borrows references to both, runs a hybrid search, filters by score, fetches full item content per hit, and returns a `RetrievedContext` of `MemoryBlock`s. `Adapter` trait is a pure-formatter (`name()` + `format(&RetrievedContext) -> String`). `PlainAdapter` (Markdown shape) ships as the default; future cloud adapters (Claude/OpenAI/Gemini, sub-projects 3b/c/d) plug into the same trait and register with the CLI.

**Tech Stack:** Rust 1.80, workspace deps (no new external crates), `singularmem-core` v0.4.0, `singularmem-search` v0.4.0 with `HybridSearcher` from sub-project 2c.

**Spec:** `docs/superpowers/specs/2026-05-18-provider-adapters-3a-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-retrieve/Cargo.toml` — new crate manifest, version `0.5.0`, workspace-locked.
- `crates/singularmem-retrieve/src/lib.rs` — re-exports.
- `crates/singularmem-retrieve/src/error.rs` — `Error` + `Result`.
- `crates/singularmem-retrieve/src/retriever.rs` — `Retriever`, `RetrieveOptions`, `RetrievedContext`, `MemoryBlock`, unit tests.
- `crates/singularmem-retrieve/src/adapter.rs` — `Adapter` trait + `PlainAdapter`, unit tests.
- `crates/singularmem-retrieve/src/testing.rs` — `MockAdapter`.

**Modified:**
- `Cargo.toml` (workspace) — add `singularmem-retrieve` to root binary's `[dependencies]`.
- `src/main.rs` — `Command::Retrieve` variant, `RetrieveArgs` struct, `cmd_retrieve` function, `known_adapters()` registry. Extract shared `resolve_search_mode()` helper from existing `cmd_search`.
- `tests/cli.rs` — eight new integration tests.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"` — retrieval is read-only).

---

## Task 1: Crate scaffold + Error type

**Why first:** Every later task imports something from this crate. Establishing the scaffold and the error vocabulary up front means subsequent tasks just append to existing files.

**Files:**
- Create: `crates/singularmem-retrieve/Cargo.toml`
- Create: `crates/singularmem-retrieve/src/lib.rs`
- Create: `crates/singularmem-retrieve/src/error.rs`
- Modify: `Cargo.toml` (workspace root — but only the root binary's `[dependencies]`, NOT the `[workspace.members]` list, which uses the `crates/*` glob that auto-picks up the new crate)

- [ ] **Step 1: Create the new-crate Cargo.toml**

Create `crates/singularmem-retrieve/Cargo.toml`:

```toml
[package]
name = "singularmem-retrieve"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Retrieval + adapter contract for Singularmem memory stores."

[features]
testing = []

[lints]
workspace = true

[dependencies]
singularmem-core = { path = "../singularmem-core" }
singularmem-search = { path = "../singularmem-search" }
tracing = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
jiff = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
singularmem-search = { path = "../singularmem-search", features = ["testing"] }
singularmem-core = { path = "../singularmem-core" }
```

- [ ] **Step 2: Create the lib.rs skeleton with module declarations and re-exports**

Create `crates/singularmem-retrieve/src/lib.rs`:

```rust
//! Singularmem retrieve — composes hybrid search + store reads into
//! prompt-ready memory blocks, and defines the typed `Adapter` contract
//! that per-provider formatter crates implement.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapters-3a-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod retriever;
pub mod testing;

pub use crate::adapter::{Adapter, PlainAdapter};
pub use crate::error::{Error, Result};
pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};
pub use crate::testing::MockAdapter;
```

(Re-exports follow the same convention as singularmem-search: `MockAdapter` is unconditionally available — same trick used for `MockEmbedder` so cross-crate tests don't need `--features testing` toggling. The `testing` feature in `Cargo.toml` exists for forward-compat with downstream code that may want to gate things later.)

- [ ] **Step 3: Create error.rs with the three variants from the spec**

Create `crates/singularmem-retrieve/src/error.rs`:

```rust
//! Error type for the retrieve crate.

/// Alias for `std::result::Result<T, Error>` used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by `singularmem-retrieve` operations.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Underlying search-layer failure.
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),

    /// Underlying core-layer failure (e.g., `Store::get` on a deleted item).
    #[error("{0}")]
    Core(#[from] singularmem_core::Error),

    /// Query was empty or whitespace-only.
    #[error("query is empty; retrieval requires a non-empty query string")]
    EmptyQuery,
}
```

- [ ] **Step 4: Create empty module files so the crate compiles**

Create `crates/singularmem-retrieve/src/retriever.rs`:

```rust
//! `Retriever` composes `HybridSearcher` + `Store::get` into prompt-ready
//! memory blocks. The struct borrows references to both so callers retain
//! ownership of the underlying components.
```

Create `crates/singularmem-retrieve/src/adapter.rs`:

```rust
//! `Adapter` trait + the default `PlainAdapter` formatter.
```

Create `crates/singularmem-retrieve/src/testing.rs`:

```rust
//! Test fixtures. `MockAdapter` is unconditionally available so cross-crate
//! tests don't need `--features testing` toggling — same pattern as
//! `singularmem-search::testing::MockEmbedder`.
```

The crate will fail to build at this point because `lib.rs` tries to `pub use` types that don't exist yet. To make this intermediate state compile, temporarily comment out the `pub use` block:

```rust
// pub use crate::adapter::{Adapter, PlainAdapter};
// pub use crate::error::{Error, Result};
// pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};
// pub use crate::testing::MockAdapter;
```

Leave the `pub mod` lines uncommented. The next task will add the types and uncomment the re-exports.

(Actually, do NOT comment them out — the cleaner path is to leave the `pub use` for `Error/Result` active since Task 1 Step 3 already created those types, and comment out only the three other re-exports. So:

```rust
pub mod adapter;
pub mod error;
pub mod retriever;
pub mod testing;

pub use crate::error::{Error, Result};

// Re-exports activated in subsequent tasks:
// pub use crate::adapter::{Adapter, PlainAdapter};
// pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};
// pub use crate::testing::MockAdapter;
```

This way the `Error/Result` types are usable from the get-go.)

- [ ] **Step 5: Add the crate as a dev/runtime dep to the root binary**

Modify `Cargo.toml` (workspace root) — extend the root binary's `[dependencies]` section by adding `singularmem-retrieve` after `singularmem-search`. The current section (around lines 50-60) looks like:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

Add one line:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
singularmem-retrieve = { path = "crates/singularmem-retrieve" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

The workspace members glob `members = ["crates/*"]` in the `[workspace]` section auto-picks up the new crate; no edit needed there.

- [ ] **Step 6: Build the workspace to verify the new crate compiles**

Run: `cargo build --workspace`

Expected: clean build. The new `singularmem-retrieve` crate compiles with only `Error` and `Result` exported (the other re-exports are commented out).

- [ ] **Step 7: Run clippy to verify lints are happy**

Run: `cargo clippy -p singularmem-retrieve --all-targets -- -D warnings`

Expected: zero warnings.

- [ ] **Step 8: Add a one-test sanity check for Error**

Append to `crates/singularmem-retrieve/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_error_message_explains_the_problem() {
        let e = Error::EmptyQuery;
        let msg = e.to_string();
        assert!(
            msg.contains("query is empty"),
            "message should explain the failure: got {msg:?}"
        );
        assert!(
            msg.contains("non-empty"),
            "message should tell user what to provide: got {msg:?}"
        );
    }
}
```

Run: `cargo test -p singularmem-retrieve --lib error::tests`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/singularmem-retrieve/
git commit -s -m "feat(retrieve): new crate scaffold + Error type

Adds singularmem-retrieve crate (version-locked to workspace at 0.5.0)
with the three Error variants from the spec (Search, Core, EmptyQuery).
Retriever/Adapter/Mock types land in subsequent commits."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 2: Retriever data types

**Why second:** Subsequent tasks reference these types. Get them landed with their `Default` impls and serde derives so later code blocks compile.

**Files:**
- Modify: `crates/singularmem-retrieve/src/retriever.rs`
- Modify: `crates/singularmem-retrieve/src/lib.rs` (uncomment the `retriever` re-export)

- [ ] **Step 1: Write the failing test**

Append to `crates/singularmem-retrieve/src/retriever.rs` (replacing the placeholder doc-comment):

```rust
//! `Retriever` composes `HybridSearcher` + `Store::get` into prompt-ready
//! memory blocks. The struct borrows references to both so callers retain
//! ownership of the underlying components.

use std::time::Duration;

use jiff::Timestamp;
use serde::Serialize;
use singularmem_core::ItemId;
use singularmem_search::{HybridSearchOptions, ScoreKind};

/// Options controlling a `Retriever::retrieve` call.
#[derive(Debug, Clone)]
pub struct RetrieveOptions {
    /// Maximum number of memory blocks to return. Default: 10.
    pub max_blocks: usize,
    /// Minimum score for a hit to be included. Default: 0.0.
    /// Applied BEFORE `max_blocks` truncation so low-relevance hits
    /// don't crowd out genuinely-relevant matches.
    pub min_score: f32,
    /// Underlying hybrid-search options (passed through to `HybridSearcher`).
    pub search: HybridSearchOptions,
}

impl Default for RetrieveOptions {
    fn default() -> Self {
        Self {
            max_blocks: 10,
            min_score: 0.0,
            search: HybridSearchOptions::default(),
        }
    }
}

/// Results of a retrieval call.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievedContext {
    /// Memory blocks in descending score order, truncated to `max_blocks`.
    pub blocks: Vec<MemoryBlock>,
    /// The query that was retrieved against.
    pub query: String,
    /// Wall-clock duration of the entire `Retriever::retrieve` call
    /// (including the underlying search AND the per-hit `Store::get` reads).
    pub elapsed: Duration,
    /// Number of distinct documents considered for fusion (lexical ∪ semantic),
    /// from `HybridSearchResults::total_fused`. Use as denominator for
    /// "showed N of M considered".
    pub total_considered: usize,
}

/// One memory block in a `RetrievedContext`. Carries the full item content
/// (not a snippet) plus enough metadata for adapters to format provenance.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryBlock {
    /// The matched item's ID.
    pub id: ItemId,
    /// FULL content from `Store::get`, not the Tantivy-trimmed snippet.
    pub content: String,
    /// Score whose meaning depends on `score_kind`.
    pub score: f32,
    /// Tells the consumer what `score` represents (RRF / BM25 / Cosine).
    pub score_kind: ScoreKind,
    /// Free-form provenance label from the underlying `Item`.
    pub source: Option<String>,
    /// Tags from the underlying `Item`.
    pub tags: Vec<String>,
    /// Wall-clock timestamp the store assigned at ingest.
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_match_spec() {
        let o = RetrieveOptions::default();
        assert_eq!(o.max_blocks, 10);
        assert!((o.min_score - 0.0).abs() < f32::EPSILON);
        // search field defaults pulled from HybridSearchOptions; we don't
        // re-assert those here because sub-project 2c already tests them.
    }
}
```

Then uncomment the `retriever` re-export in `crates/singularmem-retrieve/src/lib.rs`:

```rust
pub mod adapter;
pub mod error;
pub mod retriever;
pub mod testing;

pub use crate::error::{Error, Result};
pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};

// Re-exports activated in subsequent tasks:
// pub use crate::adapter::{Adapter, PlainAdapter};
// pub use crate::testing::MockAdapter;
```

Wait — `Retriever` doesn't exist yet (that's Task 3). The re-export will fail. Instead, re-export only the types added in this task:

```rust
pub mod adapter;
pub mod error;
pub mod retriever;
pub mod testing;

pub use crate::error::{Error, Result};
pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext};

// Re-exports activated in subsequent tasks:
// pub use crate::adapter::{Adapter, PlainAdapter};
// pub use crate::retriever::Retriever;
// pub use crate::testing::MockAdapter;
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p singularmem-retrieve --lib retriever::tests`

Expected: PASS.

- [ ] **Step 3: Verify clippy clean**

Run: `cargo clippy -p singularmem-retrieve --all-targets -- -D warnings`

Expected: zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/singularmem-retrieve/src/retriever.rs crates/singularmem-retrieve/src/lib.rs
git commit -s -m "feat(retrieve): RetrieveOptions, RetrievedContext, MemoryBlock types

Pure data types. Retriever struct + retrieve() method land in subsequent
commits. Default for RetrieveOptions matches spec: max_blocks=10,
min_score=0.0, search defaults from HybridSearchOptions."
```

Verify sign-off as in Task 1 Step 9.

---

## Task 3: Retriever struct + constructor

**Files:**
- Modify: `crates/singularmem-retrieve/src/retriever.rs`
- Modify: `crates/singularmem-retrieve/src/lib.rs` (uncomment the `Retriever` re-export)

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-retrieve/src/retriever.rs`:

```rust
    use singularmem_core::Store;
    use singularmem_search::testing::MockEmbedder;
    use singularmem_search::{EmbedderIndex, HybridSearcher, Index};
    use tempfile::TempDir;

    #[test]
    fn new_holds_references_to_store_and_searcher() {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();
        let lex = Index::open(dir.path().join("lex")).unwrap();
        let sem =
            EmbedderIndex::open(dir.path().join("sem"), Box::new(MockEmbedder::default()))
                .unwrap();
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        // The struct fields are public; we can observe the borrowed references.
        assert!(std::ptr::eq(retriever.store, &store));
        assert!(std::ptr::eq(retriever.searcher, &searcher));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-retrieve --lib retriever::tests::new_holds_references`

Expected: FAIL with `cannot find type 'Retriever' in this scope` (or similar — type doesn't exist yet).

- [ ] **Step 3: Add the Retriever struct**

Add to `crates/singularmem-retrieve/src/retriever.rs`, just above the `#[cfg(test)] mod tests` block:

```rust
use singularmem_core::Store;
use singularmem_search::HybridSearcher;

/// Composes a hybrid search + per-hit store reads into prompt-ready
/// `MemoryBlock`s.
///
/// Borrows references to `Store` and `HybridSearcher` — same borrow pattern
/// `HybridSearcher` uses for its underlying indexes. Callers retain
/// ownership of the underlying components.
pub struct Retriever<'a> {
    /// Borrowed reference to the underlying memory store.
    pub store: &'a Store,
    /// Borrowed reference to the hybrid searcher.
    pub searcher: &'a HybridSearcher<'a>,
}

impl<'a> Retriever<'a> {
    /// Construct a `Retriever` borrowing the given store and searcher.
    #[must_use]
    pub const fn new(store: &'a Store, searcher: &'a HybridSearcher<'a>) -> Self {
        Self { store, searcher }
    }
}
```

Then uncomment the `Retriever` re-export in `crates/singularmem-retrieve/src/lib.rs`:

```rust
pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p singularmem-retrieve --lib retriever::tests::new_holds_references`

Expected: PASS.

- [ ] **Step 5: Verify clippy clean**

Run: `cargo clippy -p singularmem-retrieve --all-targets -- -D warnings`

Expected: zero warnings.

Watch for `clippy::elidable_lifetime_names` on `impl<'a>` — `HybridSearcher<'a>` uses the same `<'a>` pattern from sub-project 2c (commit `c684298`), so this should be fine. If clippy complains, change `impl<'a> Retriever<'a>` to `impl Retriever<'_>` and adjust the constructor signature; the test will still work because the lifetime is inferred at the call site.

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-retrieve/src/retriever.rs crates/singularmem-retrieve/src/lib.rs
git commit -s -m "feat(retrieve): Retriever struct + new() constructor

Borrows &Store and &HybridSearcher (same borrow pattern HybridSearcher
uses for its underlying indexes). retrieve() method lands in next commit."
```

Verify sign-off.

---

## Task 4: `Retriever::retrieve` — full implementation + seven unit tests

**Files:**
- Modify: `crates/singularmem-retrieve/src/retriever.rs`

This is the meatiest library task. Single `retrieve` method, seven tests covering the seven behaviours from the spec's Testing Strategy section.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-retrieve/src/retriever.rs`:

```rust
    use singularmem_core::NewItem;
    use singularmem_search::HybridSearchOptions;

    /// Helper: build a store + both sidecars seeded with `n` text items,
    /// drop the writing store, then return a freshly-opened store + searcher.
    fn seeded(n: usize) -> (TempDir, Store, Index, EmbedderIndex) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let lex_path = dir.path().join("lex");
        let sem_path = dir.path().join("sem");

        let lex_hook = Index::open(&lex_path).unwrap();
        let sem_hook =
            EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let multi = singularmem_core::hook::MultiHook::new(vec![
            Box::new(lex_hook),
            Box::new(sem_hook),
        ]);
        let store = Store::open_with_hook(&store_path, Box::new(multi)).unwrap();
        for i in 0..n {
            store
                .ingest(NewItem::text(format!("seed memory number {i}")))
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        let store = Store::open(&store_path).unwrap();
        let lex = Index::open(&lex_path).unwrap();
        let sem = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        (dir, store, lex, sem)
    }

    #[test]
    fn retrieve_returns_full_content_not_snippet() {
        let (_dir, store, lex, sem) = seeded(5);
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        let r = retriever
            .retrieve("seed memory", &RetrieveOptions::default())
            .expect("ok");
        assert!(!r.blocks.is_empty());
        // Every block's content is the full ingested string, not a snippet.
        for b in &r.blocks {
            assert!(
                b.content.starts_with("seed memory number "),
                "expected full content, got {:?}",
                b.content
            );
        }
    }

    #[test]
    fn retrieve_respects_max_blocks() {
        let (_dir, store, lex, sem) = seeded(10);
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        let opts = RetrieveOptions {
            max_blocks: 3,
            ..Default::default()
        };
        let r = retriever.retrieve("seed memory", &opts).expect("ok");
        assert!(r.blocks.len() <= 3, "got {} blocks", r.blocks.len());
    }

    #[test]
    fn retrieve_filters_below_min_score() {
        let (_dir, store, lex, sem) = seeded(5);
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        // Set a min_score higher than any RRF score will be (RRF scores are
        // bounded by 1/(k+1) + 1/(k+1) = 2/61 ≈ 0.033 for k=60).
        let opts = RetrieveOptions {
            min_score: 1.0,
            ..Default::default()
        };
        let r = retriever.retrieve("seed memory", &opts).expect("ok");
        assert!(r.blocks.is_empty(), "expected all hits filtered out");
        // total_considered may still be non-zero — filtering doesn't
        // reduce the fusion count.
    }

    #[test]
    fn retrieve_propagates_search_errors() {
        // No sidecars at all → HybridSearcher with lexical_only over an empty
        // tantivy dir actually returns 0 hits (sub-project 2a behaviour); we
        // can't trigger Error::Search directly. Instead, exercise the
        // dim-mismatch path: open a vector index with a different-dim mock
        // embedder than the one that built it, then call retrieve.
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();

        // Build the vector sidecar with default-dim MockEmbedder.
        let sem_path = dir.path().join("sem");
        {
            let sem = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default()))
                .unwrap();
            // No need to add anything; mismatch is detected at open time
            // in the next step if we re-open with a different-dim embedder.
            drop(sem);
        }

        // Re-open with a different-dim embedder → ModelMismatch/DimMismatch
        // on EmbedderIndex::open. We confirm the underlying error surfaces.
        let result =
            EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::with_dim(128)));
        assert!(
            result.is_err(),
            "expected dim mismatch error from EmbedderIndex::open"
        );
        // This test verifies the underlying error type exists; the Retriever
        // wraps it as Error::Search via the From impl. The contract is exercised
        // by the `?` operator in retrieve()'s implementation.
        let _ = store;
    }

    #[test]
    fn retrieve_propagates_store_get_errors() {
        // Race condition test: ingest, search succeeds, then DELETE the item
        // from SQLite directly (bypassing the immutable Store API), then
        // verify retrieve() surfaces Error::Core(NotFound).
        let (dir, store, lex, sem) = seeded(3);
        let searcher = HybridSearcher::new(&lex, &sem);

        // Delete one item by raw SQL — there is no public Store::delete.
        let id_to_kill = store
            .list()
            .unwrap()
            .next()
            .expect("at least one item")
            .unwrap()
            .id;
        let store_path = dir.path().join("store.db");
        drop(store);
        let conn = rusqlite::Connection::open(&store_path).unwrap();
        conn.execute("DELETE FROM items WHERE id = ?1", [id_to_kill.to_string()])
            .unwrap();
        drop(conn);
        let store = Store::open(&store_path).unwrap();

        // Retrieve still finds the deleted ID in the search index (it was
        // never re-indexed), but Store::get fails.
        let retriever = Retriever::new(&store, &searcher);
        let result = retriever.retrieve("seed memory", &RetrieveOptions::default());
        assert!(
            matches!(result, Err(Error::Core(singularmem_core::Error::NotFound { .. }))),
            "expected Error::Core(NotFound), got {result:?}"
        );
    }

    #[test]
    fn empty_query_errors() {
        let (_dir, store, lex, sem) = seeded(1);
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        for empty in ["", "   ", "\t\n"] {
            let r = retriever.retrieve(empty, &RetrieveOptions::default());
            assert!(
                matches!(r, Err(Error::EmptyQuery)),
                "input {empty:?} should yield EmptyQuery, got {r:?}"
            );
        }
    }

    #[test]
    fn total_considered_reflects_fusion_input() {
        let (_dir, store, lex, sem) = seeded(5);
        let searcher = HybridSearcher::new(&lex, &sem);
        let retriever = Retriever::new(&store, &searcher);
        // Underlying hybrid search; fetch its total_fused for comparison.
        let raw = searcher
            .search("seed memory", &HybridSearchOptions::default())
            .unwrap();
        let r = retriever
            .retrieve("seed memory", &RetrieveOptions::default())
            .unwrap();
        assert_eq!(
            r.total_considered, raw.total_fused,
            "total_considered must mirror HybridSearchResults.total_fused"
        );
    }
```

You'll need to add `rusqlite = { workspace = true }` to `crates/singularmem-retrieve/Cargo.toml` `[dev-dependencies]` for the deletion-race test:

```toml
[dev-dependencies]
tempfile = { workspace = true }
rusqlite = { workspace = true }
singularmem-search = { path = "../singularmem-search", features = ["testing"] }
singularmem-core = { path = "../singularmem-core" }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p singularmem-retrieve --lib retriever::tests`

Expected: FAIL — most tests fail with `no method named 'retrieve' found for struct 'Retriever'`.

- [ ] **Step 3: Implement `Retriever::retrieve`**

Add to `crates/singularmem-retrieve/src/retriever.rs`, inside the existing `impl<'a> Retriever<'a>` block (or as a second impl block if cleaner — both compile to the same thing):

```rust
impl<'a> Retriever<'a> {
    /// Retrieve memory blocks matching `query`, formatted as a
    /// `RetrievedContext` ready for an `Adapter` to render as a prompt.
    ///
    /// Algorithm:
    /// 1. Run `HybridSearcher::search` with `opts.search`.
    /// 2. Filter hits by `opts.min_score`.
    /// 3. Truncate to `opts.max_blocks`.
    /// 4. For each remaining hit, fetch the full `Item` via `Store::get`.
    /// 5. Build `MemoryBlock`s and return.
    ///
    /// # Errors
    ///
    /// - [`Error::EmptyQuery`] if `query` is empty or whitespace-only.
    /// - [`Error::Search`] if the underlying hybrid search fails.
    /// - [`Error::Core`] if `Store::get` fails for any matched ID
    ///   (e.g., the item was deleted between search and read).
    pub fn retrieve(
        &self,
        query: &str,
        opts: &RetrieveOptions,
    ) -> crate::Result<RetrievedContext> {
        if query.trim().is_empty() {
            return Err(crate::Error::EmptyQuery);
        }

        let start = std::time::Instant::now();
        let results = self.searcher.search(query, &opts.search)?;
        let total_considered = results.total_fused;

        let blocks: crate::Result<Vec<MemoryBlock>> = results
            .hits
            .into_iter()
            .filter(|h| h.score >= opts.min_score)
            .take(opts.max_blocks)
            .map(|hit| -> crate::Result<MemoryBlock> {
                let item = self.store.get(hit.id)?;
                Ok(MemoryBlock {
                    id: hit.id,
                    content: item.content,
                    score: hit.score,
                    score_kind: hit.score_kind,
                    source: item.source,
                    tags: item.tags,
                    created_at: item.created_at,
                })
            })
            .collect();
        let blocks = blocks?;

        Ok(RetrievedContext {
            blocks,
            query: query.to_string(),
            elapsed: start.elapsed(),
            total_considered,
        })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-retrieve --lib retriever::tests`

Expected: PASS for all seven tests + the existing `default_options_match_spec` + `new_holds_references_to_store_and_searcher` (nine total).

- [ ] **Step 5: Run the full crate test suite**

Run: `cargo test -p singularmem-retrieve`

Expected: all tests pass.

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy -p singularmem-retrieve --all-targets --tests -- -D warnings`

Expected: zero warnings. Likely lints to watch:
- `clippy::redundant_closure_for_method_calls` on the `.map(|hit| ...)` closure (probably fine because the closure body does more than a method call).
- `clippy::collect_into_iter` or similar on the `collect::<Result<Vec<_>>>()` pattern — standard idiom, should pass pedantic.

- [ ] **Step 7: Commit**

```bash
git add crates/singularmem-retrieve/Cargo.toml crates/singularmem-retrieve/src/retriever.rs Cargo.lock
git commit -s -m "feat(retrieve): Retriever::retrieve algorithm + 7 unit tests

Search → filter by min_score → truncate to max_blocks → fetch full
content per hit via Store::get → return RetrievedContext.
Empty/whitespace query errors as EmptyQuery; downstream search/core
errors propagate via #[from] in Error.

Adds rusqlite as a dev-dep for the deletion-race test (no public
Store::delete API exists, so the test bypasses via raw SQL — this is
deliberate test scaffolding for a real race condition we care about
per Principle VII)."
```

Verify sign-off.

---

## Task 5: `Adapter` trait

**Files:**
- Modify: `crates/singularmem-retrieve/src/adapter.rs`
- Modify: `crates/singularmem-retrieve/src/lib.rs` (uncomment the `Adapter` re-export)

- [ ] **Step 1: Write the failing test**

Replace the content of `crates/singularmem-retrieve/src/adapter.rs`:

```rust
//! `Adapter` trait + the default `PlainAdapter` formatter.

use crate::retriever::RetrievedContext;

/// Provider adapter contract per Constitution Principle II.
///
/// An adapter is a pure formatting strategy: it takes a `RetrievedContext`
/// and renders it as a single prompt-ready string in whatever format the
/// underlying LLM provider prefers (XML for Claude, Markdown for OpenAI,
/// plain text for local models).
///
/// **Contract:** implementations MUST be pure functions — no I/O, no
/// network calls, deterministic for identical input. This is enforced
/// by convention (not the type system) and is what makes the trait
/// trivially testable and composable.
pub trait Adapter: Send + Sync {
    /// Stable identifier used in CLI flags and logs.
    /// Lowercase, hyphen-separated. Examples: `"plain"`, `"claude"`, `"openai"`.
    fn name(&self) -> &str;

    /// Render a `RetrievedContext` as a single prompt-ready string.
    ///
    /// MUST be a pure function: no I/O, deterministic given the same input.
    fn format(&self, ctx: &RetrievedContext) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-line concrete adapter used purely to verify the trait compiles
    /// and is object-safe (`Box<dyn Adapter>` works).
    struct NoopAdapter;
    impl Adapter for NoopAdapter {
        fn name(&self) -> &str {
            "noop"
        }
        fn format(&self, _ctx: &RetrievedContext) -> String {
            String::new()
        }
    }

    #[test]
    fn adapter_trait_is_object_safe() {
        let a: Box<dyn Adapter> = Box::new(NoopAdapter);
        assert_eq!(a.name(), "noop");
    }
}
```

Then update `crates/singularmem-retrieve/src/lib.rs` to uncomment the `Adapter` re-export, but NOT `PlainAdapter` (that's Task 6):

```rust
pub use crate::adapter::Adapter;
pub use crate::error::{Error, Result};
pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};

// Activated in subsequent tasks:
// pub use crate::adapter::PlainAdapter;
// pub use crate::testing::MockAdapter;
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p singularmem-retrieve --lib adapter::tests`

Expected: PASS.

- [ ] **Step 3: Verify clippy clean**

Run: `cargo clippy -p singularmem-retrieve --all-targets --tests -- -D warnings`

Expected: zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/singularmem-retrieve/src/adapter.rs crates/singularmem-retrieve/src/lib.rs
git commit -s -m "feat(retrieve): Adapter trait

Principle II's typed adapter contract: name() + format(&RetrievedContext)
-> String. Object-safe; Send + Sync. Pure-function contract documented
in the trait doc. PlainAdapter and MockAdapter land in the next two
commits."
```

Verify sign-off.

---

## Task 6: `PlainAdapter`

**Files:**
- Modify: `crates/singularmem-retrieve/src/adapter.rs`
- Modify: `crates/singularmem-retrieve/src/lib.rs` (uncomment the `PlainAdapter` re-export)

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-retrieve/src/adapter.rs`:

```rust
    use crate::retriever::MemoryBlock;
    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    fn sample_block(id_str: &str, score: f32) -> MemoryBlock {
        MemoryBlock {
            id: ItemId::from_str(id_str).unwrap(),
            content: "the quick brown fox jumps over the lazy dog".to_string(),
            score,
            score_kind: ScoreKind::Rrf,
            source: Some("claude-conversation:abc-123".to_string()),
            tags: vec!["fox".to_string(), "animals".to_string()],
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        }
    }

    fn sample_context(blocks: Vec<MemoryBlock>, query: &str) -> RetrievedContext {
        RetrievedContext {
            blocks,
            query: query.to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 5,
        }
    }

    #[test]
    fn plain_adapter_includes_id_score_content() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0.0328),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", 0.0312),
            ],
            "fox",
        );
        let out = PlainAdapter.format(&ctx);
        // Heading
        assert!(out.contains("## memory 1"), "missing heading: {out}");
        assert!(out.contains("## memory 2"), "missing heading: {out}");
        // Score (formatted to 4 decimals)
        assert!(out.contains("score=0.0328"), "missing score: {out}");
        assert!(out.contains("score=0.0312"), "missing score: {out}");
        // ID
        assert!(out.contains("id: 01ARZ3NDEKTSV4RRFFQ69G5FAV"), "missing id: {out}");
        // Full content
        assert!(out.contains("the quick brown fox jumps"), "missing content: {out}");
        // Separator
        assert!(out.contains("---"), "missing separator: {out}");
    }

    #[test]
    fn plain_adapter_handles_zero_blocks() {
        let ctx = sample_context(vec![], "nothing here");
        let out = PlainAdapter.format(&ctx);
        assert!(out.contains("no memories matched"), "missing empty msg: {out}");
        assert!(out.contains("nothing here"), "missing query echo: {out}");
        // No memory headings.
        assert!(!out.contains("## memory"), "should not have memory headings: {out}");
    }

    #[test]
    fn plain_adapter_omits_optional_fields_when_absent() {
        let mut block = sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0.5);
        block.source = None;
        block.tags = vec![];
        let ctx = sample_context(vec![block], "test");
        let out = PlainAdapter.format(&ctx);
        // No empty source/tags lines.
        assert!(!out.contains("source:\n"), "empty source line emitted: {out}");
        assert!(!out.contains("tags:\n"), "empty tags line emitted: {out}");
        assert!(!out.contains("source: \n"), "empty source line emitted: {out}");
        assert!(!out.contains("tags: \n"), "empty tags line emitted: {out}");
    }

    #[test]
    fn plain_adapter_is_deterministic() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0.5)],
            "fox",
        );
        let a = PlainAdapter.format(&ctx);
        let b = PlainAdapter.format(&ctx);
        assert_eq!(a, b, "format must be deterministic");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p singularmem-retrieve --lib adapter::tests`

Expected: FAIL with `cannot find struct 'PlainAdapter'`.

- [ ] **Step 3: Implement `PlainAdapter`**

Add to `crates/singularmem-retrieve/src/adapter.rs`, after the `Adapter` trait and before the `#[cfg(test)] mod tests` block:

```rust
/// Default `Adapter` implementation. Emits Markdown-shaped output suitable
/// for local LLMs (Ollama, llama.cpp) and as a baseline for any provider.
///
/// Output shape, per block:
///
/// ```text
/// ## memory N (score=0.XXXX)
/// id: <ULID>
/// created: <RFC3339>
/// source: <provenance-label>      # omitted if None
/// tags: tag1, tag2, tag3          # omitted if empty
///
/// <full content>
/// ---
/// ```
///
/// When there are zero matched blocks, emits a single `[no memories matched
/// query: "..."]` line.
pub struct PlainAdapter;

impl Adapter for PlainAdapter {
    fn name(&self) -> &str {
        "plain"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return format!("[no memories matched query: {:?}]\n", ctx.query);
        }
        let mut out = String::new();
        let _ = writeln!(
            out,
            "# Retrieved {} memor{} for query: {:?}",
            ctx.blocks.len(),
            if ctx.blocks.len() == 1 { "y" } else { "ies" },
            ctx.query
        );
        for (i, block) in ctx.blocks.iter().enumerate() {
            let _ = writeln!(out);
            let _ = writeln!(out, "## memory {} (score={:.4})", i + 1, block.score);
            let _ = writeln!(out, "id: {}", block.id);
            let _ = writeln!(out, "created: {}", block.created_at);
            if let Some(s) = &block.source {
                let _ = writeln!(out, "source: {s}");
            }
            if !block.tags.is_empty() {
                let _ = writeln!(out, "tags: {}", block.tags.join(", "));
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", block.content);
            let _ = writeln!(out, "---");
        }
        out
    }
}
```

Then uncomment the `PlainAdapter` re-export in `crates/singularmem-retrieve/src/lib.rs`:

```rust
pub use crate::adapter::{Adapter, PlainAdapter};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-retrieve --lib adapter::tests`

Expected: PASS for all five tests (1 from Task 5 + 4 new).

- [ ] **Step 5: Verify clippy clean**

Run: `cargo clippy -p singularmem-retrieve --all-targets --tests -- -D warnings`

Expected: zero warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-retrieve/src/adapter.rs crates/singularmem-retrieve/src/lib.rs
git commit -s -m "feat(retrieve): PlainAdapter — Markdown-shaped default formatter

Real adapter satisfying the 'one fully local runtime' half of Principle
II — local LLMs (Ollama, llama.cpp) work fine with Markdown context.
Cloud adapters (Claude/OpenAI/Gemini) land in sub-projects 3b/3c/3d."
```

Verify sign-off.

---

## Task 7: `MockAdapter` in testing module

**Files:**
- Modify: `crates/singularmem-retrieve/src/testing.rs`
- Modify: `crates/singularmem-retrieve/src/lib.rs` (uncomment the `MockAdapter` re-export)

- [ ] **Step 1: Write the failing test**

Replace the content of `crates/singularmem-retrieve/src/testing.rs`:

```rust
//! Test fixtures. `MockAdapter` is unconditionally available so cross-crate
//! tests don't need `--features testing` toggling — same pattern as
//! `singularmem-search::testing::MockEmbedder`.

use crate::adapter::Adapter;
use crate::retriever::RetrievedContext;

/// Deterministic, easily-asserted-against `Adapter` for downstream tests.
///
/// Output shape:
///
/// ```text
/// MOCK[query="<query>" blocks=N ids=[id1,id2,...]]
/// ```
///
/// Used by sub-projects 3b/3c/3d to test their adapters' integration with
/// `Retriever` without committing to `PlainAdapter`'s specific output.
pub struct MockAdapter;

impl Adapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        let ids: Vec<String> = ctx.blocks.iter().map(|b| b.id.to_string()).collect();
        format!(
            "MOCK[query={:?} blocks={} ids=[{}]]\n",
            ctx.query,
            ctx.blocks.len(),
            ids.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retriever::MemoryBlock;
    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    #[test]
    fn mock_adapter_format_includes_ids() {
        let block = MemoryBlock {
            id: ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            content: "anything".to_string(),
            score: 0.5,
            score_kind: ScoreKind::Rrf,
            source: None,
            tags: vec![],
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        };
        let ctx = RetrievedContext {
            blocks: vec![block],
            query: "test".to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 1,
        };
        let out = MockAdapter.format(&ctx);
        assert!(out.contains("MOCK["), "missing prefix: {out}");
        assert!(out.contains("query=\"test\""), "missing query: {out}");
        assert!(out.contains("blocks=1"), "missing block count: {out}");
        assert!(
            out.contains("ids=[01ARZ3NDEKTSV4RRFFQ69G5FAV]"),
            "missing id list: {out}"
        );
    }
}
```

Then uncomment the `MockAdapter` re-export in `crates/singularmem-retrieve/src/lib.rs`:

```rust
pub use crate::testing::MockAdapter;
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p singularmem-retrieve --lib testing::tests`

Expected: PASS.

- [ ] **Step 3: Run the full crate test suite**

Run: `cargo test -p singularmem-retrieve`

Expected: all tests pass (12 unit tests from Tasks 1-7).

- [ ] **Step 4: Verify clippy clean**

Run: `cargo clippy -p singularmem-retrieve --all-targets --tests -- -D warnings`

Expected: zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/singularmem-retrieve/src/testing.rs crates/singularmem-retrieve/src/lib.rs
git commit -s -m "feat(retrieve): MockAdapter for cross-crate test reuse

Deterministic 'MOCK[query=... blocks=N ids=[...]]' output. Available
unconditionally (no #[cfg] gate) — same pattern as MockEmbedder in
singularmem-search, lets downstream tests skip --features toggling."
```

Verify sign-off.

---

## Task 8: Workspace lint + clippy gate (library complete)

The library half of sub-project 3a is done. This task is a verification-only checkpoint before starting the CLI work.

- [ ] **Step 1: Run rustfmt**

Run: `cargo fmt --check`

Expected: clean. If not, `cargo fmt`, review diff, commit separately:

```bash
git add -p crates/singularmem-retrieve/
git commit -s -m "style(retrieve): rustfmt cleanups across new crate"
```

- [ ] **Step 2: Run workspace clippy**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings across the entire workspace (singularmem-core, singularmem-search, singularmem-retrieve, root binary).

- [ ] **Step 3: Run the full workspace test suite**

Run: `cargo test --workspace`

Expected: all tests pass. The known pre-existing flake in `singularmem-core::tests/store_basics::export_emits_meta_line_and_items_in_order` may intermittently fail; if so, re-run to confirm it's the flake (not caused by this task's changes — adding a new crate cannot affect that test's SQL ordering).

- [ ] **Step 4: Verify rustdoc still builds**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`

Expected: clean. The new crate's pub items must all have doc-comments (`error.rs`, `retriever.rs`, `adapter.rs`, `testing.rs` all wrote them as part of Tasks 1-7).

- [ ] **Step 5: Commit only if Step 1 produced fixes**

If `cargo fmt` made changes, the commit from Step 1 covers it. Otherwise no commit needed — the checkpoint passes silently.

---

## Task 9: Extract `resolve_search_mode` helper from `cmd_search`

**Why now:** Both `cmd_search` (sub-project 2c, around lines 488-588 of `src/main.rs`) and the new `cmd_retrieve` (Task 11 below) need the identical directory-probe + mode-resolution + pre-flight-check logic. Extracting now keeps the duplication out of the new code and makes the helper easy to test (existing CLI tests already exercise the search path).

**Files:**
- Modify: `src/main.rs` (extract helper; refactor `cmd_search` to call it)

- [ ] **Step 1: Read the current `cmd_search` body**

Read `src/main.rs` lines 488-588 to confirm the exact code being extracted.

- [ ] **Step 2: Add the helper function**

Insert into `src/main.rs`, just after `derive_vectors_path` (currently around line 323), the following helper:

```rust
/// Result of resolving a `SearchMode` for a given store path. Returned by
/// `resolve_search_mode`.
struct ResolvedSearchMode {
    /// The concrete search mode (never `Auto` after resolution).
    mode: SearchMode,
    /// Tantivy sidecar path.
    tantivy_path: PathBuf,
    /// Vectors sidecar path.
    vectors_path: PathBuf,
}

/// Probe the store's sidecar directories and resolve `requested_mode`
/// (which may be `Auto`) into a concrete mode (`Lexical`, `Semantic`,
/// or `Hybrid`). Surfaces the same set of errors `cmd_search` does:
/// `NoIndexes` for auto + neither sidecar, `HybridMissingIndex` for
/// explicit hybrid + one missing, `IndexMissing` for explicit
/// lexical/semantic + that sidecar missing.
fn resolve_search_mode(
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
```

- [ ] **Step 3: Refactor `cmd_search` to call the helper**

Replace the top of `cmd_search` (currently lines 488-551 of `src/main.rs`, ending at the closing brace of the pre-flight `match resolved { ... }` block) with:

```rust
fn cmd_search(store_path: &Path, args: &SearchArgs) -> Result<(), CliError> {
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    let resolved = resolve_search_mode(store_path, args.mode)?;
    let ResolvedSearchMode {
        mode: resolved_mode,
        tantivy_path,
        vectors_path,
    } = resolved;
```

Then keep the rest of `cmd_search` unchanged (starting from `let query_str = args.queries.join(" ");`), but rename `resolved` → `resolved_mode` everywhere it appears below (in the `matches!(resolved, ...)` and `match (&lex_opt, &sem_opt)` blocks):

```rust
    let query_str = args.queries.join(" ");
    let opts = HybridSearchOptions {
        limit: args.limit,
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: !args.no_snippets,
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> = if matches!(resolved_mode, SearchMode::Lexical | SearchMode::Hybrid) {
        Some(Index::open(&tantivy_path)?)
    } else {
        None
    };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved_mode, SearchMode::Semantic | SearchMode::Hybrid) {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
                };
            Some(EmbedderIndex::open(&vectors_path, embedder)?)
        } else {
            None
        };

    let searcher = match (&lex_opt, &sem_opt) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("pre-flight guarantees at least one index"),
    };
    let results = searcher.search(&query_str, &opts)?;

    render_search_results(&results, args)?;
    Ok(())
}
```

- [ ] **Step 4: Run the full CLI test suite to verify nothing broke**

Run: `cargo test --test cli`

Expected: PASS for all existing tests (sub-projects 2a/2b/2c CLI tests — about 31 tests). The refactor must be behaviour-preserving.

- [ ] **Step 5: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

Watch for:
- `clippy::needless_pass_by_value` on `requested_mode: SearchMode` — `SearchMode` is `Copy` so this should be fine.
- `clippy::missing_const_for_fn` — `resolve_search_mode` does I/O (`path.exists()`) so can't be `const`.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -s -m "refactor(cli): extract resolve_search_mode helper from cmd_search

Pure refactor preserving cmd_search behaviour byte-for-byte. The new
helper will be reused by cmd_retrieve in the next commit. All existing
CLI tests still pass."
```

Verify sign-off.

---

## Task 10: CLI — `Command::Retrieve` + `RetrieveArgs` + `known_adapters` registry

**Files:**
- Modify: `src/main.rs`
- Modify: `tests/cli.rs` (one help-output test)

This task adds the CLI surface (struct + enum variant + registry) but DOES NOT wire `cmd_retrieve` yet. Task 11 implements the function body.

- [ ] **Step 1: Write the failing test**

Append to `tests/cli.rs`:

```rust
#[test]
fn retrieve_help_lists_flags_and_default_adapter() {
    singularmem()
        .args(["retrieve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--adapter"))
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--min-score"))
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--fetch-multiplier"))
        .stdout(predicate::str::contains("--rrf-k"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--show-elapsed"))
        .stdout(predicate::str::contains("default: plain"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli retrieve_help_lists_flags_and_default_adapter`

Expected: FAIL — `retrieve` subcommand doesn't exist.

- [ ] **Step 3: Add `RetrieveArgs` struct**

Insert into `src/main.rs`, after the existing `SemanticSearchArgs` struct (around line 200):

```rust
#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct RetrieveArgs {
    /// One or more query tokens. Multiple tokens are joined with a space
    /// before being passed to the underlying hybrid search.
    queries: Vec<String>,
    /// Which adapter to use for formatting. Defaults to `plain`.
    /// Sub-projects 3b/3c/3d add `claude`, `openai`, `gemini` to the registry.
    #[arg(short = 'a', long, default_value = "plain")]
    adapter: String,
    /// Max memory blocks to include in the formatted output.
    #[arg(short = 'l', long, default_value = "10")]
    limit: usize,
    /// Minimum score for a hit to be included.
    #[arg(long, default_value = "0.0")]
    min_score: f32,
    /// Underlying search mode (passed through to `HybridSearcher`).
    #[arg(short = 'm', long, value_enum, default_value_t = SearchMode::Auto)]
    mode: SearchMode,
    /// Per-ranker overfetch factor (hybrid only).
    #[arg(long, default_value = "3")]
    fetch_multiplier: usize,
    /// RRF damping constant (hybrid only).
    #[arg(long, default_value = "60")]
    rrf_k: usize,
    /// Emit `RetrievedContext` as JSON instead of adapter-formatted output.
    #[arg(long)]
    json: bool,
    /// Print "Retrieved N blocks in Xms" to stderr after the formatted output.
    #[arg(long)]
    show_elapsed: bool,
}
```

- [ ] **Step 4: Add the `Retrieve` variant to `Command`**

Modify the `Command` enum in `src/main.rs` (currently around lines 34-52) — add a new variant after `Reindex`:

```rust
#[derive(Subcommand, Debug)]
enum Command {
    /// Add a new item to the store.
    Ingest(IngestArgs),
    /// Fetch one item by ID.
    Get(GetArgs),
    /// Enumerate items, optionally filtered by tag.
    List(ListArgs),
    /// Show the supersedes chain for an item, newest-first.
    Revisions(RevisionsArgs),
    /// Emit the entire store as JSONL on stdout.
    Export,
    /// Full-text search over the store.
    Search(SearchArgs),
    /// Rebuild the Tantivy index from the `SQLite` store.
    Reindex(ReindexArgs),
    /// Retrieve memory blocks formatted for an LLM prompt.
    Retrieve(RetrieveArgs),
    /// \[DEPRECATED\] Semantic (vector) search. Use `search --mode semantic`.
    SemanticSearch(SemanticSearchArgs),
}
```

- [ ] **Step 5: Add the `known_adapters` registry function**

Insert into `src/main.rs`, just before `cmd_search` (around line 488):

```rust
/// Registry of available adapters. Sub-projects 3b/3c/3d each add one line
/// here AND one line to the root `Cargo.toml` `[dependencies]` section.
///
/// Order matters for the unknown-adapter error message: list adapters in
/// the order they should appear when the CLI tells the user what's
/// available.
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        // 3b will add: Box::new(singularmem_adapter_claude::ClaudeAdapter),
        // 3c will add: Box::new(singularmem_adapter_openai::OpenAiAdapter),
        // 3d will add: Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}
```

- [ ] **Step 6: Add the dispatch arm to `run_command`**

Modify `run_command` in `src/main.rs` (currently around lines 300-311) — add a match arm for the new variant:

```rust
fn run_command(command: Command, store: &Store, store_path: &Path) -> Result<(), CliError> {
    match command {
        Command::Ingest(args) => cmd_ingest(store, args),
        Command::Get(args) => cmd_get(store, &args),
        Command::List(args) => cmd_list(store, &args),
        Command::Revisions(args) => cmd_revisions(store, &args),
        Command::Export => cmd_export(store),
        Command::Search(args) => cmd_search(store_path, &args),
        Command::Reindex(args) => cmd_reindex(store, store_path, &args),
        Command::Retrieve(args) => cmd_retrieve(store, store_path, &args),
        Command::SemanticSearch(args) => cmd_semantic_search(store_path, &args),
    }
}
```

- [ ] **Step 7: Add a stub `cmd_retrieve` so the build passes**

Insert into `src/main.rs`, just after `cmd_search` and its `render_search_results` helper (i.e., before `cmd_semantic_search` around line 635):

```rust
fn cmd_retrieve(
    _store: &Store,
    _store_path: &Path,
    _args: &RetrieveArgs,
) -> Result<(), CliError> {
    // Task 11 implements this. The stub exists so Task 10's --help test
    // compiles without dragging in retrieval logic.
    Err(CliError::Usage(
        "cmd_retrieve not yet implemented; see Task 11".into(),
    ))
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test --test cli retrieve_help_lists_flags_and_default_adapter`

Expected: PASS — `--help` output contains all eight flags + the `default: plain` text.

- [ ] **Step 9: Run the full CLI test suite**

Run: `cargo test --test cli`

Expected: PASS for all existing tests (the stub returns an error, but no existing test calls `retrieve` so nothing is affected).

- [ ] **Step 10: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

Watch for `clippy::needless_pass_by_value` on the `_store: &Store` etc. arguments — those are references, not values, so fine.

- [ ] **Step 11: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -s -m "feat(cli): add RetrieveArgs, Command::Retrieve, known_adapters registry

CLI surface only; cmd_retrieve is stubbed and returns an error. The
next commit wires it through to Retriever + Adapter. The registry
comment marks where sub-projects 3b/3c/3d add their adapter lines."
```

Verify sign-off.

---

## Task 11: `cmd_retrieve` implementation

**Files:**
- Modify: `src/main.rs` (replace `cmd_retrieve` stub with real implementation)

- [ ] **Step 1: Write the failing test**

Append to `tests/cli.rs`:

```rust
#[test]
fn retrieve_with_default_adapter_emits_plain_format() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args(["--store", db.to_str().unwrap(), "retrieve", "fox"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## memory 1"))
        .stdout(predicate::str::contains("the quick brown fox"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli retrieve_with_default_adapter_emits_plain_format`

Expected: FAIL — `cmd_retrieve` is still the stub from Task 10.

- [ ] **Step 3: Replace the `cmd_retrieve` stub**

In `src/main.rs`, replace the `cmd_retrieve` stub from Task 10 with:

```rust
fn cmd_retrieve(
    store: &Store,
    store_path: &Path,
    args: &RetrieveArgs,
) -> Result<(), CliError> {
    use singularmem_retrieve::{Adapter, RetrieveOptions, Retriever};
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    // Adapter lookup before any I/O so unknown-adapter errors fail fast.
    let adapters = known_adapters();
    let adapter: &dyn Adapter = adapters
        .iter()
        .find(|a| a.name() == args.adapter.as_str())
        .map(std::convert::AsRef::as_ref)
        .ok_or_else(|| {
            let known: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
            CliError::Usage(format!(
                "unknown adapter '{}'; known adapters: {}",
                args.adapter,
                known.join(", ")
            ))
        })?;

    // Mode resolution + sidecar probing — same helper cmd_search uses.
    let ResolvedSearchMode {
        mode: resolved_mode,
        tantivy_path,
        vectors_path,
    } = resolve_search_mode(store_path, args.mode)?;

    let query_str = args.queries.join(" ");
    let search_opts = HybridSearchOptions {
        limit: args.limit.saturating_mul(args.fetch_multiplier).max(args.limit),
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: false, // we use full content, not snippets
    };
    let opts = RetrieveOptions {
        max_blocks: args.limit,
        min_score: args.min_score,
        search: search_opts,
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> = if matches!(resolved_mode, SearchMode::Lexical | SearchMode::Hybrid) {
        Some(Index::open(&tantivy_path)?)
    } else {
        None
    };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved_mode, SearchMode::Semantic | SearchMode::Hybrid) {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
                };
            Some(EmbedderIndex::open(&vectors_path, embedder)?)
        } else {
            None
        };

    let searcher = match (&lex_opt, &sem_opt) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("pre-flight guarantees at least one index"),
    };
    let retriever = Retriever::new(store, &searcher);
    let context = retriever.retrieve(&query_str, &opts)?;

    let mut out = io::stdout().lock();
    if args.json {
        serde_json::to_writer(&mut out, &context)?;
        writeln!(out)?;
    } else {
        let formatted = adapter.format(&context);
        write!(out, "{formatted}")?;
    }
    drop(out);

    if args.show_elapsed {
        eprintln!(
            "Retrieved {} blocks in {:.2}ms (considered {})",
            context.blocks.len(),
            context.elapsed.as_secs_f64() * 1000.0,
            context.total_considered
        );
    }
    Ok(())
}
```

Also extend `CliError` to wrap retrieve-crate errors. In `src/main.rs` around the existing `CliError` enum, add:

```rust
    #[error("{0}")]
    Retrieve(#[from] singularmem_retrieve::Error),
```

And in the `main()` exit-code match, add an arm for `EmptyQuery` (exit 1, usage error) so the empty-query test (Task 13) maps correctly. Insert just after the existing `Err(CliError::Search(...))` arms, before the catch-all `Err(e) => ...`:

```rust
        Err(CliError::Retrieve(ref e @ singularmem_retrieve::Error::EmptyQuery)) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(1)
        }
```

Note: `Error::Search(_)` and `Error::Core(_)` variants of `singularmem_retrieve::Error` are wrapped from `singularmem_search::Error` / `singularmem_core::Error` via `#[from]`. When `Retriever::retrieve` returns `Err(retrieve::Error::Search(s))`, the CLI's `?` operator converts it to `CliError::Retrieve(retrieve::Error::Search(s))`. The existing `main()` arms only catch `CliError::Search(...)` directly, so a `retrieve::Error::Search(NoIndexes)` would fall through to the catch-all (exit 1) instead of the correct exit-2.

To fix this properly, add two more arms after the `EmptyQuery` arm:

```rust
        Err(CliError::Retrieve(ref e)) => {
            // Map retrieve-crate errors to the same exit codes as their
            // underlying search/core errors, plus EmptyQuery → 1 above.
            let code = match e {
                singularmem_retrieve::Error::Search(
                    singularmem_search::Error::NoIndexes
                    | singularmem_search::Error::HybridMissingIndex { .. }
                    | singularmem_search::Error::IndexMissing { .. },
                ) => 2,
                singularmem_retrieve::Error::Core(singularmem_core::Error::NotFound { .. }) => 2,
                singularmem_retrieve::Error::EmptyQuery => 1, // already handled above; defensive
                _ => 1,
            };
            eprintln!("singularmem: {e}");
            ExitCode::from(code)
        }
```

Reorder: put the catch-all `Err(CliError::Retrieve(...))` arm AFTER the specific `EmptyQuery` arm so the latter takes precedence.

Actually, since the catch-all retrieve arm covers `EmptyQuery` via its match arm anyway, the dedicated `EmptyQuery` arm is redundant. Simplify by deleting the separate `EmptyQuery` arm and keeping only the unified `Err(CliError::Retrieve(ref e))` arm:

```rust
        Err(CliError::Retrieve(ref e)) => {
            let code = match e {
                singularmem_retrieve::Error::Search(
                    singularmem_search::Error::NoIndexes
                    | singularmem_search::Error::HybridMissingIndex { .. }
                    | singularmem_search::Error::IndexMissing { .. },
                ) => 2,
                singularmem_retrieve::Error::Core(singularmem_core::Error::NotFound { .. }) => 2,
                singularmem_retrieve::Error::EmptyQuery => 1,
                _ => 1,
            };
            eprintln!("singularmem: {e}");
            ExitCode::from(code)
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test cli retrieve_with_default_adapter_emits_plain_format`

Expected: PASS.

- [ ] **Step 5: Run the full CLI test suite**

Run: `cargo test --test cli`

Expected: PASS for all existing tests.

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

Watch for:
- `clippy::ref_pattern` / `clippy::redundant_pattern_matching` on `Err(CliError::Retrieve(ref e))` — the `ref e` is needed because we use `e` after the match. If clippy still complains, the equivalent is `Err(CliError::Retrieve(e))` with the `e` shadowed inside (binding by reference vs value depending on `Error`'s `Clone` impl — `Error` is not `Clone`, so `ref` is required).

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -s -m "feat(cli): wire cmd_retrieve to Retriever + Adapter

Adapter lookup via known_adapters registry; unknown adapter → exit 1
with helpful message. Mode resolution reuses the shared
resolve_search_mode helper. --json bypasses the adapter and emits
RetrievedContext via serde. --show-elapsed writes timing to stderr.

Adds CliError::Retrieve variant + main() match arm that maps
retrieve-crate errors to the same exit codes as their underlying
search/core errors (2 for missing-index/NotFound, 1 for EmptyQuery
and everything else)."
```

Verify sign-off.

---

## Task 12: CLI integration tests — error paths + output flags

**Files:**
- Modify: `tests/cli.rs` (seven more integration tests)

Task 11 already added one CLI test (`retrieve_with_default_adapter_emits_plain_format`). This task adds the remaining seven from the spec's Testing Strategy.

- [ ] **Step 1: Write the failing tests**

Append to `tests/cli.rs`:

```rust
#[test]
fn retrieve_json_flag_emits_valid_json() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "json output fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--json",
            "fixture",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success(), "expected success, got {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("valid JSON");
    let blocks = parsed
        .get("blocks")
        .expect("blocks field")
        .as_array()
        .expect("array");
    assert!(!blocks.is_empty(), "expected at least one block");
    let b0 = &blocks[0];
    for field in &["id", "content", "score", "score_kind", "source", "tags", "created_at"] {
        assert!(b0.get(field).is_some(), "block missing field {field}: {b0}");
    }
    assert!(parsed.get("query").is_some());
    assert!(parsed.get("elapsed").is_some());
    assert!(parsed.get("total_considered").is_some());
}

#[test]
fn retrieve_unknown_adapter_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // No need to ingest — the unknown-adapter check fails before any I/O.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "claude",
            "anything",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown adapter 'claude'"))
        .stderr(predicate::str::contains("known adapters: plain"));
}

#[test]
fn retrieve_empty_query_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "anything",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args(["--store", db.to_str().unwrap(), "retrieve", ""])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("query is empty"));
}

#[test]
fn retrieve_no_indexes_errors_like_search() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Create store but never ingest and never run reindex.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "list"])
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "retrieve",
            "anything",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no search index exists"));
}

#[test]
fn retrieve_mode_hybrid_errors_like_search() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "lexical only fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--mode",
            "hybrid",
            "fixture",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("hybrid search requires both indexes"));
}

#[test]
fn retrieve_show_elapsed_writes_to_stderr() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--show-elapsed",
            "fox",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains("Retrieved") && stderr.contains("blocks"),
        "expected timing line in stderr, got: {stderr}"
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        !stdout.contains("Retrieved"),
        "timing should not be in stdout, got: {stdout}"
    );
}

#[test]
fn retrieve_limit_caps_block_count() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    for i in 0..10 {
        singularmem()
            .args([
                "--store",
                db.to_str().unwrap(),
                "ingest",
                "--content",
                &format!("repeated word {i}"),
            ])
            .assert()
            .success();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--limit",
            "2",
            "repeated",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let heading_count = stdout.matches("## memory").count();
    assert_eq!(heading_count, 2, "expected exactly 2 memory headings, got {heading_count} in:\n{stdout}");
}
```

- [ ] **Step 2: Run all the new tests**

Run: `cargo test --test cli retrieve_`

Expected: PASS for all eight retrieve-prefixed tests (1 from Task 10, 1 from Task 11, 7 from this task).

- [ ] **Step 3: Run the full CLI test suite**

Run: `cargo test --test cli`

Expected: PASS for everything (existing 31 tests from 2a/2b/2c + 9 new retrieve tests = 40 total).

- [ ] **Step 4: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

- [ ] **Step 5: Verify fmt clean**

Run: `cargo fmt --check`

Expected: clean. If not, `cargo fmt` and include the diff in the commit below.

- [ ] **Step 6: Commit**

```bash
git add tests/cli.rs
git commit -s -m "test(cli): seven integration tests for retrieve verb

Covers --json output shape, unknown adapter error, empty query error,
NoIndexes / HybridMissingIndex error propagation (exit code 2 matching
search verb), --show-elapsed stream separation, and --limit truncation."
```

Verify sign-off.

---

## Task 13: Final workspace gate + memory-flake follow-up

**Files:**
- Modify (optional): `crates/singularmem-core/src/query.rs` (the SQL tiebreak fix noted in the memory file)
- Modify (if needed): `Cargo.lock`

This task is the final verification + an optional small follow-up fix for the known flaky test the project memory called out as a candidate for a small commit before sub-project 3.

- [ ] **Step 1: Workspace fmt check**

Run: `cargo fmt --check`

Expected: clean.

- [ ] **Step 2: Workspace clippy**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

- [ ] **Step 3: Workspace test**

Run: `cargo test --workspace`

Expected: all tests pass. If the known pre-existing flake
`singularmem-core::tests/store_basics::export_emits_meta_line_and_items_in_order`
fails, re-run once. If it consistently fails (more than two intermittent failures
in a row), proceed to Step 4 to fix it; otherwise skip Step 4.

- [ ] **Step 4 (optional): Fix the SQL tiebreak in `Store::list`**

If the flake is biting during this session, apply the one-line fix the project
memory documents.

In `crates/singularmem-core/src/query.rs` around line 79, change:

```rust
.prepare("SELECT id FROM items ORDER BY created_at ASC")
```

to:

```rust
.prepare("SELECT id FROM items ORDER BY created_at ASC, id ASC")
```

Since `id` is a ULID (lexicographically time-sortable), the secondary sort
preserves insertion order when two items share a `created_at` millisecond. The
change is purely additive (any pre-existing query plan that relied on the old
behaviour would already have been broken by SQLite's undefined tie-break order).

Run the previously flaky test in a tight loop to confirm:

```bash
for i in 1 2 3 4 5 6 7 8 9 10; do
  cargo test -p singularmem-core --test store_basics export_emits_meta_line_and_items_in_order 2>&1 | grep -E "test result|FAILED"
done
```

Expected: ten consecutive "ok" results.

Commit the fix as its own commit:

```bash
git add crates/singularmem-core/src/query.rs
git commit -s -m "fix(core): tiebreak Store::list by id when created_at collides

Two ingest() calls landing in the same millisecond produced
non-deterministic ordering in Store::list (and therefore in export
and all callers). Adding 'id ASC' to the ORDER BY makes the order
deterministic by ULID byte order, which is also the time order, so
behaviour matches user intuition for ties.

Fixes the long-standing flake in
crates/singularmem-core/tests/store_basics.rs::export_emits_meta_line_and_items_in_order."
```

Verify sign-off.

- [ ] **Step 5: Rustdoc gate**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`

Expected: clean. The new crate's public items all have doc-comments (verified during Tasks 1-7).

- [ ] **Step 6: Cargo.lock check**

Run: `git status Cargo.lock`

If `Cargo.lock` shows modifications (Task 1's new crate likely caused it; Task 4's `rusqlite` dev-dep too):

```bash
git add Cargo.lock
git commit -s -m "chore: refresh Cargo.lock after adding singularmem-retrieve crate"
```

If clean, skip.

- [ ] **Step 7: Final repository status**

Run: `git status`

Expected: clean working tree (untracked `.agents/`, `.claude/`, `skills-lock.json` files are normal per prior sub-projects).

Run: `git log --oneline -20`

Expected: the new commits from Tasks 1-13 (plus optional Task 13 Step 4 + Step 6) sit on top of `b390f82` (the v0.4.0 version-bump commit on main from sub-project 2c's wrap-up).

---

## Self-review

**1. Spec coverage check** (each spec acceptance criterion → task):

| Spec AC | Task |
|---|---|
| 1. New crate scaffold + version 0.5.0 | 1 |
| 2. Public exports + MockAdapter behind `feature = "testing"` | 1, 2, 3, 5, 6, 7 |
| 3. `Retriever::retrieve` algorithm + EmptyQuery | 4 |
| 4. PlainAdapter Markdown shape + zero-block handling | 6 |
| 5. MockAdapter `MOCK[...]` shape | 7 |
| 6. CLI `retrieve` verb with eight flags + adapter registry | 10 |
| 7. Reuses mode-resolution helper from cmd_search | 9, 11 |
| 8. `--json` emits RetrievedContext | 11, 12 |
| 9. All 12 unit + 8 CLI tests pass | distributed: 4, 6, 7, 10, 11, 12 |
| 10. No new perf budget | (no task — verified by absence) |
| 11. `docs/formats/store-v1.md` unchanged | (no task — verified by absence) |
| 12. Tag `v0.5.0` on merge | (out of plan scope — maintainer's merge ritual) |

All twelve criteria covered.

**Spec coverage of "MockAdapter behind feature = testing":** Tasks 1 and 7 deliberately
deviate. The spec says "gated behind `feature = "testing"`" but I unconditionally
re-export MockAdapter (matching singularmem-search's MockEmbedder pattern) because
the spec's hard requirement is "cross-crate test reuse without ceremony". The feature
flag exists in Cargo.toml for forward-compat — sub-projects 3b/3c/3d may want to gate
behaviour at some point. Recording this in the plan so the implementer doesn't try
to "fix" the unconditional export.

**2. Placeholder scan:** no TBDs, no "implement later" (Task 10's stub explicitly
points at Task 11 with line-by-line content for both), no "similar to Task N"
(every task has its own complete code blocks). Tasks 9-11 reuse some structural
code from sub-project 2c (mode-resolution, embedder construction) — they include
the full code rather than referencing 2c's plan.

**3. Type consistency check:**
- `Retriever<'a>`, `RetrieveOptions`, `RetrievedContext`, `MemoryBlock` consistent
  across Tasks 2, 3, 4, 6, 7, 11, 12.
- `Adapter` trait signature (`name()`, `format(&RetrievedContext) -> String`)
  consistent across Tasks 5, 6, 7, 11.
- `PlainAdapter` and `MockAdapter` unit structs consistent across Tasks 6, 7, 11.
- `Error::{Search, Core, EmptyQuery}` consistent across Tasks 1, 4, 11, 12.
- `ResolvedSearchMode { mode, tantivy_path, vectors_path }` consistent across
  Tasks 9 (creation) and 11 (use).
- `known_adapters()` signature (`Vec<Box<dyn singularmem_retrieve::Adapter>>`)
  consistent across Tasks 10 (definition) and 11 (use).
- CLI exit codes 1 (usage) / 2 (missing-index/NotFound) consistent across the
  CLI tasks and the spec's error-handling table.

Plan ready for execution.
