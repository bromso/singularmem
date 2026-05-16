---
title: Singularmem — Search v0 (Lexical, sub-project 2a)
date: 2026-05-16
status: draft
sub-project: 2a-search-v0-lexical
supersedes: none
---

# Singularmem — Search v0 (Lexical, sub-project 2a)

This is sub-project **2a** of Singularmem, the lexical-search slice of
the broader "Search v0" piece named in the constitution's Open / Closed
Split. It adds a new crate `singularmem-search` that wraps a Tantivy
index, hooks into `Store::ingest` for live writes, and exposes two new
CLI verbs: `search` and `reindex`.

It does **not** ship embeddings, vector indexing, or hybrid retrieval —
each of those is its own sub-sub-project (2b, 2c) that follows in its
own brainstorm cycle.

## Problem & motivation

After sub-project 1 (Memory Store v0), Singularmem can ingest, list,
and export items but has no way to *find* them other than enumerating
all items and visually scanning. For a memory layer whose purpose is to
make a developer's accumulated artefacts retrievable, this is the
load-bearing missing feature.

The constitution's Open / Closed Split commits to lexical (Tantivy) +
vector + embeddings as one item. Decomposing it into three
sub-sub-projects matches the "smallest viable surface" pattern that
sub-projects 0 and 1 followed: 2a (lexical) is genuinely useful on its
own, exercises the perf-budget gate (Principle X), and unlocks search
as a feature without committing to the ONNX/embedding/vector-index
complexity that 2b will require.

The first sub-project where every Principle X budget becomes
genuinely meaningful: query latency was N/A in v0.1.0; ingest
throughput now pays for the dual-write to Tantivy; cold start adds
Tantivy crate init; binary size grows by Tantivy + dependencies. All
four budgets get measured here for the first time.

## Goals & non-goals

### Goals

1. Ship `crates/singularmem-search` — a standalone Rust library whose
   public API exposes a `TantivyIndex`, query/result types, and a
   reindex driver.
2. Add a minimal `IndexHook` trait to `singularmem-core` so the search
   crate (or any future search implementation) can be wired in at
   `Store::open_with_hook` time without coupling the core crate to
   Tantivy.
3. Implement live writes: `Store::ingest` and `Store::ingest_many` call
   the hook after the SQLite commit. Failures are surfaced as
   `tracing::warn!` and do NOT roll back the SQLite write
   (Principle VII compliance via asymmetric failure semantics).
4. Add two CLI verbs: `singularmem search <QUERY>` (Tantivy
   QueryParser-style) and `singularmem reindex` (full rebuild from
   SQLite).
5. Document the Tantivy sidecar layout in `docs/formats/store-v1.md`
   as an optional, additive on-disk artefact that does NOT bump
   `format_version` (third-party loaders that only read SQLite
   continue to work — Principle III.b preserved).
6. Re-promote the `perf-budgets` CI job to blocking by hardening the
   criterion-output parser (switch to `--output-format=json` per the
   v0.1.0 follow-up note).
7. Bump the workspace version to `0.2.0` and tag `v0.2.0` after merge.

### Non-goals

- Embeddings (ONNX runtime, model selection) — sub-project 2b.
- Vector indexing (LanceDB / Qdrant-embedded / USearch selection) —
  sub-project 2b.
- Hybrid retrieval (merging lexical + vector results) — sub-project 2c.
- LLM provider integration — sub-project 3.
- MCP server — sub-project 4.
- TypeScript SDK binding — sub-project 5.
- A `singularmem index-info` verb — deferred (`Index::doc_count` exists
  in the lib for SDK consumers).
- A `--latest-only` query filter that excludes superseded items —
  deferred to a later sub-project.
- A CLI `import` verb that re-loads a JSONL export — still deferred
  from sub-project 1.
- Incremental Tantivy schema migration tooling — v1 Tantivy schema is
  the only one in v0.2.0; future schema changes get migrators.
- Multi-language tokenizers — English-style default only.
- Re-promoting `tests-offline` to blocking — explicit non-goal; a
  separate "CI infrastructure" sub-project will harden the
  namespace-availability story.

## Recommended approach

