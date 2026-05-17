# Search v0 — Hybrid Retrieval Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Reciprocal Rank Fusion (RRF) hybrid search combining the existing Tantivy lexical and USearch vector indexes, expose it via `singularmem search --mode {auto|lexical|semantic|hybrid}`, and enforce a sixth Principle X perf budget (`hybrid_search_latency < 150 ms`).

**Architecture:** A new `HybridSearcher<'a>` in `crates/singularmem-search/src/hybrid_query.rs` borrows references to the existing `Index` and/or `EmbedderIndex` and dispatches to one of three branches: lexical-only, semantic-only, or RRF-fused hybrid. The CLI's `cmd_search` becomes a thin shell that probes for `.tantivy/`/`.vectors/` sidecars (when `--mode auto`), constructs the appropriate searcher, and renders the results.

**Tech Stack:** Rust 1.80, Tantivy 0.22.1 (lexical), USearch 2.15.3 + fastembed 4.4.0 (semantic), criterion 0.5 (bench), clap 4.5 (CLI), assert_cmd 2.0 + predicates 3.1 (CLI tests).

**Spec:** `docs/superpowers/specs/2026-05-17-search-v0-hybrid-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-search/src/hybrid_query.rs` — `HybridSearcher`, `HybridSearchOptions`, `HybridSearchResults`, `HybridHit`, `ScoreKind`, `rrf_fuse` helper, unit tests.

**Modified:**
- `crates/singularmem-core/src/item.rs` — `ItemId` gains `Ord + PartialOrd` derives.
- `crates/singularmem-search/src/error.rs` — `Error::NoIndexes` and `Error::HybridMissingIndex` variants.
- `crates/singularmem-search/src/lib.rs` — `pub mod hybrid_query;` + re-exports.
- `crates/singularmem-search/benches/search_perf.rs` — `bench_hybrid_search_latency` added; registered in `criterion_group!`.
- `src/main.rs` — `SearchArgs` grows `--mode`, `--fetch-multiplier`, `--rrf-k`, `--show-ranks`, `--json`; `cmd_search` rewritten; `cmd_semantic_search` emits one-shot deprecation note; `Command::SemanticSearch` doc-string flagged deprecated.
- `tests/cli.rs` — existing `search_missing_index_exits_2` updated; eleven new tests added.
- `.github/scripts/perf-check.sh` — sixth `check_budget` call for `hybrid_search_latency`.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"` — hybrid search is read-time only).

---

## Task 1: `ItemId` derives `Ord + PartialOrd`

**Why first:** Task 7's hybrid fusion sorts by `ItemId` to break ties deterministically. The derive must land before any consumer relies on it. The change is additive — ULID byte order = lexicographic time order, so existing `HashMap<ItemId, _>` usages keep working unchanged.

