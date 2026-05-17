---
title: Singularmem — Search v0 (Embeddings + Vector index, sub-project 2b)
date: 2026-05-17
status: draft
sub-project: 2b-search-v0-embeddings
supersedes: none
---

# Singularmem — Search v0 (Embeddings + Vector index, sub-project 2b)

This is sub-project **2b** of Singularmem, the embeddings + vector slice of
the broader "Search v0" piece named in the constitution's Open / Closed
Split. It extends `singularmem-search` with an ONNX-based `Embedder`, a
USearch-backed `VectorIndex`, the new `EmbedderIndex` (a second `IndexHook`
implementation alongside Tantivy's), a `MultiHook` composite in
`singularmem-core`, and a new `semantic-search` CLI verb.

It does **not** ship hybrid retrieval (combined lexical + vector ranking) —
that is sub-project 2c.

## Problem & motivation

Sub-project 2a (Lexical) added Tantivy-backed full-text search. That solves
"find items containing these words" but not "find items semantically
related to this passage". For a memory layer whose purpose is recall over
accumulated artefacts, semantic search is the load-bearing missing piece:
users often remember an idea or a phrase that's adjacent to — but not
literally present in — the item they want.

Sub-project 2b adds local-first semantic search via ONNX embeddings stored
in a USearch HNSW index. It establishes the multi-hook pattern that
sub-project 2c (hybrid retrieval) needs and validates that the
constitution's binary-size budget (Principle X) has headroom for the ONNX
runtime.

## Goals & non-goals

### Goals

1. Ship an `Embedder` trait + `FastembedEmbedder` concrete impl in
   `crates/singularmem-search`.
2. Ship a `VectorIndex` (USearch wrapper) + `EmbedderIndex` (the trait's
   `IndexHook` implementation) in the same crate.
3. Add `MultiHook` to `singularmem-core::hook` so `Store` can fan out
   ingest calls to multiple `IndexHook` instances (the additive change is
   the only `singularmem-core` change in this sub-project).
4. Add `singularmem semantic-search <query>` CLI verb + `singularmem
   reindex --with-embeddings` flag.
5. Document the on-disk sidecar format in
   `docs/formats/store-v1.md` (additive section; no `format_version` bump).
6. Establish the **cheap-vs-heavy default rule** in `CONTRIBUTING.md`:
   features requiring downloads or non-trivial per-op cost are opt-in;
   trivial features default-on.
7. Bump the workspace version to `0.3.0` and tag `v0.3.0` after merge.

### Non-goals

- Hybrid retrieval (combining lexical + vector scores) — sub-project 2c.
- Sliding-window chunking for long items — deferred to v0.4+.
- Multiple vectors per item — deferred (changes the storage shape).
- Non-English embedding models — the curated catalogue is the three
  English models from Section 2; users wanting other languages use
  `FastembedEmbedder::from_files` with their own ONNX weights.
- GPU inference — CPU-only via ONNX runtime.
- LLM provider integration — sub-project 3.
- MCP server — sub-project 4.
- TypeScript SDK binding — sub-project 5.
- Re-promoting `tests-offline` to blocking — explicit non-goal carried
  over from 2a.
- A `singularmem config` command — opt-in is via `reindex --with-embeddings`
  creating the `.vectors/` directory; persistent config in
  `singularmem_meta` is out of scope.

## Recommended approach

**Approach A — Extend `singularmem-search` with embedder + vector-index
modules.** New modules inside the existing crate: `embedder.rs` (Embedder
trait + FastembedEmbedder), `model.rs` (model registry + caching),
`vector_index.rs` (USearch wrapper + EmbedderIndex), `semantic_query.rs`
(search execution). Add deps `fastembed = "=4.4.0"` and
`usearch = "=2.15.3"`. The existing `IndexHook` trait gets a second
implementation (`EmbedderIndex`); `Store` gets a tiny extension to support
multiple hooks via a `MultiHook` composite. Sub-project 2c naturally lives
in the same crate.

### Approaches discarded

- **Approach B — New crate `singularmem-vectors`.** Tidier per-crate
  concerns. Rejected: artificial split — both crates are "search" and 2c
  will need to depend on both. Workspace fragments without benefit.
- **Approach C — Two new crates `singularmem-embedder` +
  `singularmem-vector-index`.** Maximum isolation. Rejected: nobody
  outside Singularmem will consume just the embedder or just the vector
  index. Both exist to serve `singularmem-search`.

## Architecture

`crates/singularmem-search/` gains four new modules:

```
crates/singularmem-search/
├── Cargo.toml              # +fastembed = "=4.4.0", +usearch = "=2.15.3"
└── src/
    ├── lib.rs              # re-export new public types
    ├── (existing 2a modules)
    ├── embedder.rs         # Embedder trait + FastembedEmbedder impl
    ├── model.rs            # EmbeddingModel enum, model paths, cache helpers
    ├── vector_index.rs     # VectorIndex (USearch wrapper) + EmbedderIndex
    └── semantic_query.rs   # SemanticQuery + EmbedderIndex::semantic_search
```

**Workspace dependency additions:**

```toml
fastembed = { version = "=4.4.0", default-features = false, features = ["ort-download-binaries"] }
usearch = "=2.15.3"
```

Both exact-pinned per the rusqlite / tantivy convention. Pinning rationale:
- `fastembed`: model API + ONNX format coupling.
- `usearch`: on-disk binary format is sensitive to version drift; pinning
  makes the necessary reindex on bump explicit.

**Estimated binary-size impact:** ONNX runtime ~25–35 MB + USearch ~3–5 MB
+ fastembed glue ~5 MB. v0.2.0 was ~35 MB; v0.3.0 estimated ~70 MB (well
under 150 MB budget).

**`singularmem-core` change:** small + additive — a new `MultiHook` type in
`hook.rs` + a new `Store::open_with_hooks(path, Vec<Box<dyn IndexHook>>)`
constructor. Existing `Store::open` and `Store::open_with_hook` are
unchanged.

**Concurrency.** `VectorIndex` wraps `usearch::Index` in a `Mutex` (same
pattern as the SQLite connection and Tantivy writer). Single-writer for
v0.3.0; concurrent search reads serialize through the mutex briefly. USearch
itself supports lock-free reads after construction, but our wrapper
serializes for v0.3.0 simplicity.

## Data model

### Embedder trait

```rust
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    fn model_id(&self) -> &str;
    fn embed(&self, content: &str) -> Result<Vec<f32>>;
    fn embed_batch(&self, items: &[&str]) -> Result<Vec<Vec<f32>>>;
}
```

Synchronous (CPU-bound; no tokio runtime). Output vectors are unit-length
(L2-normalized) so cosine similarity reduces to dot product.

### FastembedEmbedder concrete implementation

```rust
pub struct FastembedEmbedder { /* inner: fastembed::TextEmbedding, model_id, dim */ }

impl FastembedEmbedder {
    pub fn new() -> Result<Self>;                     // default: all-MiniLM-L6-v2
    pub fn with_model(model: EmbeddingModel) -> Result<Self>;
    pub fn from_files(model_dir: &Path, model_id: &str) -> Result<Self>;
}

pub enum EmbeddingModel {
    AllMiniLmL6V2,       // 384-dim, ~80 MB, English-focused, fast. Default.
    BgeSmallEnV15,       // 384-dim, ~130 MB, English, slightly higher quality.
    NomicEmbedTextV15,   // 768-dim, ~250 MB, English, larger context.
}
```

**Default model: `all-MiniLM-L6-v2`** (384-dim). Small enough that the
download isn't painful (~80 MB) once a user opts in, fast on CPU (~10–15
ms per text), wide ecosystem compatibility.