**Approach A — New crate `singularmem-search` + minimal `IndexHook`
trait in `singularmem-core`.** A new workspace member
`crates/singularmem-search` owns the Tantivy schema, indexer, query
logic, and reindex driver. `singularmem-core` gains a tiny pub
`IndexHook` trait that `Store::ingest`/`ingest_many` calls on every
write; the search crate provides a `TantivyIndex` implementation.
`Store::open_with_hook(path, Box<dyn IndexHook>)` wires the hook;
plain `Store::open` keeps the v0.1.0 behaviour (no search, faster).
The root binary picks up `singularmem-search` via a path dep and
auto-wires the hook unless `--no-index` is passed.

### Approaches discarded

- **Approach B — Add search directly to `singularmem-core`.** Tantivy
  becomes a direct dep of the existing crate. Smaller workspace; tighter
  coupling. Rejected: makes `singularmem-core` a heavier dep for
  downstream consumers who only want storage (the TS SDK in sub-project
  5, the MCP server in sub-project 4). Tantivy + its dependencies are
  non-trivial (~5 MB more).
- **Approach C — Search as an entirely separate binary that reads the
  SQLite store directly.** No core integration; no hook. Rejected:
  breaks the Principle IV "every CLI capability through one binary"
  pattern, and the search index needs to be updated atomically with
  writes — which means the search binary needs to hook into ingest,
  which means we're back to Approach A or B.

## Architecture

The workspace gains one new member: `crates/singularmem-search`.

```
crates/singularmem-search/
├── Cargo.toml              # inherits workspace; deps: singularmem-core (path), tantivy, tracing
└── src/
    ├── lib.rs              # public API re-exports
    ├── index.rs            # `TantivyIndex` — owns the Tantivy index + writer
    ├── schema.rs           # Tantivy schema definition (fields, tokenizers)
    ├── query.rs            # query parsing + execution
    ├── hook.rs             # `impl IndexHook for TantivyIndex`
    ├── reindex.rs          # full reindex from Store iteration
    └── error.rs            # `Error` enum (thiserror)
```

**`singularmem-core` change** (small, additive): a new `pub mod hook`
defines the `IndexHook` trait. `Store` gains `open_with_hook` and
`set_hook` methods. Existing `Store::open` is unchanged.

```rust
// In crates/singularmem-core/src/hook.rs (new module)

/// Hook called by `Store::ingest` / `ingest_many` for each persisted `Item`,
/// and by the `reindex` flow for each iterated item. Implementations are
/// out-of-scope for this crate — see `singularmem-search` for the Tantivy
/// implementation.
///
/// Hook failures are surfaced as `tracing::warn!` but DO NOT roll back the
/// underlying SQLite write. Per Principle VII the user sees an honest
/// warning naming the item ID that is now un-searchable; `reindex` recovers.
pub trait IndexHook: Send + Sync {
    fn on_ingest(&self, item: &crate::Item) -> Result<()>;
    fn on_reindex(&self, item: &crate::Item) -> Result<()>;
    fn commit(&self) -> Result<()>;
}
```

**Workspace `Cargo.toml`** gains:

- `tantivy = "=0.22.0"` in `[workspace.dependencies]` (exact patch
  version for reproducibility; same convention as rusqlite in v0.1.0).

**Dependency boundary preserved:** `singularmem-core` has no Tantivy
dep. The `IndexHook` trait is pure Rust + `Item`. Sub-project 4 (MCP)
can wire up Tantivy via the same hook; sub-project 5 (TS SDK) can wire
up something else or nothing.

**Concurrency.** `TantivyIndex` wraps the `tantivy::IndexWriter` in a
`Mutex`, same pattern as `singularmem-core`'s SQLite connection. One
writer at a time; concurrent readers via `IndexReader::reload()` are
safe and lock-free. Sub-project 4's MCP server inherits this — its
requests serialize through the writer mutex on writes but read-fanout
is unbounded.

## Data model — Tantivy schema