**Files:**
- Modify: `crates/singularmem-core/src/item.rs:21`
- Test: `crates/singularmem-core/src/item.rs` (extend existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add at the bottom of `mod tests` in `crates/singularmem-core/src/item.rs`:

```rust
#[test]
fn item_id_orders_by_ulid_bytes() {
    // Two ULIDs in known order; the first lexicographically precedes the second.
    let a: ItemId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid");
    let b: ItemId = "01BX5ZZKBKACTAV9WEVGEMMVRZ".parse().expect("valid");
    assert!(a < b, "lexicographically smaller ULID must sort first");

    let mut v = vec![b, a];
    v.sort();
    assert_eq!(v, vec![a, b], "sort must produce ascending order");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-core item_id_orders_by_ulid_bytes`

Expected: FAIL with `error[E0369]: binary operation '<' cannot be applied to type 'ItemId'` (and similar for `sort`).

- [ ] **Step 3: Add the derives**

Modify line 21 of `crates/singularmem-core/src/item.rs`. The line currently reads:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
```

Change it to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
```

`Ulid` (the wrapped field type) already implements `Ord` and `PartialOrd`, so the derive succeeds. Order is by raw 128-bit value, which matches the spec's "ULID byte order = lexicographic time order" claim because ULID's high-order bits are the millisecond timestamp.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p singularmem-core item_id_orders_by_ulid_bytes`

Expected: PASS.

- [ ] **Step 5: Sanity-check that nothing else broke**

Run: `cargo test -p singularmem-core`

Expected: all existing tests still pass. (The derive is additive; nothing should regress.)

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-core/src/item.rs
git commit -s -m "feat(core): derive Ord + PartialOrd on ItemId

Needed by sub-project 2c's hybrid search to break RRF ties
deterministically by ID. ULID byte order = lexicographic time order,
so the derive is purely additive — no behaviour change for existing
HashMap<ItemId, _> consumers."
```

Verify sign-off:

Run: `git log -1 --format=%B | grep -c '^Signed-off-by:'`
Expected: `1` (DCO requires exactly one trailer; do not let `-s` and a manually-included trailer combine).

---

## Task 2: Two new error variants

**Files:**
- Modify: `crates/singularmem-search/src/error.rs:12-109` (extend the `Error` enum)
- Test: `crates/singularmem-search/src/error.rs` (add a `#[cfg(test)] mod tests` at the bottom)

- [ ] **Step 1: Write the failing test**

Append to `crates/singularmem-search/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn no_indexes_error_message_mentions_reindex() {
        let e = Error::NoIndexes;
        let msg = e.to_string();
        assert!(
            msg.contains("no search index exists"),
            "message should explain the failure: got {msg:?}"
        );
        assert!(
            msg.contains("reindex"),
            "message should tell the user the fix: got {msg:?}"
        );
    }

    #[test]
    fn hybrid_missing_index_error_names_the_missing_side() {
        let e = Error::HybridMissingIndex {
            missing: "semantic",
            path: PathBuf::from("/tmp/foo.vectors"),
        };
        let msg = e.to_string();
        assert!(msg.contains("semantic"), "missing side must appear: {msg:?}");
        assert!(
            msg.contains("/tmp/foo.vectors"),
            "path must appear: {msg:?}"
        );
        assert!(
            msg.contains("reindex --with-embeddings"),
            "fix must appear: {msg:?}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-search --lib error::tests`

Expected: FAIL with `no variant or associated item named 'NoIndexes' found for enum 'Error'`.

- [ ] **Step 3: Add the two variants**

Append the following two variants inside the `pub enum Error { ... }` block in `crates/singularmem-search/src/error.rs`, immediately before the closing brace (after the existing `Usearch` variant):

```rust
    /// Neither lexical nor vector index exists for this store.
    #[error(
        "no search index exists for this store; \
         run `singularmem reindex` (and optionally `--with-embeddings`) first"
    )]
    NoIndexes,

    /// User requested `--mode hybrid` but only one of the two indexes exists.
    #[error(
        "hybrid search requires both indexes; {missing} index missing at {path}; \
         run `singularmem reindex --with-embeddings` to build both"
    )]
    HybridMissingIndex {
        /// Which side was missing — `"lexical"` or `"semantic"`.
        missing: &'static str,
        /// Path that was probed.
        path: std::path::PathBuf,
    },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p singularmem-search --lib error::tests`

Expected: PASS (both `no_indexes_error_message_mentions_reindex` and `hybrid_missing_index_error_names_the_missing_side`).

- [ ] **Step 5: Commit**

```bash
git add crates/singularmem-search/src/error.rs
git commit -s -m "feat(search): add NoIndexes and HybridMissingIndex error variants

Used by hybrid search to surface the two distinct missing-index
states: auto mode with neither sidecar present (NoIndexes) vs
explicit --mode hybrid with one sidecar missing (HybridMissingIndex)."
```

Verify sign-off as in Task 1 Step 6.

---

## Task 3: `hybrid_query.rs` types

**Files:**
- Create: `crates/singularmem-search/src/hybrid_query.rs`
- Modify: `crates/singularmem-search/src/lib.rs:9-32` (add `pub mod hybrid_query;` and re-exports)

This task introduces only the data types (`HybridSearchOptions`, `HybridSearchResults`, `HybridHit`, `ScoreKind`). The `HybridSearcher` struct, constructors, and `search` method come in Tasks 4–7.

- [ ] **Step 1: Write the failing test**

Create `crates/singularmem-search/src/hybrid_query.rs` with this content:

```rust
//! Hybrid (lexical + semantic) search via Reciprocal Rank Fusion.
//!
//! See `docs/superpowers/specs/2026-05-17-search-v0-hybrid-design.md`
//! for the design rationale.

use serde::Serialize;
use singularmem_core::ItemId;
use std::time::Duration;

/// Options controlling a hybrid search query.
#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    /// Maximum number of fused hits to return. Default: 20.
    pub limit: usize,
    /// Per-ranker overfetch factor. Each underlying ranker fetches
    /// `limit * fetch_multiplier` candidates before fusion. Default: 3.
    pub fetch_multiplier: usize,
    /// RRF damping constant `k`. Larger → flatter weighting between
    /// top-1 and top-N. Default: 60 (Cormack et al. 2009).
    pub rrf_k: usize,
    /// Include lexical snippet highlights (if available). Default: true.
    pub include_snippets: bool,
}

impl Default for HybridSearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            fetch_multiplier: 3,
            rrf_k: 60,
            include_snippets: true,
        }
    }
}

/// Discriminator naming which kind of score `HybridHit::score` carries.
///
/// Lets `--json` consumers interpret the float correctly without inspecting
/// rank fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind {
    /// Fused Reciprocal Rank Fusion score (both rankers ran).
    Rrf,
    /// Tantivy BM25 score (lexical-only mode).
    Bm25,
    /// USearch cosine similarity (semantic-only mode).
    Cosine,
}

/// One hit in a `HybridSearchResults`.
#[derive(Debug, Clone, Serialize)]
pub struct HybridHit {
    /// The matched item's ID. Call `Store::get(hit.id)` for the full payload.
    pub id: ItemId,
    /// Score whose meaning depends on `score_kind`.
    pub score: f32,
    /// Tells the consumer what `score` represents.
    pub score_kind: ScoreKind,
    /// 1-based rank in the lexical ranker, or `None` if absent.
    pub lexical_rank: Option<usize>,
    /// 1-based rank in the semantic ranker, or `None` if absent.
    pub semantic_rank: Option<usize>,
    /// Highlighted snippet from the lexical hit, when available.
    /// `None` when `include_snippets` is false OR when the hit did not appear
    /// in the lexical ranker.
    pub snippet: Option<String>,
}

/// Results of a hybrid search query.
#[derive(Debug, Clone, Serialize)]
pub struct HybridSearchResults {
    /// Hits in descending `score` order, with `ItemId` ascending as tie-break.
    pub hits: Vec<HybridHit>,
    /// Wall-clock duration of the entire `HybridSearcher::search` call.
    pub elapsed: Duration,
    /// Number of distinct documents considered for fusion (lexical ∪ semantic).
    pub total_fused: usize,
    /// Number of hits the lexical ranker returned (before fusion), or `None`
    /// if the lexical ranker did not run.
    pub lexical_hits: Option<u64>,
    /// Number of hits the semantic ranker returned (before fusion), or `None`
    /// if the semantic ranker did not run.
    pub semantic_hits: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_match_spec() {
        let o = HybridSearchOptions::default();
        assert_eq!(o.limit, 20);
        assert_eq!(o.fetch_multiplier, 3);
        assert_eq!(o.rrf_k, 60);
        assert!(o.include_snippets);
    }

    #[test]
    fn score_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ScoreKind::Rrf).unwrap(), "\"rrf\"");
        assert_eq!(serde_json::to_string(&ScoreKind::Bm25).unwrap(), "\"bm25\"");
        assert_eq!(
            serde_json::to_string(&ScoreKind::Cosine).unwrap(),
            "\"cosine\""
        );
    }
}
```

Then add the module to `crates/singularmem-search/src/lib.rs`. Replace the existing block:

```rust
pub mod embedder;
pub mod error;
pub mod index;
pub mod model;
pub mod query;
pub mod result;
pub mod semantic_query;
pub mod testing;
pub mod vector_index;
```

with:

```rust
pub mod embedder;
pub mod error;
pub mod hybrid_query;
pub mod index;
pub mod model;
pub mod query;
pub mod result;
pub mod semantic_query;
pub mod testing;
pub mod vector_index;
```

And add re-exports — replace the existing `pub use` block at the bottom of `lib.rs`:

```rust
pub use crate::embedder::{Embedder, FastembedEmbedder};
pub use crate::error::{Error, Result};
pub use crate::index::{Index, IndexOptions};
pub use crate::model::EmbeddingModel;
pub use crate::query::{Field, Query, QueryBuilder};
pub use crate::result::{Hit, SearchOptions, SearchResults};
pub use crate::semantic_query::{SemanticHit, SemanticSearchOptions, SemanticSearchResults};
pub use crate::vector_index::{
    EmbedderIndex, VectorHit, VectorIndex, VectorIndexMeta, VectorIndexOptions,
};
```

with:

```rust
pub use crate::embedder::{Embedder, FastembedEmbedder};
pub use crate::error::{Error, Result};
pub use crate::hybrid_query::{
    HybridHit, HybridSearchOptions, HybridSearchResults, ScoreKind,
};
pub use crate::index::{Index, IndexOptions};
pub use crate::model::EmbeddingModel;
pub use crate::query::{Field, Query, QueryBuilder};
pub use crate::result::{Hit, SearchOptions, SearchResults};
pub use crate::semantic_query::{SemanticHit, SemanticSearchOptions, SemanticSearchResults};
pub use crate::vector_index::{
    EmbedderIndex, VectorHit, VectorIndex, VectorIndexMeta, VectorIndexOptions,
};
```

Note: `HybridSearcher` itself is NOT yet re-exported — added in Task 4.

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests`

Expected: PASS — both `default_options_match_spec` and `score_kind_serializes_lowercase` pass (this task is type definitions; the tests verify the shape).

- [ ] **Step 3: Sanity-check that the crate still builds**

Run: `cargo build -p singularmem-search`

Expected: success, no warnings (`pedantic + nursery` clippy lints not yet checked; that's Task 8).

- [ ] **Step 4: Commit**

```bash
git add crates/singularmem-search/src/hybrid_query.rs crates/singularmem-search/src/lib.rs
git commit -s -m "feat(search): add hybrid_query module types

Adds HybridSearchOptions, HybridSearchResults, HybridHit, ScoreKind.
HybridSearcher struct + impl land in subsequent commits."
```

---

## Task 4: `HybridSearcher` struct + three constructors

**Files:**
- Modify: `crates/singularmem-search/src/hybrid_query.rs` (append the struct and constructors above the existing `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Write the failing test**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `crates/singularmem-search/src/hybrid_query.rs`, after the existing tests:

```rust
    use crate::testing::MockEmbedder;
    use crate::{EmbedderIndex, Index};
    use tempfile::TempDir;

    #[test]
    fn new_holds_both_index_references() {
        let dir = TempDir::new().unwrap();
        let lex = Index::open(dir.path().join("lex")).unwrap();
        let sem = EmbedderIndex::open(
            dir.path().join("sem"),
            Box::new(MockEmbedder::default()),
        )
        .unwrap();
        let s = HybridSearcher::new(&lex, &sem);
        assert!(s.lexical.is_some(), "lexical must be set");
        assert!(s.semantic.is_some(), "semantic must be set");
    }

    #[test]
    fn lexical_only_constructor_omits_semantic() {
        let dir = TempDir::new().unwrap();
        let lex = Index::open(dir.path().join("lex")).unwrap();
        let s = HybridSearcher::lexical_only(&lex);
        assert!(s.lexical.is_some());
        assert!(s.semantic.is_none());
    }

    #[test]
    fn semantic_only_constructor_omits_lexical() {
        let dir = TempDir::new().unwrap();
        let sem = EmbedderIndex::open(
            dir.path().join("sem"),
            Box::new(MockEmbedder::default()),
        )
        .unwrap();
        let s = HybridSearcher::semantic_only(&sem);
        assert!(s.lexical.is_none());
        assert!(s.semantic.is_some());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests`

Expected: FAIL with `cannot find function 'new' in struct 'HybridSearcher'` (or similar — `HybridSearcher` doesn't exist yet).

- [ ] **Step 3: Add the struct + constructors**

Insert the following just above the `#[cfg(test)]` block in `crates/singularmem-search/src/hybrid_query.rs`:

```rust
use crate::index::Index;
use crate::vector_index::EmbedderIndex;

/// Combines an optional lexical (`Index`) and an optional semantic
/// (`EmbedderIndex`) backend, dispatching `search` to the appropriate code path
/// based on which references are present.
///
/// Construct via [`HybridSearcher::new`], [`HybridSearcher::lexical_only`], or
/// [`HybridSearcher::semantic_only`] depending on what's available at the call
/// site. The CLI's `cmd_search` chooses based on directory probes when
/// `--mode auto`; explicit modes pick directly.
pub struct HybridSearcher<'a> {
    /// Lexical (Tantivy) index, when available.
    pub lexical: Option<&'a Index>,
    /// Semantic (USearch + embedder) index, when available.
    pub semantic: Option<&'a EmbedderIndex>,
}

impl<'a> HybridSearcher<'a> {
    /// Both rankers present; [`Self::search`] will fuse via RRF.
    #[must_use]
    pub const fn new(lexical: &'a Index, semantic: &'a EmbedderIndex) -> Self {
        Self {
            lexical: Some(lexical),
            semantic: Some(semantic),
        }
    }

    /// Lexical only; [`Self::search`] returns BM25-scored hits with
    /// `semantic_rank == None`.
    #[must_use]
    pub const fn lexical_only(lexical: &'a Index) -> Self {
        Self {
            lexical: Some(lexical),
            semantic: None,
        }
    }

    /// Semantic only; [`Self::search`] returns cosine-scored hits with
    /// `lexical_rank == None` and `snippet == None`.
    #[must_use]
    pub const fn semantic_only(semantic: &'a EmbedderIndex) -> Self {
        Self {
            lexical: None,
            semantic: Some(semantic),
        }
    }
}
```

Also add a re-export to `crates/singularmem-search/src/lib.rs`. Replace the `hybrid_query` re-export line from Task 3:

```rust
pub use crate::hybrid_query::{
    HybridHit, HybridSearchOptions, HybridSearchResults, ScoreKind,
};
```

with:

```rust
pub use crate::hybrid_query::{
    HybridHit, HybridSearcher, HybridSearchOptions, HybridSearchResults, ScoreKind,
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests`

Expected: PASS for the three new constructor tests.

- [ ] **Step 5: Commit**

```bash
git add crates/singularmem-search/src/hybrid_query.rs crates/singularmem-search/src/lib.rs
git commit -s -m "feat(search): add HybridSearcher struct with three constructors

new / lexical_only / semantic_only borrow references to existing Index
and EmbedderIndex. The search method itself lands in subsequent commits."
```

---

## Task 5: `rrf_fuse` helper

**Files:**
- Modify: `crates/singularmem-search/src/hybrid_query.rs` (append the helper function above the test module)

This task introduces the pure fusion function tested in isolation, before wiring it into `HybridSearcher::search` (Task 7).

- [ ] **Step 1: Write the failing test**

Add inside the `#[cfg(test)] mod tests` block of `crates/singularmem-search/src/hybrid_query.rs`:

```rust
    use singularmem_core::ItemId;
    use std::str::FromStr;

    fn id(s: &str) -> ItemId {
        ItemId::from_str(s).expect("valid ULID")
    }

    #[test]
    fn rrf_fuse_overlapping_results() {
        let a = id("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let b = id("01BX5ZZKBKACTAV9WEVGEMMVRZ");
        // Lexical: a@1, b@2. Semantic: b@1, a@2.
        let lex = vec![a, b];
        let sem = vec![b, a];
        let fused = rrf_fuse(&lex, &sem, 60);
        // Both docs in both rankers; both get 1/(60+1) + 1/(60+2) = 0.032520...
        assert_eq!(fused.len(), 2);
        let (got_a, got_b) = if fused[0].0 == a {
            (&fused[0], &fused[1])
        } else {
            (&fused[1], &fused[0])
        };
        let expected = 1.0 / 61.0 + 1.0 / 62.0;
        assert!(
            (got_a.1 - expected).abs() < 1e-6,
            "a score {} should be {}",
            got_a.1,
            expected
        );
        assert!((got_b.1 - expected).abs() < 1e-6);
    }

    #[test]
    fn rrf_fuse_disjoint_results() {
        let a = id("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let b = id("01BX5ZZKBKACTAV9WEVGEMMVRZ");
        // a only in lexical (rank 1); b only in semantic (rank 1).
        let lex = vec![a];
        let sem = vec![b];
        let fused = rrf_fuse(&lex, &sem, 60);
        assert_eq!(fused.len(), 2);
        for (_id, score) in &fused {
            let expected = 1.0 / 61.0;
            assert!(
                (*score - expected).abs() < 1e-6,
                "single-ranker doc gets 1/(k+1): got {score}"
            );
        }
    }

    #[test]
    fn rrf_fuse_ties_break_by_item_id_ascending() {
        let a = id("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let b = id("01BX5ZZKBKACTAV9WEVGEMMVRZ");
        // Identical ranks → identical RRF scores → sort by ItemId ascending.
        let lex = vec![b, a]; // b@1, a@2
        let sem = vec![a, b]; // a@1, b@2
        let fused = rrf_fuse(&lex, &sem, 60);
        // Both have score 1/61 + 1/62. Tie-break by id ascending → a first.
        assert_eq!(fused[0].0, a, "lower ItemId first on tie");
        assert_eq!(fused[1].0, b);
    }

    #[test]
    fn rrf_fuse_empty_inputs_returns_empty() {
        let fused = rrf_fuse::<Vec<ItemId>>(&[], &[], 60);
        assert!(fused.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests::rrf_fuse`

Expected: FAIL with `cannot find function 'rrf_fuse'`.

- [ ] **Step 3: Implement `rrf_fuse`**

Insert above the `#[cfg(test)] mod tests` block in `crates/singularmem-search/src/hybrid_query.rs`:

```rust
use std::collections::HashMap;

/// Compute Reciprocal Rank Fusion scores for the union of two ranked
/// `ItemId` lists.
///
/// For each unique `ItemId` `d` appearing at rank `r_i` in ranker `i` (1-based),
/// the RRF score is `Σ_i 1 / (k + r_i)`.
///
/// Returns `(id, score)` pairs sorted by `score` descending, with `ItemId`
/// ascending as the deterministic tie-break.
///
/// # Panics
///
/// Does not panic. `k = 0` is allowed (degenerate but well-defined).
#[must_use]
pub fn rrf_fuse(lexical: &[ItemId], semantic: &[ItemId], k: usize) -> Vec<(ItemId, f32)> {
    #[allow(clippy::cast_precision_loss)]
    let k_f = k as f32;
    let mut scores: HashMap<ItemId, f32> = HashMap::new();
    for (rank0, id) in lexical.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let r = (rank0 + 1) as f32;
        *scores.entry(*id).or_insert(0.0) += 1.0 / (k_f + r);
    }
    for (rank0, id) in semantic.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let r = (rank0 + 1) as f32;
        *scores.entry(*id).or_insert(0.0) += 1.0 / (k_f + r);
    }
    let mut fused: Vec<(ItemId, f32)> = scores.into_iter().collect();
    // Sort by score descending; tie-break by ItemId ascending.
    // `f32` is not `Ord` so we use `partial_cmp` with `Equal` fallback.
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests`

Expected: PASS (all four new `rrf_fuse_*` tests plus the earlier tests).

- [ ] **Step 5: Commit**

```bash
git add crates/singularmem-search/src/hybrid_query.rs
git commit -s -m "feat(search): add rrf_fuse pure helper

Reciprocal Rank Fusion (Cormack et al. 2009): score(d) = Σ 1/(k+r_i)
over rankers i where d appears. Sorts descending by score with
ItemId ascending as deterministic tie-break."
```

---

## Task 6: `HybridSearcher::search` — single-ranker paths

**Files:**
- Modify: `crates/singularmem-search/src/hybrid_query.rs` (append the search method's lexical-only and semantic-only branches)

This task implements `search` for the `lexical_only` and `semantic_only` constructors. Task 7 adds the fused-hybrid branch.

- [ ] **Step 1: Write the failing test**

Add inside the `#[cfg(test)] mod tests` block of `crates/singularmem-search/src/hybrid_query.rs`:

```rust
    use crate::query::Query as ParsedQuery;
    use singularmem_core::{NewItem, Store};

    #[test]
    fn lexical_only_search_returns_bm25_scored_hits() {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let lex_path = dir.path().join("lex");

        let hook = Index::open(&lex_path).unwrap();
        let store = Store::open_with_hook(&store_path, Box::new(hook)).unwrap();
        store
            .ingest(NewItem::text("the quick brown fox jumps"))
            .unwrap();
        store
            .ingest(NewItem::text("lazy dogs sleep all day"))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        let lex = Index::open(&lex_path).unwrap();
        let searcher = HybridSearcher::lexical_only(&lex);
        let opts = HybridSearchOptions::default();
        let r = searcher.search("fox", &opts).expect("search ok");

        assert!(!r.hits.is_empty(), "expected at least one hit");
        for hit in &r.hits {
            assert_eq!(hit.score_kind, ScoreKind::Bm25);
            assert!(hit.lexical_rank.is_some());
            assert!(
                hit.semantic_rank.is_none(),
                "lexical-only must not populate semantic_rank"
            );
        }
        assert_eq!(r.semantic_hits, None);
        assert!(r.lexical_hits.is_some());
    }

    #[test]
    fn semantic_only_search_returns_cosine_scored_hits() {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let sem_path = dir.path().join("sem");

        let hook = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(&store_path, Box::new(hook)).unwrap();
        store
            .ingest(NewItem::text("the quick brown fox jumps"))
            .unwrap();
        store
            .ingest(NewItem::text("lazy dogs sleep all day"))
            .unwrap();
        drop(store);

        let sem = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let searcher = HybridSearcher::semantic_only(&sem);
        let opts = HybridSearchOptions::default();
        let r = searcher.search("fox", &opts).expect("search ok");

        assert!(!r.hits.is_empty());
        for hit in &r.hits {
            assert_eq!(hit.score_kind, ScoreKind::Cosine);
            assert!(hit.semantic_rank.is_some());
            assert!(hit.lexical_rank.is_none());
            assert!(
                hit.snippet.is_none(),
                "semantic-only has no snippet source"
            );
        }
        assert_eq!(r.lexical_hits, None);
        assert!(r.semantic_hits.is_some());
    }

    #[test]
    fn search_with_both_indexes_missing_errors() {
        // Construct via the panic-safe path: lexical_only(...) but neither
        // index exists — actually impossible to construct without an Index.
        // The "no indexes" condition is enforced at the CLI layer (Task 11);
        // here we verify the library type can't be in that state.
        // This is a compile-time guarantee: HybridSearcher requires at least
        // one constructor, all of which require an &Index or &EmbedderIndex.
        // No runtime test needed; this comment documents the invariant.
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests::lexical_only_search`

Expected: FAIL with `no method named 'search' found for struct 'HybridSearcher'`.

- [ ] **Step 3: Implement the single-ranker branches**

Append this `impl` block to `crates/singularmem-search/src/hybrid_query.rs`, immediately after the existing `impl<'a> HybridSearcher<'a>` block (i.e. before `rrf_fuse`):

```rust
use crate::error::{Error, Result};
use crate::result::SearchOptions;
use crate::semantic_query::SemanticSearchOptions;

impl<'a> HybridSearcher<'a> {
    /// Run a search against whichever rankers this `HybridSearcher` holds.
    ///
    /// - Both present: RRF-fused results, `score_kind = Rrf`.
    /// - Lexical only: BM25-scored hits, `score_kind = Bm25`.
    /// - Semantic only: cosine-scored hits, `score_kind = Cosine`.
    ///
    /// # Errors
    ///
    /// Returns whatever error the underlying ranker raises
    /// ([`Error::QueryParse`], [`Error::Tantivy`], [`Error::Embedding`],
    /// [`Error::Usearch`], etc.).
    pub fn search(
        &self,
        query: &str,
        opts: &HybridSearchOptions,
    ) -> Result<HybridSearchResults> {
        let start = std::time::Instant::now();
        let fetch_n = opts.limit.saturating_mul(opts.fetch_multiplier).max(1);

        match (self.lexical, self.semantic) {
            (Some(lex), None) => self.search_lexical_only(lex, query, opts, fetch_n, start),
            (None, Some(sem)) => self.search_semantic_only(sem, query, opts, fetch_n, start),
            (Some(_lex), Some(_sem)) => {
                // Task 7 replaces this stub with RRF fusion.
                Err(Error::NoIndexes)
            }
            (None, None) => Err(Error::NoIndexes),
        }
    }

    fn search_lexical_only(
        &self,
        lex: &Index,
        query: &str,
        opts: &HybridSearchOptions,
        fetch_n: usize,
        start: std::time::Instant,
    ) -> Result<HybridSearchResults> {
        let parsed = crate::Query::parse(query)?;
        let lex_opts = SearchOptions {
            limit: opts.limit,
            offset: 0,
            include_snippets: opts.include_snippets,
        };
        let _ = fetch_n; // unused in lexical-only (we ask for `limit` directly)
        let res = lex.search(&parsed, lex_opts)?;
        let lexical_hits = Some(res.total_matched);
        let hits: Vec<HybridHit> = res
            .hits
            .into_iter()
            .enumerate()
            .map(|(rank0, h)| HybridHit {
                id: h.id,
                score: h.score,
                score_kind: ScoreKind::Bm25,
                lexical_rank: Some(rank0 + 1),
                semantic_rank: None,
                snippet: h.snippet,
            })
            .collect();
        let total_fused = hits.len();
        Ok(HybridSearchResults {
            hits,
            elapsed: start.elapsed(),
            total_fused,
            lexical_hits,
            semantic_hits: None,
        })
    }

    fn search_semantic_only(
        &self,
        sem: &EmbedderIndex,
        query: &str,
        opts: &HybridSearchOptions,
        fetch_n: usize,
        start: std::time::Instant,
    ) -> Result<HybridSearchResults> {
        let sem_opts = SemanticSearchOptions {
            limit: opts.limit,
            min_score: 0.0,
        };
        let _ = fetch_n; // unused in semantic-only
        let res = sem.semantic_search(query, &sem_opts)?;
        let semantic_hits = Some(res.total_indexed);
        let hits: Vec<HybridHit> = res
            .hits
            .into_iter()
            .enumerate()
            .map(|(rank0, h)| HybridHit {
                id: h.id,
                score: h.score,
                score_kind: ScoreKind::Cosine,
                lexical_rank: None,
                semantic_rank: Some(rank0 + 1),
                snippet: None,
            })
            .collect();
        let total_fused = hits.len();
        Ok(HybridSearchResults {
            hits,
            elapsed: start.elapsed(),
            total_fused,
            lexical_hits: None,
            semantic_hits,
        })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests`

Expected: PASS for `lexical_only_search_returns_bm25_scored_hits` and `semantic_only_search_returns_cosine_scored_hits`. (The earlier `rrf_fuse_*` and type-shape tests should still pass.)

- [ ] **Step 5: Commit**

```bash
git add crates/singularmem-search/src/hybrid_query.rs
git commit -s -m "feat(search): HybridSearcher::search single-ranker paths

Lexical-only returns BM25-scored hits with snippets when requested;
semantic-only returns cosine-scored hits with no snippets. The fused
hybrid branch is stubbed and lands in the next commit."
```

---

## Task 7: `HybridSearcher::search` — fused-hybrid path

**Files:**
- Modify: `crates/singularmem-search/src/hybrid_query.rs` (replace the stub in the `(Some, Some)` arm with the full fusion implementation)

- [ ] **Step 1: Write the failing test**

Add inside the `#[cfg(test)] mod tests` block of `crates/singularmem-search/src/hybrid_query.rs`:

```rust
    #[test]
    fn hybrid_search_fuses_lexical_and_semantic() {
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
        store
            .ingest(NewItem::text("the quick brown fox jumps over"))
            .unwrap();
        store
            .ingest(NewItem::text("lazy dogs sleep all day"))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        let lex = Index::open(&lex_path).unwrap();
        let sem =
            EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let searcher = HybridSearcher::new(&lex, &sem);
        let opts = HybridSearchOptions::default();
        let r = searcher.search("fox", &opts).expect("search ok");

        assert!(!r.hits.is_empty());
        // First hit should have score_kind Rrf and at least one rank populated.
        let h0 = &r.hits[0];
        assert_eq!(h0.score_kind, ScoreKind::Rrf);
        assert!(h0.lexical_rank.is_some() || h0.semantic_rank.is_some());
        assert!(r.lexical_hits.is_some());
        assert!(r.semantic_hits.is_some());
    }

    #[test]
    fn hybrid_snippet_provenance_from_lexical() {
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
        store
            .ingest(NewItem::text("a memorable phrase about foxes"))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        let lex = Index::open(&lex_path).unwrap();
        let sem =
            EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let searcher = HybridSearcher::new(&lex, &sem);
        let r = searcher
            .search("foxes", &HybridSearchOptions::default())
            .unwrap();
        // For a doc that appears in the lexical ranker, snippet must be Some.
        let hit_with_lex_rank = r
            .hits
            .iter()
            .find(|h| h.lexical_rank.is_some())
            .expect("expected at least one lexically-matched hit");
        assert!(
            hit_with_lex_rank.snippet.is_some(),
            "doc with lexical_rank must carry a snippet when include_snippets=true"
        );
    }

    #[test]
    fn hybrid_search_respects_limit() {
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
        for i in 0..30 {
            store
                .ingest(NewItem::text(format!("repeated word number {i}")))
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        let lex = Index::open(&lex_path).unwrap();
        let sem =
            EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let searcher = HybridSearcher::new(&lex, &sem);
        let opts = HybridSearchOptions {
            limit: 5,
            ..HybridSearchOptions::default()
        };
        let r = searcher.search("repeated", &opts).unwrap();
        assert!(r.hits.len() <= 5, "got {} hits, expected ≤ 5", r.hits.len());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests::hybrid_search_fuses`

Expected: FAIL — the `(Some, Some)` arm currently returns `Err(Error::NoIndexes)`.

- [ ] **Step 3: Replace the stub with fusion logic**

In `crates/singularmem-search/src/hybrid_query.rs`, replace the entire `(Some(_lex), Some(_sem))` arm in the `search` method:

```rust
            (Some(_lex), Some(_sem)) => {
                // Task 7 replaces this stub with RRF fusion.
                Err(Error::NoIndexes)
            }
```

with a call to a new private method:

```rust
            (Some(lex), Some(sem)) => self.search_hybrid(lex, sem, query, opts, fetch_n, start),
```

Then add the `search_hybrid` method inside the same `impl<'a> HybridSearcher<'a>` block, after `search_semantic_only`:

```rust
    fn search_hybrid(
        &self,
        lex: &Index,
        sem: &EmbedderIndex,
        query: &str,
        opts: &HybridSearchOptions,
        fetch_n: usize,
        start: std::time::Instant,
    ) -> Result<HybridSearchResults> {
        // Lexical sub-search: overfetch by fetch_multiplier; snippets only if
        // requested (we still need the lex `Hit`s for snippet provenance below).
        let parsed = crate::Query::parse(query)?;
        let lex_opts = SearchOptions {
            limit: fetch_n,
            offset: 0,
            include_snippets: opts.include_snippets,
        };
        let lex_res = lex.search(&parsed, lex_opts)?;
        let lexical_hits = Some(lex_res.total_matched);

        // Semantic sub-search: overfetch likewise.
        let sem_opts = SemanticSearchOptions {
            limit: fetch_n,
            min_score: 0.0,
        };
        let sem_res = sem.semantic_search(query, &sem_opts)?;
        let semantic_hits = Some(sem_res.total_indexed);

        // Build ItemId-keyed lookups so we can re-attach ranks + snippets after
        // fusion.
        let lex_ids: Vec<ItemId> = lex_res.hits.iter().map(|h| h.id).collect();
        let sem_ids: Vec<ItemId> = sem_res.hits.iter().map(|h| h.id).collect();
        let lex_rank: HashMap<ItemId, usize> = lex_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i + 1))
            .collect();
        let sem_rank: HashMap<ItemId, usize> = sem_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i + 1))
            .collect();
        let snippets: HashMap<ItemId, Option<String>> =
            lex_res.hits.into_iter().map(|h| (h.id, h.snippet)).collect();

        // Fuse and truncate to `limit`.
        let fused = rrf_fuse(&lex_ids, &sem_ids, opts.rrf_k);
        let total_fused = fused.len();
        let hits: Vec<HybridHit> = fused
            .into_iter()
            .take(opts.limit)
            .map(|(id, rrf_score)| HybridHit {
                id,
                score: rrf_score,
                score_kind: ScoreKind::Rrf,
                lexical_rank: lex_rank.get(&id).copied(),
                semantic_rank: sem_rank.get(&id).copied(),
                snippet: snippets.get(&id).cloned().flatten(),
            })
            .collect();

        Ok(HybridSearchResults {
            hits,
            elapsed: start.elapsed(),
            total_fused,
            lexical_hits,
            semantic_hits,
        })
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-search --lib hybrid_query::tests`

Expected: PASS for all hybrid tests (`hybrid_search_fuses_lexical_and_semantic`, `hybrid_snippet_provenance_from_lexical`, `hybrid_search_respects_limit`) and all earlier tests.

- [ ] **Step 5: Run the full search-crate test suite to catch regressions**

Run: `cargo test -p singularmem-search`

Expected: all tests pass (existing + new).

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-search/src/hybrid_query.rs
git commit -s -m "feat(search): RRF-fused hybrid path in HybridSearcher::search

Both rankers fetch limit * fetch_multiplier candidates; rrf_fuse combines
them; snippets are inherited from the lexical hit when available.
Truncates to opts.limit after fusion."
```

---

## Task 8: Lint + clippy gate

**Files:** no source changes; this task is a quality gate.

- [ ] **Step 1: Run rustfmt**

Run: `cargo fmt --check`

Expected: clean (no diff). If it fails, run `cargo fmt`, review the diff, and re-run `--check`.

- [ ] **Step 2: Run clippy with workspace lints**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

If `pedantic`/`nursery` flag anything in `hybrid_query.rs`, apply targeted fixes in `hybrid_query.rs`. Common ones to expect:
- `clippy::cast_precision_loss` on `usize as f32` — already wrapped with `#[allow]` in Task 5; check if any new sites need it.
- `clippy::missing_const_for_fn` on `rrf_fuse` — leave non-const (uses `HashMap`).
- `clippy::missing_panics_doc` / `clippy::missing_errors_doc` — every public fn already has doc; verify.

- [ ] **Step 3: Run all tests once more**

Run: `cargo test --workspace`

Expected: PASS.

- [ ] **Step 4: Commit only if Step 1 or Step 2 produced fixes**

If `cargo fmt` made changes or you adjusted a clippy lint:

```bash
git add -p crates/singularmem-search/src/hybrid_query.rs
git commit -s -m "style(search): rustfmt / clippy fixes for hybrid_query

No behaviour change."
```

If nothing changed, skip the commit and move to Task 9.

---

## Task 9: CLI — add new flags to `SearchArgs`

**Files:**
- Modify: `src/main.rs:130-146` (extend `SearchArgs` and add a `SearchMode` enum)

This task only extends the CLI argument surface. Behaviour wiring happens in Task 10.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `tests/cli.rs`:

```rust
#[test]
fn search_help_lists_mode_flag() {
    singularmem()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("auto"))
        .stdout(predicate::str::contains("lexical"))
        .stdout(predicate::str::contains("semantic"))
        .stdout(predicate::str::contains("hybrid"));
}

#[test]
fn search_help_lists_show_ranks_and_json_flags() {
    singularmem()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--show-ranks"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--fetch-multiplier"))
        .stdout(predicate::str::contains("--rrf-k"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test cli search_help_lists_mode_flag search_help_lists_show_ranks_and_json_flags`

Expected: FAIL — `--mode` etc. not in help output.

- [ ] **Step 3: Add `SearchMode` enum**

Add to `src/main.rs`, immediately after the `enum ListFormat { ... }` block (around line 122):

```rust
/// Which search backend(s) to use for `search`.
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum SearchMode {
    /// Use hybrid when both `.tantivy/` and `.vectors/` exist; degrade to
    /// whichever single index is present; error when neither exists.
    Auto,
    /// Tantivy BM25 only.
    Lexical,
    /// USearch cosine only.
    Semantic,
    /// RRF-fused lexical + semantic; error if either is missing.
    Hybrid,
}
```

- [ ] **Step 4: Extend `SearchArgs`**

Replace the entire `SearchArgs` struct in `src/main.rs:130-146`:

```rust
#[derive(Args, Debug)]
struct SearchArgs {
    /// One or more query tokens. Multiple tokens become an implicit AND.
    queries: Vec<String>,
    /// Max hits to return.
    #[arg(long, default_value = "20")]
    limit: usize,
    /// Skip first N hits (pagination).
    #[arg(long, default_value = "0")]
    offset: usize,
    /// Suppress snippet highlighting (faster).
    #[arg(long)]
    no_snippets: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
}
```

with:

```rust
#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct SearchArgs {
    /// One or more query tokens. Multiple tokens become an implicit AND.
    queries: Vec<String>,
    /// Which backend(s) to use. `auto` picks hybrid when both sidecars exist,
    /// falls back to whichever one is present, and errors when neither is.
    #[arg(short = 'm', long, value_enum, default_value_t = SearchMode::Auto)]
    mode: SearchMode,
    /// Max hits to return.
    #[arg(short = 'l', long, default_value = "20")]
    limit: usize,
    /// Skip first N hits (pagination, lexical mode only).
    #[arg(long, default_value = "0")]
    offset: usize,
    /// Per-ranker overfetch factor; hybrid only. Default 3.
    #[arg(long, default_value = "3")]
    fetch_multiplier: usize,
    /// RRF damping constant; hybrid only. Default 60.
    #[arg(long, default_value = "60")]
    rrf_k: usize,
    /// Suppress snippet highlighting (faster).
    #[arg(long)]
    no_snippets: bool,
    /// Include per-ranker rank columns in human output.
    #[arg(long)]
    show_ranks: bool,
    /// Emit JSON results instead of human-readable output.
    #[arg(long)]
    json: bool,
    /// Output format. (Legacy; `--json` and `--show-ranks` are preferred.)
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
}
```

- [ ] **Step 5: Make the build pass**

`cmd_search` currently doesn't use `args.mode`, `args.fetch_multiplier`, `args.rrf_k`, `args.show_ranks`, or `args.json`. Rust will warn-then-fail on `unused`. Suppress for now by prefixing each unused field with `let _ = &args.mode;` (and so on) at the top of `cmd_search`, OR — preferred — wire them through in this same task by replacing the entire `cmd_search` body with a placeholder that ignores them. Since the next task rewrites `cmd_search`, the simplest path is to add `let _ = (&args.mode, args.fetch_multiplier, args.rrf_k, args.show_ranks, args.json);` at the very top of `cmd_search` (around line 454):

```rust
fn cmd_search(store_path: &Path, args: &SearchArgs) -> Result<(), CliError> {
    // Suppress unused-arg warnings; Task 10 wires these through.
    let _ = (&args.mode, args.fetch_multiplier, args.rrf_k, args.show_ranks, args.json);
    use singularmem_search::{Index, Query, SearchOptions};
    // ... rest unchanged
```

- [ ] **Step 6: Run tests to verify the new tests pass**

Run: `cargo test --test cli search_help_lists_mode_flag search_help_lists_show_ranks_and_json_flags`

Expected: PASS.

- [ ] **Step 7: Run existing CLI tests to verify nothing broke**

Run: `cargo test --test cli`

Expected: PASS. (Existing `search_finds_ingested_item` etc. still work because behaviour is unchanged.)

- [ ] **Step 8: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -s -m "feat(cli): add --mode, --fetch-multiplier, --rrf-k, --show-ranks, --json to search

New flags surface only; cmd_search behaviour unchanged in this commit.
The next commit wires them through to HybridSearcher."
```

---

## Task 10: CLI — rewrite `cmd_search` with mode dispatch + degradation

**Files:**
- Modify: `src/main.rs:453-493` (rewrite `cmd_search`)
- Modify: `src/main.rs:214-230` (extend `CliError` if needed — see Step 3)
- Modify: `tests/cli.rs:252-276` (update `search_missing_index_exits_2` to match new behaviour)
- Modify: `tests/cli.rs` (add seven new tests covering auto, explicit, and error modes)

- [ ] **Step 1: Write the failing tests**

First, update the existing `search_missing_index_exits_2` test in `tests/cli.rs:252-276`. Replace the entire `#[test] fn search_missing_index_exits_2()` block with:

```rust
#[test]
fn search_errors_when_both_indexes_missing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Create store but never ingest and never run reindex.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "list"])
        .assert()
        .success();

    // With neither .tantivy/ nor .vectors/ on disk, auto mode must error.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "search",
            "anything",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no search index exists"));
}
```

Then add these new tests at the bottom of `tests/cli.rs`:

```rust
#[test]
fn search_default_mode_uses_hybrid_when_vectors_exist() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps over the lazy dog",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Build the vector sidecar so auto mode picks hybrid.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "fox"])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("rrf="));
}

#[test]
fn search_default_mode_falls_back_to_lexical_when_no_vectors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "a memorable phrase about brown foxes",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    // No reindex --with-embeddings, so .vectors/ does not exist.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "foxes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bm25="));
}

#[test]
fn search_mode_lexical_explicit_works() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "lexical mode test fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "lexical",
            "lexical",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("bm25="));
}

#[test]
fn search_mode_semantic_explicit_works() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "semantic mode test fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "semantic",
            "fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("cos="));
}

#[test]
fn search_mode_hybrid_errors_when_vectors_missing() {
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
            "search",
            "--mode",
            "hybrid",
            "fixture",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("hybrid search requires both indexes"))
        .stderr(predicate::str::contains("semantic index missing"));
}

#[test]
fn search_mode_hybrid_errors_when_lexical_missing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Ingest with --no-index so .tantivy/ is never created. Then run
    // reindex --with-embeddings only (which currently always builds the
    // tantivy sidecar too). To get a vectors-only state we delete .tantivy/
    // after the reindex.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "ingest",
            "--content",
            "semantic only fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();
    // Delete the tantivy sidecar that reindex built.
    let tantivy_dir = {
        let mut s = db.clone().into_os_string();
        s.push(".tantivy");
        std::path::PathBuf::from(s)
    };
    std::fs::remove_dir_all(&tantivy_dir).expect("remove tantivy dir");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "hybrid",
            "fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("hybrid search requires both indexes"))
        .stderr(predicate::str::contains("lexical index missing"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test cli search_default_mode search_mode_ search_errors_when_both`

Expected: most FAIL — `cmd_search` ignores `args.mode`, doesn't probe directories, doesn't print `rrf=`/`bm25=`/`cos=` prefixes.

- [ ] **Step 3: Extend `CliError`**

In `src/main.rs:214-230`, extend the `CliError` enum to wrap `singularmem_search::Error` directly (we currently stringify it via `CliError::IndexOpen`, which loses exit-code discrimination). Add this variant inside `enum CliError`:

```rust
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),
```

Also extend the `main()` exit-code match (around line 195) to map the two new search errors:

```rust
        Err(CliError::Search(ref e @ singularmem_search::Error::NoIndexes)) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(2)
        }
        Err(CliError::Search(ref e @ singularmem_search::Error::HybridMissingIndex { .. })) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(2)
        }
```

Put both arms just after the existing `Err(CliError::IndexOpen(...))` arm and before the catch-all `Err(e) => ...`.

- [ ] **Step 4: Rewrite `cmd_search`**

Replace the entire `cmd_search` function in `src/main.rs:453-493` with:

```rust
fn cmd_search(store_path: &Path, args: &SearchArgs) -> Result<(), CliError> {
    use singularmem_search::{
        EmbedderIndex, HybridSearchOptions, HybridSearcher, Index, ScoreKind,
    };

    let tantivy_path = derive_index_path(store_path);
    let vectors_path = derive_vectors_path(store_path);
    let has_lexical = tantivy_path.exists();
    let has_vectors = vectors_path.exists();

    // Resolve --mode auto → concrete mode (or NoIndexes error).
    let resolved = match args.mode {
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

    // Explicit-mode pre-flight checks (--mode auto bypasses these because it
    // already degraded above).
    match resolved {
        SearchMode::Hybrid => {
            if !has_lexical {
                return Err(CliError::Search(
                    singularmem_search::Error::HybridMissingIndex {
                        missing: "lexical",
                        path: tantivy_path.clone(),
                    },
                ));
            }
            if !has_vectors {
                return Err(CliError::Search(
                    singularmem_search::Error::HybridMissingIndex {
                        missing: "semantic",
                        path: vectors_path.clone(),
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

    let query_str = args.queries.join(" ");
    let opts = HybridSearchOptions {
        limit: args.limit,
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: !args.no_snippets,
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> = if matches!(resolved, SearchMode::Lexical | SearchMode::Hybrid) {
        Some(Index::open(&tantivy_path)?)
    } else {
        None
    };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved, SearchMode::Semantic | SearchMode::Hybrid) {
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

fn render_search_results(
    results: &singularmem_search::HybridSearchResults,
    args: &SearchArgs,
) -> Result<(), CliError> {
    use singularmem_search::ScoreKind;

    if results.hits.is_empty() {
        tracing::info!("0 matches");
        return Ok(());
    }

    let mut out = io::stdout().lock();
    if args.json {
        serde_json::to_writer(&mut out, results)?;
        writeln!(out)?;
        return Ok(());
    }

    for hit in &results.hits {
        let tag = match hit.score_kind {
            ScoreKind::Rrf => "rrf",
            ScoreKind::Bm25 => "bm25",
            ScoreKind::Cosine => "cos",
        };
        let snip = hit.snippet.as_deref().unwrap_or("").replace('\n', " ");
        if args.show_ranks {
            let lex = hit
                .lexical_rank
                .map_or_else(|| "—".to_string(), |r| r.to_string());
            let sem = hit
                .semantic_rank
                .map_or_else(|| "—".to_string(), |r| r.to_string());
            writeln!(
                out,
                "{}  {}={:.4}  lex={}  sem={}  {}",
                hit.id, tag, hit.score, lex, sem, snip
            )?;
        } else {
            writeln!(out, "{}  {}={:.4}  {}", hit.id, tag, hit.score, snip)?;
        }
    }
    Ok(())
}
```

Also drop the suppression line from Task 9 Step 5 (the unused args are now consumed).

- [ ] **Step 5: Run all the search CLI tests to verify they pass**

Run: `cargo test --test cli search`

Expected: PASS for `search_finds_ingested_item`, `search_errors_when_both_indexes_missing`, `search_default_mode_uses_hybrid_when_vectors_exist`, `search_default_mode_falls_back_to_lexical_when_no_vectors`, `search_mode_lexical_explicit_works`, `search_mode_semantic_explicit_works`, `search_mode_hybrid_errors_when_vectors_missing`, `search_mode_hybrid_errors_when_lexical_missing`. Existing tests must still pass.

- [ ] **Step 6: Run full CLI test suite**

Run: `cargo test --test cli`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -s -m "feat(cli): wire HybridSearcher into cmd_search with mode dispatch

--mode auto probes for sidecar directories and degrades gracefully
(lexical-only, semantic-only) or errors when neither exists.
--mode hybrid/lexical/semantic enforce strict pre-flight checks.
Output format prefixes the score with its kind (rrf=/bm25=/cos=)."
```

---

## Task 11: CLI — `--show-ranks` and `--json` output verification

This task is a verification-only TDD pass on the renderer paths already implemented in Task 10. No source changes if Task 10's implementation is correct.

**Files:**
- Modify: `tests/cli.rs` (add two tests)

- [ ] **Step 1: Write the failing tests**

Add at the bottom of `tests/cli.rs`:

```rust
#[test]
fn search_show_ranks_flag_includes_columns() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "show ranks fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--show-ranks",
            "fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("lex="))
        .stdout(predicate::str::contains("sem="));
}

#[test]
fn search_json_flag_emits_valid_json() {
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
            "search",
            "--json",
            "fixture",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success(), "expected success, got {:?}", out);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("valid JSON");
    let hits = parsed.get("hits").expect("hits field").as_array().expect("array");
    assert!(!hits.is_empty(), "expected at least one hit");
    let h0 = &hits[0];
    assert!(h0.get("id").is_some(), "hit must have id");
    assert!(h0.get("score").is_some(), "hit must have score");
    assert!(h0.get("score_kind").is_some(), "hit must have score_kind");
    // lexical_rank/semantic_rank may be null but the keys must exist.
    assert!(h0.get("lexical_rank").is_some());
    assert!(h0.get("semantic_rank").is_some());
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --test cli search_show_ranks_flag_includes_columns search_json_flag_emits_valid_json`

Expected: PASS. (The renderer code in Task 10 already implements both flags. If a test fails, fix the renderer in `src/main.rs::render_search_results` to match — the spec is authoritative on the field names and the `score_kind` enum serialisation.)

One thing to note for `serde_json::to_writer(&mut out, results)`: this requires `HybridSearchResults` (which contains `Duration`) to serialise correctly. The default `Serialize` for `std::time::Duration` produces `{"secs":<u64>,"nanos":<u32>}`. That's acceptable but not lovely. If the JSON test passes as-is, leave it. If a future test cares about the shape of `elapsed`, that's a separate issue.

- [ ] **Step 3: Commit (only if changes were needed)**

If you had to edit `src/main.rs::render_search_results` to make the tests pass:

```bash
git add src/main.rs tests/cli.rs
git commit -s -m "test(cli): add coverage for --show-ranks and --json flags"
```

If no implementation changes were required (Task 10 was correct), commit only the tests:

```bash
git add tests/cli.rs
git commit -s -m "test(cli): add coverage for --show-ranks and --json flags"
```

---

## Task 12: `semantic-search` deprecated alias

**Files:**
- Modify: `src/main.rs:495-553` (rewrite `cmd_semantic_search` to forward through `cmd_search`)
- Modify: `src/main.rs:34-52` (update `Command::SemanticSearch` doc string)
- Modify: `tests/cli.rs` (add one CLI integration test + one in-process test)

- [ ] **Step 1: Write the failing test**

Add at the bottom of `tests/cli.rs`:

```rust
#[test]
fn semantic_search_deprecated_alias_still_works() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "deprecation alias fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "semantic-search",
            "fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("cos="))
        .stderr(predicate::str::contains("deprecated"));
}
```

(The "appears once per process" property is documented in the spec but is awkward to test from `assert_cmd` because each invocation spawns a fresh process. The `OnceLock` ensures correctness within a single process; covered by code review rather than a test.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli semantic_search_deprecated_alias_still_works`

Expected: FAIL — current `cmd_semantic_search` doesn't print `cos=` (it prints raw `{score}\t{id}` table format) and doesn't emit a deprecation note.

- [ ] **Step 3: Rewrite `cmd_semantic_search`**

Replace the entire `cmd_semantic_search` function in `src/main.rs:495-553` with:

```rust
fn cmd_semantic_search(store_path: &Path, args: &SemanticSearchArgs) -> Result<(), CliError> {
    use std::sync::OnceLock;
    static DEPRECATION_NOTICE: OnceLock<()> = OnceLock::new();
    DEPRECATION_NOTICE.get_or_init(|| {
        eprintln!(
            "note: 'semantic-search' is deprecated; use 'search --mode semantic'"
        );
    });

    // Forward through cmd_search with mode=Semantic.
    let forwarded = SearchArgs {
        queries: args.queries.clone(),
        mode: SearchMode::Semantic,
        limit: args.limit,
        offset: 0,
        fetch_multiplier: 3,
        rrf_k: 60,
        no_snippets: true, // semantic mode has no snippets anyway
        show_ranks: false,
        json: matches!(args.format, ListFormat::Jsonl),
        format: args.format,
    };
    cmd_search(store_path, &forwarded)
}
```

Update the `Command::SemanticSearch` doc-string in `src/main.rs:51` from:

```rust
    /// Semantic (vector) search over the store.
    SemanticSearch(SemanticSearchArgs),
```

to:

```rust
    /// [DEPRECATED] Semantic (vector) search. Use `search --mode semantic`.
    SemanticSearch(SemanticSearchArgs),
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test cli semantic_search_deprecated_alias_still_works`

Expected: PASS.

Run: `cargo test --test cli` to make sure no existing tests broke (the existing `semantic_search_with_mock_embedder_finds_ingested_item` test might fail because it expects the old `{score}\t{id}` table output and the new `cos={score}` line format). If it does, update its expectations to look for `cos=` and the ID separately, OR delete it as redundant with `search_mode_semantic_explicit_works` (the new test from Task 10).

Recommended: update existing test to use the new prefix. In `tests/cli.rs:368` (`semantic_search_with_mock_embedder_finds_ingested_item`), replace the assertion that checks for the ID-formatted output line with one that checks for `cos=` in stdout.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs tests/cli.rs
git commit -s -m "feat(cli): make semantic-search a deprecated alias for search --mode semantic

Emits a one-shot stderr note (via OnceLock) and forwards through the
hybrid search code path. Preserves backwards compatibility with
v0.3.0's top-level semantic-search verb per Principle III.a
(one-way ratchet — public surface only grows)."
```

---

## Task 13: Bench — `hybrid_search_latency`

**Files:**
- Modify: `crates/singularmem-search/benches/search_perf.rs` (add `bench_hybrid_search_latency`; register in `criterion_group!`)

- [ ] **Step 1: Add the bench function**

Append to `crates/singularmem-search/benches/search_perf.rs`, just above the `criterion_group!` macro:

```rust
fn bench_hybrid_search_latency(c: &mut Criterion) {
    use singularmem_search::HybridSearcher;
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
    for i in 0..1_000 {
        store
            .ingest(NewItem::text(format!("benchmark hybrid item number {i}")))
            .unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(store);

    let lex = Index::open(&lex_path).unwrap();
    let sem = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
    let searcher = HybridSearcher::new(&lex, &sem);
    let opts = singularmem_search::HybridSearchOptions::default();

    c.bench_function("hybrid_search_latency", |b| {
        b.iter(|| {
            let _ = searcher.search("benchmark hybrid item", &opts).unwrap();
        });
    });
}
```

- [ ] **Step 2: Register the bench in `criterion_group!`**

In `crates/singularmem-search/benches/search_perf.rs`, replace:

```rust
criterion_group!(
    benches,
    bench_search_latency,
    bench_reindex_throughput,
    bench_embed_throughput,
    bench_semantic_search_latency,
);
```

with:

```rust
criterion_group!(
    benches,
    bench_search_latency,
    bench_reindex_throughput,
    bench_embed_throughput,
    bench_semantic_search_latency,
    bench_hybrid_search_latency,
);
```

- [ ] **Step 3: Run the bench locally and capture median**

Run: `cargo bench -p singularmem-search --bench search_perf -- hybrid_search_latency`

Expected: bench completes; criterion writes `target/criterion/hybrid_search_latency/new/estimates.json`.

Read the median:

Run: `python3 -c "import json; d=json.load(open('target/criterion/hybrid_search_latency/new/estimates.json')); print(d['median']['point_estimate'])"`

Expected: a value far below `150_000_000` (150 ms in nanoseconds). On a dev machine with MockEmbedder, expect single-digit milliseconds or less; the budget gives ample headroom for slower CI runners.

- [ ] **Step 4: Commit**

```bash
git add crates/singularmem-search/benches/search_perf.rs
git commit -s -m "perf(search): add hybrid_search_latency criterion bench

Seeds 1000 items into both lexical and semantic sidecars via MultiHook,
then measures HybridSearcher::search wall-clock time. Drives the sixth
Principle X perf budget (added in the next commit)."
```

---

## Task 14: Perf budget — sixth `check_budget` in CI script

**Files:**
- Modify: `.github/scripts/perf-check.sh:1-87` (add the sixth budget check; update header comment)

- [ ] **Step 1: Update the header comment**

In `.github/scripts/perf-check.sh:1-6`, replace:

```bash
#!/usr/bin/env bash
# Enforce the five perf budgets from Constitution Principle X.
# Reads criterion's per-bench estimates.json (stable JSON schema) rather
# than parsing CLI bencher output.
# Exit codes: 0 success, 11=size, 12=cold start, 13=ingest, 14=query, 15=semantic.
```

with:

```bash
#!/usr/bin/env bash
# Enforce the six perf budgets from Constitution Principle X.
# Reads criterion's per-bench estimates.json (stable JSON schema) rather
# than parsing CLI bencher output.
# Exit codes: 0 success, 11=size, 12=cold start, 13=ingest, 14=query,
# 15=semantic, 16=hybrid.
```

- [ ] **Step 2: Add the sixth budget block**

In `.github/scripts/perf-check.sh`, after the existing semantic-search block (lines 70-79, the block ending with `exit 15`) and before the `echo "All perf budgets satisfied:"` summary, insert:

```bash
# 6. Hybrid search latency: < 150 ms (median of criterion estimates.json)
# bench path: target/criterion/hybrid_search_latency/new/estimates.json
# (same single-level path as search_latency_p95 and semantic_search_latency.)
HYBRID_NS=$(read_median_ns "hybrid_search_latency")
HYBRID_MS=$(awk -v ns="$HYBRID_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$HYBRID_MS" 'BEGIN { exit !(v >= 150) }'; then
    echo "FAIL: hybrid search latency ${HYBRID_MS} ms exceeds 150 ms" >&2
    exit 16
fi
```

Then extend the summary block at the bottom of the script:

```bash
echo "All perf budgets satisfied:"
echo "  binary size:       ${SIZE_BYTES} bytes (limit ${SIZE_LIMIT})"
echo "  cold start (p50):  ${COLD_START_P50} ms (limit 200)"
echo "  ingest throughput: ${THROUGHPUT} items/s (limit 50)"
echo "  search latency:    ${QUERY_MS} ms (limit 100)"
echo "  semantic search:   ${SEM_MS} ms (limit 100)"
```

becomes:

```bash
echo "All perf budgets satisfied:"
echo "  binary size:       ${SIZE_BYTES} bytes (limit ${SIZE_LIMIT})"
echo "  cold start (p50):  ${COLD_START_P50} ms (limit 200)"
echo "  ingest throughput: ${THROUGHPUT} items/s (limit 50)"
echo "  search latency:    ${QUERY_MS} ms (limit 100)"
echo "  semantic search:   ${SEM_MS} ms (limit 100)"
echo "  hybrid search:     ${HYBRID_MS} ms (limit 150)"
```

- [ ] **Step 3: Run the script locally to verify it passes**

Run: `bash .github/scripts/perf-check.sh`

Expected: exits 0 with the six-budget summary line. (This re-runs `cargo build --release` + all benches; takes several minutes. If you only want to verify the script change without re-benching, ensure `target/criterion/hybrid_search_latency/new/estimates.json` exists from Task 13 first; the script will reuse it via `cargo bench`'s incremental output.)

- [ ] **Step 4: Commit**

```bash
git add .github/scripts/perf-check.sh
git commit -s -m "ci(perf): add 6th budget — hybrid_search_latency < 150 ms

Exit code 16 distinguishes hybrid budget failure from the existing
five. Comment header + summary print updated to reflect six budgets."
```

---

## Task 15: Final gate — workspace lint, full test suite, Cargo.lock

**Files:** verification only; commit only `Cargo.lock` if it changed.

- [ ] **Step 1: Run rustfmt across the workspace**

Run: `cargo fmt --check`

Expected: clean. If not, `cargo fmt`, review diff, commit separately.

- [ ] **Step 2: Run clippy across the workspace**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`

Expected: zero warnings.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --workspace`

Expected: PASS for every test in every crate (core + search + root binary).

- [ ] **Step 4: Run the full perf-check script**

Run: `bash .github/scripts/perf-check.sh`

Expected: exits 0 with all six budgets satisfied.

- [ ] **Step 5: Verify rustdoc still builds with no missing-doc warnings**

Run: `RUSTDOCFLAGS='-D missing-docs' cargo doc --workspace --no-deps`

Expected: builds with no errors. (Sub-project 2b's doc audit added missing field docs throughout `error.rs`; the two new variants in Task 2 must already satisfy this. If `--show-ranks` reveals a missing doc, fix it inline.)

- [ ] **Step 6: Check for Cargo.lock changes**

Run: `git status Cargo.lock`

If `Cargo.lock` was modified (likely yes, because adding `serde_json::to_writer` for `HybridSearchResults` does not pull new deps but adding `serde::Serialize` derive sometimes nudges lockfile entries; also re-running `cargo build` may have refreshed metadata):

```bash
git add Cargo.lock
git commit -s -m "chore: refresh Cargo.lock after hybrid search additions"
```

If `git status` shows no change, skip the commit.

- [ ] **Step 7: Final repository status check**

Run: `git status`

Expected: clean working tree.

Run: `git log --oneline -20`

Expected: the new commits from Tasks 1–14 (plus optional Task 15) sit on top of `eb4e441d8476b62e6e59b026feba3124db2ab055` (sub-project 2b merge) and `6bda51b` (the design spec commit).

---

## Self-review

**1. Spec coverage check** (each spec requirement → task):

| Spec section / requirement | Task |
|---|---|
| `hybrid_query.rs` module + types | 3 |
| `HybridSearcher` struct + 3 constructors | 4 |
| `rrf_fuse` (RRF formula, k=60) | 5 |
| Single-ranker search paths | 6 |
| Fused hybrid search path | 7 |
| `Error::NoIndexes` + `Error::HybridMissingIndex` | 2 |
| `ItemId` `Ord + PartialOrd` derive | 1 |
| CLI `--mode`/`--fetch-multiplier`/`--rrf-k`/`--show-ranks`/`--json` | 9, 10, 11 |
| `--mode auto` degradation behaviour | 10 |
| Explicit `--mode` strict failure | 10 |
| `semantic-search` deprecated alias + one-shot note | 12 |
| Score-kind output tags (`rrf=`/`bm25=`/`cos=`) | 10 |
| `hybrid_search_latency` bench | 13 |
| 6th Principle X budget (150 ms) | 14 |
| `docs/formats/store-v1.md` unchanged | (no task — verified by self-review) |
| Acceptance criterion 9: all 9 unit + 11 CLI tests pass | Distributed across Tasks 3, 4, 5, 6, 7, 9, 10, 11, 12 |
| Acceptance criterion 10: perf budget blocking | 14 |
| Acceptance criterion 12: tag `v0.4.0` on merge | (out of plan scope — handled by maintainer during PR merge) |

All twelve acceptance criteria from the spec have a task that satisfies them.

**2. Placeholder scan:** no TBDs, no "implement later", no "add appropriate error handling", no "similar to Task N". Every task contains the complete code or shell command needed.

**3. Type consistency:** field names checked:
- `HybridSearchOptions.{limit, fetch_multiplier, rrf_k, include_snippets}` consistent across Tasks 3, 6, 7, 10, 12, 13.
- `HybridHit.{id, score, score_kind, lexical_rank, semantic_rank, snippet}` consistent across Tasks 3, 6, 7, 10, 11.
- `ScoreKind::{Rrf, Bm25, Cosine}` consistent across Tasks 3, 6, 7, 10.
- `HybridSearcher::{new, lexical_only, semantic_only, search}` consistent across Tasks 4, 6, 7, 10, 13.
- `Error::{NoIndexes, HybridMissingIndex { missing, path }}` consistent across Tasks 2, 10.
- `SearchMode::{Auto, Lexical, Semantic, Hybrid}` consistent across Tasks 9, 10, 12.

**4. Open questions resolution:** the spec's two open notes are handled:
- `ItemId` `Ord` derive lands in Task 1 (not deferred).
- `HybridHit.score` + `score_kind` discriminator (single-score shape) lands in Task 3 (not separate per-ranker fields).

Plan ready for execution.