**Long-text handling — truncate-to-first-N-tokens in v0.3.0.** Each model
has a hard input token limit (all-MiniLM-L6-v2 = 256 tokens). For items
longer than the limit, we truncate the tokenized input and embed the
prefix; truncation emits `tracing::warn!` naming the item ID (Principle
VII: honest failure). Sliding-window chunking is deferred to v0.4+.

**Model weights cache location:** `dirs::cache_dir()/singularmem/models/`.
Distinct from `dirs::data_dir()` so deleting all stores doesn't waste the
download.

**Determinism.** Given identical model weights + identical input, the
output vector is byte-identical. Property-tested for Principle VI
compliance.

### VectorIndex (USearch wrapper)

```rust
pub struct VectorIndex {
    pub(crate) inner: Mutex<usearch::Index>,
    pub(crate) path: PathBuf,
    pub(crate) meta_path: PathBuf,
    pub(crate) meta: VectorIndexMeta,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct VectorIndexMeta {
    pub format_version: String,       // "1"
    pub model_id: String,             // e.g. "sentence-transformers/all-MiniLM-L6-v2@v1"
    pub dim: usize,                   // 384
    pub distance: String,             // "cosine"
    pub hnsw_m: usize,                // 16
    pub hnsw_ef_construction: usize,  // 128
    pub created_at: jiff::Timestamp,
}
```