```rust
use tantivy::schema::{Schema, SchemaBuilder, STORED, STRING, TEXT, INDEXED, FAST};

pub(crate) fn build_schema() -> (Schema, Fields) {
    let mut b = SchemaBuilder::new();

    // Searchable + stored — the primary search target.
    let content = b.add_text_field("content", TEXT | STORED);

    // Tokenized as raw strings (no stemming) so tag queries are exact.
    let tags = b.add_text_field("tags", STRING | STORED);

    // Tokenized for partial-match search.
    let source = b.add_text_field("source", TEXT | STORED);

    // Stored only (not searchable) — needed to reconstruct the Item from a hit.
    let id = b.add_text_field("id", STRING | STORED);

    // FAST + INDEXED so we can sort by created_at and filter by date ranges
    // (a v0.3 feature but the schema commits the column shape now).
    let created_at = b.add_date_field("created_at", INDEXED | STORED | FAST);

    // Stored-only pointer to the prior revision; useful for revision-aware
    // result filtering later.
    let supersedes = b.add_text_field("supersedes", STRING | STORED);

    let schema = b.build();
    (schema, Fields { content, tags, source, id, created_at, supersedes })
}
```

**Field decisions:**

- **`content` uses `TEXT`** (Tantivy's default English-tokenizing
  analyzer: lowercase, English stop-word filter, no stemming by default
  in 0.22). Suitable for v0.
- **`tags` uses `STRING`** (no tokenization). Tag queries are
  exact-match.
- **`source` uses `TEXT`** (tokenized). Source labels are often
  free-form; tokenizing lets users search for `source:conversation` or
  `source:abc-123` partially.
- **`metadata` is NOT in the schema.** Indexing arbitrary user JSON
  requires schema flattening or a JSON-blob field with no real query
  story; deferred to v0.3+ if asked for.
- **`created_at` is `FAST`** so range filtering by date is O(log n). v0
  search doesn't expose a date-range query, but committing the column
  shape now means later releases don't need to re-index.

## On-disk format

**Sidecar directory next to the SQLite file:**

```
~/.local/share/singularmem/
├── store.db                # SQLite (canonical store, format_version=1)
├── store.db-wal            # SQLite WAL
├── store.db-shm
└── store.db.tantivy/       # Tantivy segment directory (sub-project 2a adds)
    ├── meta.json           # Tantivy's own metadata
    ├── *.fast              # column-oriented per-segment files
    ├── *.term              # term dictionaries
    └── *.idx               # postings
```

Path convention: `<store_path>.tantivy/`. Configurable via
`StoreOptions.index_path: Option<PathBuf>` override.

**Format-version impact: none.** SQLite `format_version` stays at
`"1"`. The Tantivy sidecar is an opt-in performance optimization —
third-party loaders that only read SQLite continue to work. If the
Tantivy directory is missing, `Store::open_with_hook` rebuilds it from
a full SQLite iteration on first use (one-time cost; equivalent to
running `reindex` automatically). If it's present but at a stale
Tantivy index format, the hook rebuilds it. Principle III.b is
preserved.