**HNSW parameters** (defaults):
- `m = 16`, `ef_construction = 128`, `expansion_search = 64`, distance =
  cosine.
- Documented in the format spec so third-party loaders can use compatible
  params.

**ItemId ↔ USearch key.** USearch uses `u64` keys. Our `ItemId` is a
128-bit ULID. We use a `BTreeMap<u64, ItemId>` in memory mapping sequential
u64 keys to ULIDs, persisted as `keymap.bin` (bincode-serialized). At 100K
items the map is ~3 MB — acceptable. v0.4+ may switch to a more compact
representation.

### EmbedderIndex (IndexHook impl)

```rust
pub struct EmbedderIndex {
    embedder: Box<dyn Embedder>,
    vector_index: VectorIndex,
}

impl singularmem_core::IndexHook for EmbedderIndex {
    fn on_ingest(&self, item: &Item) -> singularmem_core::Result<()>;
    fn on_reindex(&self, item: &Item) -> singularmem_core::Result<()>;
    fn commit(&self) -> singularmem_core::Result<()>;
}
```

`on_ingest` and `on_reindex` both embed the item's content + add to the
vector index. `commit` saves the USearch file + meta.json to disk.

### MultiHook composite

```rust
pub struct MultiHook {
    hooks: Vec<Box<dyn IndexHook>>,
}

impl IndexHook for MultiHook {
    fn on_ingest(&self, item: &Item) -> Result<()> {
        let mut first_err: Option<Error> = None;
        for (i, hook) in self.hooks.iter().enumerate() {
            if let Err(e) = hook.on_ingest(item) {
                tracing::warn!(hook_index = i, item_id = %item.id, error = %e,
                    "MultiHook member failed on_ingest; other hooks will still run");
                if first_err.is_none() { first_err = Some(e); }
            }
        }
        first_err.map_or(Ok(()), Err)
    }
    fn on_reindex(&self, item: &Item) -> Result<()> { /* same shape */ }
    fn commit(&self) -> Result<()> { /* same shape */ }
}
```

**Critical Principle VII property:** `Store::ingest` already swallows hook
errors into `tracing::warn!` and does NOT roll back SQLite. Adding
`MultiHook` preserves this — even if both hooks fail, the SQLite write
stands and the user gets two warnings naming what's now stale.
`singularmem reindex` recovers both.

## On-disk format

Sidecar directory next to the SQLite + Tantivy sidecars:

```
~/.local/share/singularmem/
├── store.db                 # SQLite (canonical, format_version=1)
├── store.db.tantivy/        # Tantivy sidecar (2a, optional)
└── store.db.vectors/        # USearch sidecar (2b, optional, NEW)
    ├── .meta.json           # VectorIndexMeta
    ├── index.usearch        # USearch HNSW binary file
    └── keymap.bin           # bincode-serialized BTreeMap<u64, ItemId>
```

**Format-version impact: none.** SQLite `format_version` stays at `"1"`.
Both sidecars are additive — third-party loaders that only read SQLite
continue to work.

**Rebuild from SQLite.** The entire `.vectors/` directory can be deleted at
any time. The next `singularmem reindex --with-embeddings` regenerates
everything from `items.content` + the chosen embedding model. This is the
Principle III.b commitment for vector data: canonical content is in
SQLite; vector representation is derived + regeneratable.

**Format spec update.** `docs/formats/store-v1.md` gains a new top-level
`##` section titled "USearch vector sidecar (optional)" with the directory
layout, `VectorIndexMeta` JSON schema, `keymap.bin` bincode schema, HNSW
parameters, USearch version compatibility note, and a "writing a
third-party vector loader" walkthrough.

## Interfaces

### Library (`singularmem-search` additions)

```rust
// lib.rs re-exports add:
pub use crate::embedder::{Embedder, FastembedEmbedder};
pub use crate::model::EmbeddingModel;
pub use crate::vector_index::{
    VectorIndex, VectorIndexMeta, VectorIndexOptions, EmbedderIndex, VectorHit,
};
pub use crate::semantic_query::{
    SemanticSearchOptions, SemanticSearchResults, SemanticHit,
};
```

**EmbedderIndex search method:**

```rust
impl EmbedderIndex {
    pub fn semantic_search(
        &self,
        query: &str,
        opts: SemanticSearchOptions,
    ) -> Result<SemanticSearchResults>;
}

pub struct SemanticSearchOptions {
    pub limit: usize,        // default 20
    pub min_score: f32,      // default 0.0
}

pub struct SemanticSearchResults {
    pub hits: Vec<SemanticHit>,
    pub elapsed: std::time::Duration,
    pub total_indexed: u64,
}

pub struct SemanticHit {
    pub id: ItemId,
    pub score: f32,   // cosine [-1.0, 1.0]; higher = more similar
}
```

### CLI

```
singularmem semantic-search <QUERY>...
    [--limit N]              # max hits; default 20
    [--min-score FLOAT]      # cosine threshold; default 0.0
    [--format <FMT>]         # table | jsonl | ids; default table

singularmem reindex
    [--with-embeddings]      # NEW — also rebuild the vector index
    [--embedding-model M]    # NEW — all-mini-lm-l6-v2 | bge-small-en | nomic-embed
    [--reset-vectors]        # NEW — destructive; deletes .vectors/ before reindex
    [--force]                # NEW — required with --reset-vectors
    [--quiet]
```

**Exit codes for `semantic-search`:**
- `0` Success (hits OR zero matches)
- `1` Usage error
- `2` Vector index missing — message names `reindex --with-embeddings`
- `3` Embedder model mismatch — message names `reindex --with-embeddings --reset-vectors --force`

**Auto-wiring mechanic.** If `<store>.vectors/` exists, the root binary's
`open_store` builds a `MultiHook` containing both the Tantivy hook (from
2a) AND the EmbedderIndex hook, and attaches it via `Store::set_hook`.
Auto-wiring fires ONLY for `Ingest` commands (preserves the writer-lock
conflict avoidance from 2a; USearch also has writer semantics).

### Wire (MCP, HTTP, etc.)

None in this sub-project. Sub-project 4 introduces the MCP server, which
will consume the search library through its public API.

## Error handling

New Error variants in `singularmem_search::Error`:

```rust
pub enum Error {
    // (existing 2a variants)

    /// Embedding inference failed.
    #[error("embedding inference failed during {context}: {reason}")]
    Embedding { context: &'static str, reason: String },

    /// Model weight download failed.
    #[error("could not download embedding model {model}: {reason}")]
    ModelDownload { model: String, reason: String },

    /// Model file invalid (missing files in from_files path).
    #[error("invalid model files at {path}: {reason}; expected ONNX weights + tokenizer")]
    InvalidModelFiles { path: PathBuf, reason: String },

    /// Vector dimension mismatch (Embedder vs VectorIndex).
    #[error("vector dimension mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },

    /// Stored vector index uses a different model than the one opening it.
    #[error("vector index at {path} was built with model {found_model}; \
             current Embedder uses {expected_model}; \
             run `singularmem reindex --with-embeddings --reset-vectors --force` to rebuild")]
    ModelMismatch {
        path: PathBuf,
        found_model: String,
        expected_model: String,
    },

    /// USearch error.
    #[error("USearch error during {context}: {reason}")]
    Usearch { context: &'static str, reason: String },
}
```