**Format spec update:** `docs/formats/store-v1.md` gains a new section
titled "Tantivy sidecar index (optional, format unstable across
Tantivy versions)" documenting: (a) the sidecar's location convention,
(b) the schema's field names + types, (c) the rebuild-from-SQLite path.
The Tantivy on-disk format itself is documented upstream (link out,
don't duplicate).

## Write-path semantics

**Per-item ingest path** (single-item `Store::ingest`):

1. Validate the `NewItem` (existing path, unchanged).
2. SQLite transaction: insert `items` row + per-tag rows + commit.
3. If hook is set: call `hook.on_ingest(&item)` followed by
   `hook.commit()`. Both wrapped in `tracing::warn!` on error.
4. Return the persisted `Item`.

**Bulk ingest path** (`Store::ingest_many`):

1. Validate all items up front (existing path, unchanged).
2. Single SQLite transaction: insert every row + every tag + commit.
3. If hook is set: for each persisted item, call
   `hook.on_ingest(&item)`. Then call `hook.commit()` ONCE after the
   loop. Bulk ingest pays one Tantivy commit, not N.
4. Return the persisted `Vec<Item>`.

**Failure semantics — explicit and honest per Principle VII:**

- **SQLite write fails:** existing behaviour. Transaction rolls back,
  hook is never called, ingest returns `Error::Sqlite { context, source }`.
- **SQLite succeeds, hook fails:** the item is durably stored but
  un-searchable. The hook implementation logs a `tracing::warn!` with
  the item ID and the underlying error. `Store::ingest` still returns
  `Ok(item)` — the user's ingest did succeed at the storage layer. The
  honest recovery is `singularmem reindex`, which the warning message
  names.
- **SQLite succeeds, hook commit fails:** same as above. Tantivy
  discards uncommitted state cleanly on the next open.

**Why this asymmetry is honest, not silent-fallback:**

1. The user sees a `tracing::warn!` naming the failed item and pointing
   at `singularmem reindex`. That's the opposite of silent.
2. The storage layer's contract (the item is in the SQLite store) is
   honoured. Lying about that by failing the ingest would be the
   dishonest move.
3. `singularmem reindex` is one command away from full consistency.

The alternative — rolling back SQLite on Tantivy failure — would mean
a transient Tantivy hiccup loses a user's ingest, which is the worse
failure mode for a memory store.

**Supersedes handling:** items stay searchable. When a new item
supersedes an older one, both are indexed and searchable. A future
`--latest-only` query filter (deferred) will exclude items whose
`supersedes` field is pointed at by some other item's `id`.

## Interfaces

### Library (`singularmem-search` public surface)

```rust
// crates/singularmem-search/src/lib.rs re-exports:
pub use crate::index::{Index, IndexOptions};
pub use crate::query::{Query, QueryBuilder, SearchOptions};
pub use crate::result::{Hit, SearchResults};
pub use crate::error::{Error, Result};
```

**`Index` — the type implementing `IndexHook`:**

```rust
impl Index {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self>;
    pub fn open_with_options(dir: impl AsRef<Path>, options: IndexOptions) -> Result<Self>;
    pub fn search(&self, query: &Query, options: SearchOptions) -> Result<SearchResults>;
    pub fn reindex_from<F>(&self, items: impl Iterator<Item = Item>, on_progress: F) -> Result<u64>
    where F: FnMut(u64);
    pub fn doc_count(&self) -> Result<u64>;
}

impl singularmem_core::IndexHook for Index { /* on_ingest / on_reindex / commit */ }

pub struct IndexOptions {
    pub writer_memory_bytes: usize,  // Tantivy default 50 MB
}
```

**`Query` — built one of two ways:**

```rust
impl Query {
    /// Parse a Tantivy QueryParser-style string. Supports:
    /// - Bare terms:        `decision`
    /// - Required:          `+decision +urgent`
    /// - Excluded:          `decision -draft`
    /// - Field:value:       `tags:work`, `source:conversation`
    /// - Phrase:            `"deferred to v0.3"`
    /// - Boolean:           `(decision OR fix) AND -draft`
    ///
    /// Default search fields: `content`, `source` (bare terms match either).
    /// `tags` is queryable only via the explicit `tags:` prefix.
    pub fn parse(query_str: &str) -> Result<Self>;
}

impl QueryBuilder {
    pub fn new() -> Self;
    pub fn term(self, field: Field, value: impl Into<String>) -> Self;
    pub fn must(self, term: Query) -> Self;
    pub fn must_not(self, term: Query) -> Self;
    pub fn should(self, term: Query) -> Self;
    pub fn build(self) -> Query;
}

#[derive(Copy, Clone, Debug)]
pub enum Field { Content, Tags, Source }
```

**`SearchOptions` + `Hit` + `SearchResults`:**

```rust
pub struct SearchOptions {
    pub limit: usize,          // default 20
    pub offset: usize,         // default 0
    pub include_snippets: bool,  // default true
}

pub struct SearchResults {
    pub hits: Vec<Hit>,
    pub total_matched: u64,
    pub elapsed: std::time::Duration,
}

pub struct Hit {
    pub id: ItemId,
    pub score: f32,
    pub snippet: Option<String>,  // ~160 chars centered on highest-scoring match
}
```

`Hit` returns `id + score + snippet` only — the caller has a `Store`
handle and can call `store.get(hit.id)` for the full payload. Returning
the full `Item` from `Index::search` would duplicate stored data and
couple the search crate to the store, which we don't want.

### CLI

```
singularmem [OPTIONS] <COMMAND>

OPTIONS (existing + one new):
  --store <PATH>     Default: $XDG_DATA_HOME/singularmem/store.db
  --read-only        Open the store in read-only mode (refuses ingest)
  --no-index         Skip wiring up the Tantivy hook on open.

SUBCOMMANDS (existing + two new):
  ingest      Add a new item to the store
  get         Fetch one item by ID
  list        Enumerate items, optionally filtered by tag
  revisions   Show the supersedes chain for an item
  export      Emit the entire store as JSONL on stdout
  search      Full-text search over the store        [NEW]
  reindex     Rebuild the Tantivy index from SQLite  [NEW]
```

**`singularmem search`:**

```
singularmem search <QUERY>...
    [--limit N]         max hits, default 20
    [--offset N]        skip first N, default 0
    [--no-snippets]     suppress snippet generation (faster)
    [--format <FMT>]    table | jsonl | ids  (default table)

QUERY is a Tantivy QueryParser string. Multiple QUERY tokens become an
implicit AND-join. Quote terms to pass them as a single argument.

Exit codes:
  0    Success (results found OR no results matched)
  1    Usage / query parse error
  2    Store opens fine but Tantivy index is missing — run reindex
  3    Store corruption / unsupported format
```

**`singularmem reindex`:**

```
singularmem reindex
    [--store PATH]
    [--progress | --quiet]

Streams every item from the SQLite store into a freshly-rebuilt Tantivy
index. Safe to run while other singularmem processes are reading the
store (concurrent readers see OLD results until the new commit lands).
Concurrent writers block on the writer mutex.

Prints "reindex: N items processed" every 1000 items by default.

Exit codes: 0 success, 1 SQLite store missing/unopenable, 3 Tantivy
directory unwritable.
```

**Auto-wiring** in the root binary: `Store::open_with_options` is
wrapped to also `Index::open(<store_path>.tantivy/)` and set the hook
unless `--no-index` is passed. Index-open failure is non-fatal for
non-search commands — a `tracing::warn!` fires and `Store::set_hook`
is left at `None`.

### Wire (MCP, HTTP, etc.)

None in this sub-project. Sub-project 4 introduces the MCP server,
which will consume the search library through its public API.

## Error handling

The search crate's `Error` enum carries the three Principle VII pieces
(what failed, what was attempted, what state was preserved).

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Tantivy error during {context}: {source}")]
    Tantivy {
        context: &'static str,
        #[source]
        source: tantivy::TantivyError,
    },

    #[error("could not parse search query: {0}")]
    QueryParse(String),

    #[error("Tantivy index at {path} is missing or unreadable; run `singularmem reindex` to rebuild")]
    IndexMissing { path: std::path::PathBuf },

    #[error("Tantivy index at {path} appears corrupted: {reason}; run `singularmem reindex`")]
    IndexCorrupted { path: std::path::PathBuf, reason: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

The CLI maps these to the stable exit codes in the Interfaces section.
The `IndexMissing` variant's message explicitly names the recovery
command — never just "index missing" with no path forward.

## Testing strategy

Test layout for `crates/singularmem-search/`:

```
crates/singularmem-search/
├── src/
│   └── *.rs                # `#[cfg(test)] mod tests` — pure-function tests
├── tests/
│   ├── ingest_to_search.rs # Open Store+Index; ingest items; assert search returns them
│   ├── reindex.rs          # Empty index → reindex from store → search works
│   ├── query_parser.rs     # Tantivy query parsing edge cases
│   ├── concurrency.rs      # Search readers during a long-running reindex
│   └── failure_modes.rs    # Hook errors logged; SQLite write still succeeds
└── benches/
    └── search_perf.rs      # criterion: query latency p95, reindex throughput
```

**Plus a new integration test in `crates/singularmem-core/`:**

`crates/singularmem-core/tests/hook.rs` — verifies:

- `Store::open` (without a hook) works unchanged from v0.1.0.
- A hook that always returns `Err` does NOT cause `Store::ingest` to fail.
- The Item is still in the store after a hook error.

This is the Principle VII compliance test for the asymmetric failure
semantics. The asymmetric write contract is constitutional, not just
convention.

**Principle III.b preservation:**

The existing
`crates/singularmem-core/tests/format.rs::open_core_only_round_trip`
test continues to pass with the hook plumbing in place. It imports
ONLY from `singularmem-core` + stdlib + `tempfile`. A user with only
the core crate can still ingest → list → export → re-import.

A NEW test in `singularmem-search/tests/ingest_to_search.rs` covers
the search-augmented round-trip: ingest with hook attached → search
finds items → reindex from scratch → search still works.

**Property tests for the query parser:**

- For any random valid query string built from `QueryBuilder`,
  serializing to a string and re-parsing via `Query::parse` produces an
  equivalent query.
- For any item ingested with content X, searching for any individual
  word in X returns that item.
- Hook failure (simulated) is observable as a `tracing::warn!` AND the
  Item is still in the store via the round-trip test.

**Concurrency tests:**

- 16 reader threads doing `Index::search` calls concurrent with a
  long-running reindex: readers always see consistent results (either
  all OLD or all NEW, never mixed).
- One writer (live ingest) blocked during reindex; resumes immediately
  after reindex commit; new ingests land on the freshly-rebuilt index.

## Performance budgets — measured and enforced

Search 2a is the first sub-project where every Principle X budget gets
genuinely exercised. Pre-flight estimates and risk assessment:

| Budget | v0.1.0 measured | v0.2.0 estimate | Risk |
|---|---|---|---|
| Query p95 | N/A (no search) | < 10 ms (BM25 at ~10K docs is fast) | Low |
| Ingest throughput | ~19,800 items/s | ~500–2,000 items/s | **Medium** — Tantivy single-item commit-per-write could drop us below 50 |
| CLI cold start | ~10 ms | ~40–80 ms (Tantivy init opens segments) | Low |
| Binary size | ~12 MB | ~25–35 MB (Tantivy + tokenizer data) | Low |

The ingest-throughput risk is the real concern. `Store::ingest_many`
already batches the Tantivy commit (one commit per bulk call), so bulk
ingest paths shouldn't regress. If single-item ingest with Tantivy
commit-per-write drops below 50/s on the reference runner, the
implementation plan tasks include explicit measurement-then-tighten
steps and three mitigations:

1. Defer per-item Tantivy commits to a background flush (adds
   threading; punts to v0.3).
2. Document the regression in the spec; tighten the budget after
   measuring.
3. Promote `ingest_many` as the canonical bulk path in docs and let
   single-item ingest pay the cost.

The plan tasks measure BEFORE locking the spec — if a number violates
Principle X, the design needs to flex.

**Updated CI workflow:**

The seven blocking jobs from v0.1.0 (fmt, clippy, check, build, test,
audit, dco) automatically cover the new crate via `--workspace`.

- `perf-budgets` is **re-promoted to blocking** in this sub-project,
  after hardening the criterion-output parser (switch to
  `--output-format=json`).
- `tests-offline` stays advisory — separate "CI infrastructure"
  sub-project will harden the namespace-availability story later.

## Open questions

The implementation plan must resolve, but they are operational rather
than design:

1. **Tantivy version pinning.** `tantivy = "=0.22.X"` for some exact
   patch version current at plan-write time. Same pattern as rusqlite.
2. **Exact perf measurements.** The plan tasks run benches BEFORE
   committing the spec values; if the actual ingest throughput is below
   50/s, the design needs to choose among the three mitigations above
   before merging.
3. **Snippet format default.** `<mark>...</mark>` is the obvious
   default; a future CLI `--format=jsonl-plain` could strip the tags.
   Punt for v0.

## Acceptance criteria

Search 2a is done when *all* of these are observable on `main`:

1. **`crates/singularmem-search/`** exists as a workspace member with
   the modules from "Architecture"; `cargo doc -p singularmem-search`
   builds and every `pub` item has a doc comment.
2. **`singularmem-core::IndexHook` trait** is `pub`, with the three
   methods from "Architecture". `Store::open_with_hook` and
   `Store::set_hook` work end-to-end; existing `Store::open` is
   unchanged (verified by a `Cargo.lock`-level absence of Tantivy in
   `singularmem-core`'s dep tree).
3. **`docs/formats/store-v1.md`** gains a "Tantivy sidecar index
   (optional)" section per "On-disk format".
4. **Live ingest writes to Tantivy** per "Write-path semantics":
   ingest an item → search finds it within the same process; ingest
   with a deliberately-failing hook → SQLite has the item, stderr has
   the warning, Tantivy doesn't have it, `singularmem reindex`
   recovers.
5. **`singularmem search`** end-to-end:
   - `singularmem ingest --content "decision: use SQLite"` then
     `singularmem search decision` returns the item.
   - `singularmem search 'tags:work +urgent'` works (Tantivy
     QueryParser syntax).
   - `singularmem search 'no-such-term' --format=jsonl` exits 0 with
     no stdout output and a stderr "0 matches" tracing line.
   - Malformed query → exits 1 with stderr parse error.
   - Index missing → exits 2 with "run `singularmem reindex`" message.
6. **`singularmem reindex`** end-to-end:
   - On a store with no Tantivy directory: builds the index from
     scratch, exits 0.
   - On a store with a stale Tantivy directory: full rebuild, exits 0.
   - During a long reindex, concurrent `singularmem search` (separate
     process) returns OLD results until the reindex commits, then new
     results — no error, no partial state visible.
7. **Hook-failure asymmetry verified** by
   `crates/singularmem-core/tests/hook.rs`: a `IndexHook` that always
   returns `Err` does NOT cause `Store::ingest` to fail; the item is
   persisted; `tracing::warn!` fires with the item ID.
8. **Principle III.b round-trip preserved**:
   `crates/singularmem-core/tests/format.rs::open_core_only_round_trip`
   continues to pass, imports unchanged.
9. **All four Principle X budgets satisfied on `ubuntu-latest`** with
   measured numbers in the PR description:
   - Query latency p95 < 100 ms (expected ~10 ms)
   - Ingest throughput ≥ 50 items/s (single-item) — if violated,
     document the chosen mitigation
   - CLI cold start < 200 ms (expected ~40–80 ms)
   - Binary size < 150 MB (expected ~25–35 MB)
10. **`perf-budgets` CI job re-promoted to blocking** — the criterion
    parser hardened (switch to `--output-format=json`).
    `continue-on-error: true` removed from this job.
11. **`tests-offline` CI job stays advisory** — explicit non-goal for
    this sub-project.
12. **Version bump to `0.2.0`** in workspace `Cargo.toml`.
    `singularmem --version` prints `singularmem 0.2.0`. Tag `v0.2.0`
    pushed after merge.
13. **No `[PLACEHOLDER]` strings** in any committed file under
    `docs/formats/`, `crates/singularmem-search/src/**/*.rs`, or in
    new public API doc comments.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | Tantivy is a pure-Rust crate operating entirely on the local filesystem. No network calls in the search path. The sidecar `*.tantivy/` directory is local data alongside the SQLite store. |
| **II — Provider-Agnostic by Contract** | No LLM provider integration in this sub-project. Search is keyword/lexical only. Embeddings + provider-side semantic search are sub-projects 2b+ and 3. |
| **III — Open Core with a Stable Boundary** | Wholly open. The sidecar format is documented in `docs/formats/store-v1.md`. III.b is preserved by the unchanged `open_core_only_round_trip` test. III.c remains satisfied — `singularmem export` is unchanged. |
| **IV — CLI-First, GUI-Visible** | Two new CLI verbs (`search`, `reindex`) expose every new library capability. No GUI in this sub-project. |
| **V — Composable Library Architecture** | `singularmem-search` is a standalone library with documented public API and its own test suite. `singularmem-core` does NOT depend on Tantivy — the `IndexHook` trait is pure Rust. Sub-project 4's MCP server, sub-project 5's TS SDK, and the eventual proprietary GUI all consume the search library through the same public API. |
| **VI — Deterministic and Offline-Testable** | Tantivy is deterministic given fixed inputs. All tests use `tempfile` and avoid system clocks. The `tests-offline` advisory job continues to attempt the namespace check; the test suite remains network-free by review. |
| **VII — Honest Failure Modes** | The asymmetric write semantics (SQLite succeeds, hook fails → item stored but un-searchable, warning emitted) are explicit and tested. No silent fallback — every failure path names the operation, the cause, and the recovery (`singularmem reindex`). Empty search results print "0 matches" to stderr rather than ambiguously empty stdout. |
| **VIII — Privacy Telemetry Boundary** | No telemetry added. Search queries and results stay local. |
| **IX — Accessible by Default (WCAG 2.2 AA)** | CLI-only surface. clap output respects `NO_COLOR`. Snippet `<mark>...</mark>` tags are inline text — no animations. Screen readers handle the table/jsonl output as expected. |
| **X — Performance Budgets, Enforced in CI** | This is the sub-project where Principle X budgets become genuinely meaningful. All four budgets are exercised by real code; the `perf-budgets` CI job is re-promoted to blocking after the parser is hardened. The plan tasks include measurement-then-tighten steps for each budget. |