Each variant names what failed, what was attempted, and what state was
preserved (Principle VII). The `ModelMismatch` and "vector index missing"
errors include the recovery command verbatim.

## Testing strategy

**Test layout** for `crates/singularmem-search/`:

```
crates/singularmem-search/
├── src/*.rs                # `#[cfg(test)] mod tests` — pure-function tests
├── tests/
│   ├── (existing 2a tests)
│   ├── embedder.rs             # Embedder trait: dim, determinism, batch=single, truncation warning
│   ├── vector_index.rs         # VectorIndex: add/search round-trip, model mismatch, keymap persistence
│   ├── embedder_to_search.rs   # End-to-end: Store + EmbedderIndex → semantic_search returns ingested items
│   ├── multi_hook.rs           # MultiHook fans out; failing member doesn't block others
│   └── (extend concurrency.rs) # Parallel readers during embedder reindex
└── benches/
    └── embedder_perf.rs        # NEW: embed throughput; batch speedup; query embed latency
```

Plus extend `crates/singularmem-core/tests/hook.rs` with `MultiHook`
Principle VII tests.

**MockEmbedder for unit tests.** A deterministic-pseudo-hash `Embedder`
impl that returns the same vector for the same content but doesn't require
ONNX runtime or model downloads. Lives in `singularmem-search/src/testing.rs`
behind `#[cfg(any(test, feature = "testing"))]`. Tests use this; real-network
integration tests use `FastembedEmbedder::new()` behind `#[ignore]` so they
only run when explicitly opted in.

**Principle VII compliance test** (new `tests/multi_hook.rs`):

`failing_embedder_does_not_prevent_tantivy`: wires a real Tantivy hook +
a FailingEmbedderHook. Ingests an item. Asserts:
- `Store::ingest` returns `Ok`
- Item is in SQLite
- Item IS searchable in Tantivy
- Vector index does NOT have the item
- `tracing::warn!` fired with the embedder's failure

**Property tests** (extend `tests/property.rs`):

- `embed_is_deterministic`: same content + same model → byte-identical
  vectors (32 cases).
- `ingest_then_semantic_search_finds_self`: ingested item is its own
  nearest neighbour with cosine ≥ 0.95 (32 cases, uses MockEmbedder for
  speed).

**Concurrency** (extend `tests/concurrency.rs`):

- 8 reader threads doing `semantic_search` calls during a long
  `reindex --with-embeddings`. Readers always get consistent results
  (either pre-reindex set or post-reindex set; never mixed mid-flight).

**Principle III.b round-trip preserved.** `tests/format.rs::open_core_only_round_trip`
continues to pass unchanged — the search crate's new code doesn't add any
dep into `singularmem-core` beyond the additive `MultiHook` type.

**`tests-offline` integration.** Embedder + USearch don't add runtime
network deps. `FastembedEmbedder::new()` DOES download model weights on
first use — but tests use `MockEmbedder` or `FastembedEmbedder::from_files`,
never `new()` in `cargo test`. The `tests-offline` advisory job's
behaviour is unchanged.

## Performance budgets in CI

`perf-check.sh` gains two new reads from criterion estimates.json:

- `embed_throughput`: items per second through `FastembedEmbedder::embed_batch`.
- `semantic_search_latency`: median of `EmbedderIndex::semantic_search` calls
  against a 10K-item seeded store.

Plus the existing four budgets stay enforced.

**Pre-flight estimates and risks:**

| Budget | v0.2.0 measured | v0.3.0 estimate (embedder wired) | Risk |
|---|---|---|---|
| Binary size | ~35 MB | ~70 MB (ONNX ~30 + USearch ~5) | Medium — well under 150 MB but eats headroom |
| Cold start | ~10 ms | ~10 ms (embedder lazy; not in --version path) | Low |
| Ingest throughput (single-item, embedder on) | ~50/s | **~20/s — VIOLATES 50/s** | **HIGH** |
| Ingest throughput (`ingest_many`, embedder on) | ~200/s | ~100-200/s (batched ONNX) | Low |
| Search latency (semantic) p95 | N/A | ~15-25 ms (embed query + KNN) | Low — well under 100 ms |

**Single-item ingest regression treatment.** Mirrors sub-project 2a's
approach for Tantivy single-item ingest:

1. `ingest_many` is the canonical bulk path for embeddings-on stores —
   documented in CONTRIBUTING.md, the CLI's `ingest --help`, and the README.
2. `tracing::info!` on the first slow single-item ingest mentions
   `ingest_many` for bulk imports.
3. Constitution Principle X budget is NOT amended; the budget reads
   "ingest throughput ≥ 50 items/s on the reference runner" and
   `ingest_many` meets it. Single-item is a recognised slow path.

If measurement shows single-item < 20/s on the reference runner, escalate
to "real regression" and consider deferred-commit background batching
(architectural change deferred from 2a).

**CI workflow updates** — minimal:

- `perf-budgets` stays blocking. Gains the two new bench reads + a fifth
  budget check (semantic_search_latency).
- `tests-offline` stays advisory. Explicit non-goal carried from 2a.

## Open questions

The implementation plan must resolve, but they are operational rather than
design:

1. **Exact fastembed + usearch versions.** `=4.4.0` and `=2.15.3` are
   plan-write-time current; verify they remain current at execution time
   and re-pin to whichever non-yanked patch is freshest. (Same lesson as
   tantivy 0.22.0 being yanked in sub-project 2a — implementer should
   double-check yank status.)
2. **`from_files` user docs.** Air-gapped users need a short docs page
   explaining how to download weights manually + point `FastembedEmbedder::from_files`
   at them. Deferred to a separate docs PR; spec mentions but doesn't
   draft the page.
3. **`--reset-vectors` UX safety.** The `--force` requirement is in the
   spec; the plan should ensure the CLI's confirmation message is clear
   (e.g., "this will delete <N> indexed embeddings and re-embed from
   scratch; ~estimated 5 minutes on this machine"). Plan picks the
   wording.

## Acceptance criteria

Sub-project 2b is done when *all* of these are observable on `main`:

1. **`crates/singularmem-search/`** gains four new modules
   (`embedder.rs`, `model.rs`, `vector_index.rs`, `semantic_query.rs`).
   `cargo doc -p singularmem-search` builds clean; every `pub` item has a
   doc comment.
2. **`fastembed = "=4.4.0"` + `usearch = "=2.15.3"`** in
   `[workspace.dependencies]` with the documented feature flags. Cargo.lock
   locks the transitive graph.
3. **`docs/formats/store-v1.md`** gains the "USearch vector sidecar
   (optional)" section per the design — directory layout,
   `VectorIndexMeta` JSON schema, `keymap.bin` bincode schema, HNSW
   params, USearch version compatibility.
4. **`singularmem-core::hook::MultiHook`** + `Store::open_with_hooks` —
   existing `Store::open` and `Store::open_with_hook` unchanged (verified
   by recompiling the v0.2.0 integration tests).
5. **Embedder works deterministically** — `FastembedEmbedder::new()` with
   `all-MiniLM-L6-v2` returns 384-dim unit vectors; same content → byte-
   identical vectors; truncation of long content triggers `tracing::warn!`.
6. **VectorIndex works end-to-end** — add → search round-trip;
   model-mismatch open returns `Error::ModelMismatch`; keymap.bin
   persists across opens; `remove` succeeds on absent IDs without panic.
7. **`singularmem reindex --with-embeddings`** end-to-end on fresh store
   and on re-run; with `--embedding-model bge-small-en` on a store created
   with `all-MiniLM-L6-v2` → fails with `Error::ModelMismatch` unless
   `--reset-vectors --force` is also passed.
8. **`singularmem semantic-search`** end-to-end — semantic recall with
   cosine ≥ 0.5 for paraphrased queries; missing index → exit 2 with
   recovery hint; model mismatch → exit 3.
9. **Auto-wiring is opt-in.** Fresh v0.3.0 store with no `.vectors/` dir
   behaves identically to v0.2.0 for `ingest`. After `reindex
   --with-embeddings` creates the dir, subsequent ingests auto-wire the
   embedder.
10. **MultiHook Principle VII compliance** verified by
    `tests/multi_hook.rs::failing_embedder_does_not_prevent_tantivy`.
11. **Principle III.b round-trip preserved** — `crates/singularmem-core/tests/format.rs::open_core_only_round_trip`
    continues to pass unchanged.
12. **Principle X budgets** measured on `ubuntu-latest`:
    - Binary size < 150 MB (estimated ~70 MB).
    - Cold start < 200 ms.
    - Search latency p95 < 100 ms for both `search` AND `semantic-search`.
    - Ingest throughput ≥ 50/s via `ingest_many`; single-item documented
      as recognised slow path.
13. **`perf-budgets` CI job green** with the new semantic_search_latency
    check added to `perf-check.sh`. `tests-offline` stays advisory.
14. **`CONTRIBUTING.md` documents the cheap-vs-heavy default rule.**
    Features requiring downloads or non-trivial per-op cost are opt-in;
    trivial features default-on.
15. **Version bump to `0.3.0`** in workspace `Cargo.toml`.
    `singularmem --version` prints `singularmem 0.3.0`. Tag `v0.3.0`
    pushed after merge.
16. **No `[PLACEHOLDER]` strings** in any committed file under
    `docs/formats/`, `crates/singularmem-search/src/**/*.rs`, or new
    public API doc comments.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | All inference runs locally via ONNX runtime. Model weights download from HuggingFace once on opt-in (`reindex --with-embeddings`) — single network call, scoped + explicit + user-initiated. No telemetry. System is fully offline-operable after download. |
| **II — Provider-Agnostic by Contract** | No LLM provider integration. Embedding model selection is local (fastembed catalogue + bring-your-own ONNX), not provider-tied. |
| **III — Open Core with a Stable Boundary** | Wholly open. III.b preserved by the unchanged `open_core_only_round_trip` test — `singularmem-core` gains only the additive `MultiHook` type, no Tantivy / USearch / fastembed deps. The vector sidecar is documented + regeneratable from canonical SQLite content + the named model. |
| **IV — CLI-First, GUI-Visible** | One new CLI verb (`semantic-search`) + four new flags on `reindex` expose every new library capability. No GUI work. |
| **V — Composable Library Architecture** | `Embedder` is a public trait with `FastembedEmbedder` as the concrete impl; consumers can swap in alternatives (`MockEmbedder` for tests; bring-your-own via `from_files`). `VectorIndex` is its own type usable without an Embedder for callers supplying pre-computed vectors. `EmbedderIndex` composes both and implements `IndexHook`. |
| **VI — Deterministic and Offline-Testable** | Embedding is deterministic given fixed weights + identical input (property-tested). Tests use `MockEmbedder` to avoid network. The advisory `tests-offline` job continues; embedder-related tests don't add new network requirements. |
| **VII — Honest Failure Modes** | `MultiHook` log-and-continue isolation. Model-mismatch on open is an explicit `Error::ModelMismatch` (not silent re-embedding). Truncation of long content emits `tracing::warn!`. Single-item-ingest throughput regression is explicitly documented in spec + CLI help. |
| **VIII — Privacy Telemetry Boundary** | No telemetry added. Model download from HuggingFace is the only network call; explicit, one-time, scoped. |
| **IX — Accessible by Default (WCAG 2.2 AA)** | CLI-only surface. clap output stays plain text. Progress reporting via `tracing::info!`. Reduce-motion N/A. |
| **X — Performance Budgets, Enforced in CI** | Three of four budgets comfortably met. Ingest throughput meets 50/s via `ingest_many`; single-item with embedder regresses to ~20/s and is documented as a recognised slow path (mirrors sub-project 2a's pattern for Tantivy single-item ingest). The constitution's budget is not amended. |
