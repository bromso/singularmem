---
title: Singularmem — Memory Store v0
date: 2026-05-16
status: draft
sub-project: 1-memory-store-v0
supersedes: none
---

# Singularmem — Memory Store v0

This is sub-project **1** of Singularmem, the first sub-project to ship
domain functionality. It introduces the persistence layer the rest of the
system will hang off: an immutable, supersedes-revisioned, locally-stored,
SQLite-backed store of UTF-8 text memory items, with a documented on-disk
format and a thin CLI shell over a Rust library.

It does **not** ship indexing, embeddings, LLM provider integration,
retrieval, MCP, the TypeScript SDK, or any proprietary code. Each of those
is its own sub-project.

## Problem & motivation

Sub-project 0 (bootstrap) ratified the constitution and stood up the repo
skeleton, but it shipped no actual storage code — the do-nothing CLI prints
its version and exits. Until a memory store exists, the rest of the
ecosystem (search, retrieval, provider adapters, MCP server, SDK bindings,
GUI) has nothing to operate on. Memory Store v0 is the foundation that
unblocks every later sub-project.

Two constitutional commitments come due in this sub-project that bootstrap
deferred:

- **Principle III.b open-side viability** — a user must be able to ingest,
  query, and export their entire memory using only open components. v0 is
  where that becomes a runnable test.
- **Principle X performance budgets in CI** — the constitution defines
  four numeric budgets (query p95 < 100 ms, ingest ≥ 50 items/s, CLI cold
  start < 200 ms, binary < 150 MB) and requires CI enforcement. Bootstrap
  had no code to measure; v0 closes that gap.

## Goals & non-goals

### Goals

1. Ship `crates/singularmem-core` — a standalone Rust library whose public
   API covers ingest, point-read, list, supersedes-aware revision walks,
   and JSONL export.
2. Define and document the on-disk format (`docs/formats/store-v1.md`)
   with sufficient detail that a third party can write a loader without
   reading our source code.
3. Thicken the root-level `singularmem` binary into a thin CLI shell over
   `singularmem-core`, exposing five new subcommands: `ingest`, `get`,
   `list`, `revisions`, `export`.
4. Land the first Principle X enforcement (`perf-budgets` CI job)
   measuring all four budgets on `ubuntu-latest`.
5. Land the Principle VI offline guarantee as a real test
   (`tests-offline` CI job in a `--network=none` container).
6. Bump the workspace version to `0.1.0` and tag `v0.1.0` after merge.

### Non-goals

- Indexes (lexical Tantivy + vector) — sub-project 2.
- Embedding generation — sub-project 2.
- LLM provider adapters and retrieval — sub-project 3.
- MCP server — sub-project 4.
- TypeScript SDK binding — sub-project 5.
- A CLI `import` verb that re-loads a JSONL export — deferred to v0.2
  alongside cycle detection (which `import` enables).
- Blob/file types beyond UTF-8 text — deferred to v0.3 or later; v0 caps
  content at 1 MiB.
- Multi-process write contention — single-writer-per-process via SQLite
  WAL is acceptable for v0; sub-project 2's search work may revisit if
  contention arises.
- The proprietary Flutter GUI and any visualisation work.

## Recommended approach

**Approach A — Single `singularmem-core` crate + thick root binary.**
All domain logic lives in `crates/singularmem-core`. The root-level
`src/main.rs` becomes a clap-based CLI that imports the core crate and
orchestrates: parse args → call core methods → format output. Sync API
(no tokio runtime; SQLite is sync-friendly). Concrete `SqliteStore` (no
`Storage` trait abstraction yet — premature). This matches the bootstrap
design's "thin shell over `crates/singularmem-core`" pattern exactly.

### Approaches discarded

- **Approach B — Library owns the CLI struct too.** Same crate count, but
  the clap `Cli` struct lives inside `singularmem-core` and the binary is
  one line. Slightly more testable (CLI logic gets unit tests in the
  lib's test suite). Rejected: a library that exposes both domain types
  AND a CLI struct is doing two things; concern-clarity wins.
- **Approach C — Multi-crate from day one (`singularmem-core` +
  `singularmem-cli` + thin root binary).** Aligns hardest with Principle
  V's "thin shells over libraries". Rejected: premature for v0. There is
  no second consumer of CLI logic. Sub-project 4's MCP server is a
  separate crate; sub-project 5's napi-rs binding is a separate crate.
  The CLI doesn't need its own crate until there is a real reason.

## Architecture

The workspace gains one new member: `crates/singularmem-core`.

```
crates/singularmem-core/
├── Cargo.toml              # inherits workspace edition/version/license/lints
└── src/
    ├── lib.rs              # public API surface (re-exports)
    ├── store.rs            # `Store` type — open/close, transactions
    ├── item.rs             # `Item`, `ItemId`, `NewItem` types
    ├── ingest.rs           # `Store::ingest()` — insert + supersedes resolution
    ├── query.rs            # `get`, `list`, iteration
    ├── export.rs           # `export()` — emit JSONL of all items
    ├── error.rs            # `Error` enum (thiserror)
    ├── schema.rs           # SQL DDL + migration runner
    └── format.rs           # on-disk format version constants + introspection
```

The root-level `singularmem` binary keeps its existing `src/main.rs`
location but grows from three lines to roughly eighty: a clap `Cli`
struct, subcommand match, calls into `singularmem_core::Store`. Same
`[lints] workspace = true`. The root `Cargo.toml` adds `clap` 4 (with
`derive`) and a path dependency on `singularmem-core`.

**Library dependencies** — added to `[workspace.dependencies]` so future
crates inherit consistent versions:

- `rusqlite` 0.32, feature `bundled` — embedded SQLite with JSON1; no
  system dependency.
- `ulid` 1 — ULID generation; 26-char Crockford base32; time-sortable.
- `serde` 1 + `serde_json` 1 — for the JSON `metadata` column and JSONL
  export.
- `thiserror` 2 — error enum derivation.
- `jiff` 0.1 — modern Rust time crate; deterministic RFC3339-nanos
  serialisation. Chosen over `chrono` for new-project hygiene.
- `tracing` 0.1 — structured logging crate (events only, no subscriber).
  The library emits `tracing::warn!` for soft-warning conditions
  (oversized metadata, etc.) and `tracing::debug!` for transaction
  boundaries; SDK consumers attach their own subscriber. The CLI
  attaches `tracing-subscriber` 0.3 in the root binary so warnings
  reach stderr.

**Dev dependencies** — `tempfile` (isolated SQLite files for tests),
`assert_cmd` + `predicates` (CLI integration tests), `criterion` 0.5
(perf benches), `proptest` 1 (property tests).

**Concurrency model.** A `Store` is `Send + Sync`. Internally the
`rusqlite::Connection` is wrapped in a `Mutex` — single-writer,
multi-reader semantics under WAL would let us hold a connection pool, but
for v0 a single guarded connection is simpler and the perf budget does
not require pooling. Sub-project 2 may revisit when actual contention
exists.

## Data model

```rust
/// Stable, opaque identifier for a memory item.
/// Implemented as a ULID (26-char base32) — time-sortable, URL-safe,
/// fits in a TEXT column.
pub struct ItemId(Ulid);

/// A persisted memory item. Immutable once stored.
pub struct Item {
    pub id: ItemId,
    pub content: String,                // UTF-8, non-empty, ≤ 1 MiB
    pub created_at: jiff::Timestamp,    // wall-clock at ingest
    pub supersedes: Option<ItemId>,     // pointer to prior item this corrects
    pub tags: Vec<String>,              // 0..N tags; deduped on ingest
    pub source: Option<String>,         // free-form provenance label
    pub metadata: serde_json::Value,    // arbitrary user-defined; MUST be JSON object
}

/// The "to be ingested" form. ID and created_at are assigned by `Store::ingest`.
pub struct NewItem {
    pub content: String,
    pub supersedes: Option<ItemId>,
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub metadata: serde_json::Value,
}
```

**Validation rules**, enforced inside `Store::ingest` and surfaced as
`Error::Validation` per Principle VII:

- `content` — non-empty UTF-8. Max length 1 MiB (1,048,576 bytes). Items
  larger than that go in v0.3 or via a future "blob" type.
- `tags` — each tag is non-empty, ≤ 64 bytes, contains no `\0`.
  Duplicate tags within one item are deduped silently.
- `source` — when present, ≤ 256 bytes.
- `metadata` — must be a JSON object (`{}`-shaped); arrays and scalars
  are rejected. Empty object is the default. No hard cap inside v0;
  the lib emits a `tracing::warn!` log line above 64 KiB so SDK
  consumers and the CLI both see it, but the ingest still succeeds.
- `supersedes` — if present, the referenced item MUST exist in the
  store. Ingest fails with `Error::SupersedesNotFound` otherwise.
  Cycles are not constructible in v0 because IDs are assigned by the
  store, not by the caller; cycle detection is a v0.2+ concern that
  arrives with `import`.

**Why these specific shapes:**

- `Item` is `pub` with `pub` fields rather than getter methods — it is a
  data record, not a behaviour-bearing type. Future v0.x can switch to
  private fields + accessors without breaking the JSONL export format.
- `NewItem` is separate from `Item` so the type system enforces "you
  cannot ingest an item with a pre-assigned ID."
- `serde_json::Value` for `metadata` is the boring right tradeoff: SDK
  consumers can pass any JSON without us shipping a typed schema; we
  still validate at the boundary that it is an object.
- `jiff::Timestamp` over `SystemTime`: jiff serialises deterministically
  as RFC 3339 nanos; `SystemTime` is platform-dependent. Determinism is
  a Principle VI requirement.

## On-disk format

A single SQLite 3 file, default name `store.db`, opened in WAL mode. The
schema is documented in a versioned `docs/formats/store-v1.md` spec that
ships in the repo — that is the artefact a third-party tool reads.

### Schema

```sql
CREATE TABLE singularmem_meta (
    key    TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
) STRICT;
-- Required keys: 'format_version' (currently '1'), 'created_at' (RFC3339).

CREATE TABLE items (
    id          TEXT PRIMARY KEY NOT NULL,        -- 26-char ULID
    content     TEXT NOT NULL,                    -- UTF-8, non-empty
    created_at  TEXT NOT NULL,                    -- RFC3339 nanos
    supersedes  TEXT,                             -- nullable; FK to items.id
    source      TEXT,                             -- nullable
    metadata    TEXT NOT NULL DEFAULT '{}',       -- JSON object as text
    FOREIGN KEY (supersedes) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK (length(content) > 0),
    CHECK (length(content) <= 1048576),
    CHECK (json_valid(metadata) AND json_type(metadata) = 'object')
) STRICT;

CREATE TABLE item_tags (
    item_id  TEXT NOT NULL,
    tag      TEXT NOT NULL,
    PRIMARY KEY (item_id, tag),
    FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_items_created_at ON items(created_at);
CREATE INDEX idx_items_supersedes ON items(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_item_tags_tag ON item_tags(tag);
```

**Notes:**

- `STRICT` tables enforce column types — part of the third-party
  readability story.
- Tags live in a separate junction table rather than a JSON array so
  "items with tag X" is an O(log n) index lookup, not a JSON scan. The
  JSON `metadata` column carries arbitrary user fields; tags are
  first-class and queryable.
- `supersedes` FK is `DEFERRABLE INITIALLY DEFERRED` so a single
  transaction can insert several items that supersede each other in any
  order (relevant for batch ingest in later sub-projects).
- WAL mode adds two sidecar files (`store.db-wal`, `store.db-shm`) —
  recreated on next open and not required for backup. The format spec
  calls them out.

### Migration strategy

The `singularmem_meta.format_version` row is the truth. On
`Store::open`, the lib reads this value:

- Missing → fresh store; create schema + write `format_version = '1'`.
- `'1'` → no migration; ready.
- Anything else → `Error::UnsupportedFormatVersion { found, max_supported: 1 }`.

Future v0.2 ships a migrator from `'1'` → `'2'`. Migrators run in a
transaction; failures roll back to `'1'`.

### JSONL export format (export-v1)

```jsonl
{"_singularmem_format":"export-v1","_kind":"meta","store_format_version":"1","exported_at":"2026-05-16T..."}
{"_kind":"item","id":"01J...","content":"...","created_at":"2026-05-16T...","supersedes":null,"source":null,"tags":["work","decision"],"metadata":{"project":"alpha"}}
{"_kind":"item","id":"01J...","content":"...","created_at":"...","supersedes":"01J...","source":"claude-conversation:abc","tags":[],"metadata":{}}
```

First line is always a meta record naming the format. Each subsequent
line is one item (UTF-8 JSON, no trailing comma, terminated by `\n`).
The format is round-trippable. Items are emitted in `created_at`
ascending order so the export is deterministic given a deterministic
store.

### Format spec location

`docs/formats/store-v1.md` (new directory) contains the full DDL, the
`singularmem_meta` key registry, the JSONL export schema, the migration
ratchet rules, and a "writing a third-party loader" walkthrough. This
file is the constitutional artefact required by Principle III's
third-party readability mandate.

## ID + revision system

**ULID generation.** Production uses `ulid::Ulid::new()` which combines
wall-clock time and OS RNG. For Principle VI, the lib injects both via a
`Clock` and `Rng` trait pair, defaulting to system implementations:

```rust
pub trait Clock: Send + Sync {
    fn now(&self) -> jiff::Timestamp;
}

pub trait Rng: Send + Sync {
    fn fill_bytes(&mut self, dst: &mut [u8]);
}
```

`Store::open(path)` uses `SystemClock` + `OsRng`. `Store::open_with(path,
clock, rng)` lets tests inject deterministic implementations. The
default `SystemClock` and `OsRng` ship in the lib for callers who do
not care.

**ULID details.**

- 26-char Crockford base32; case-insensitive on parse, uppercase on
  emit. `ItemId` parsing accepts either case; `Display` produces
  uppercase.
- Time component is millisecond precision (per ULID spec). Two ULIDs
  generated in the same millisecond are ordered by their random
  component.
- Within a single millisecond, ingests are still totally ordered because
  the `created_at` column is RFC3339 nanos and the `idx_items_created_at`
  index uses string ordering — correct for ISO-8601-ish RFC3339
  representations.

**Revision navigation API:**

```rust
impl Store {
    /// Walk the supersedes chain from a starting item back to the original.
    /// Items returned newest-first; the starting item is included.
    /// Errors: NotFound.
    pub fn revision_history(&self, id: ItemId) -> Result<Vec<Item>>;

    /// Find the latest revision — the item that nothing supersedes,
    /// reachable from the given starting ID by walking forward.
    /// Errors: NotFound, AmbiguousLatest { candidates: Vec<ItemId> }.
    pub fn latest_revision(&self, id: ItemId) -> Result<Item>;
}
```

**"Latest revision" semantics.** An item is the "latest" iff no other
item has it in their `supersedes` field. Multiple items can supersede
the same item (forking is allowed); in that case `latest_revision`
returns `Error::AmbiguousLatest` rather than guess. v0 does not promise
to merge forks; sub-project 1.5+ may add explicit fork-resolution
semantics.

**Cycle handling.** A `supersedes` cycle is theoretically possible
(item A supersedes B, B supersedes A) only if the user constructs it
across two separate ingests with explicit IDs — which cannot happen in
v0 because IDs are assigned by the store, not by the caller. The FK +
the "you cannot reference an ID that does not exist yet" property gives
us cycle-freedom for free in v0. Sub-project 1.5+ that introduces
`import` (with explicit IDs) re-verifies this assumption.

**Why ULID over UUIDv7:** ULID is 26 chars vs UUIDv7's 36; both are
time-sortable. ULID's Crockford base32 is human-readable (no `0/O` or
`1/I/L` confusion). For a store where IDs appear in `singularmem get
<id>` commands and JSONL exports, the shorter, friendlier form wins.

## Interfaces

### Library (`singularmem-core` public surface)

`crates/singularmem-core/src/lib.rs` re-exports a tight surface; nothing
else escapes.

```rust
pub use crate::store::{Store, StoreOptions};
pub use crate::item::{Item, ItemId, NewItem};
pub use crate::error::{Error, Result};
pub use crate::format::FORMAT_VERSION;          // const &str = "1"
pub use crate::clock::{Clock, SystemClock};
pub use crate::rng::{Rng, OsRng};
```

The `Store` API:

```rust
impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;
    pub fn open_with(
        path: impl AsRef<Path>,
        clock: Box<dyn Clock>,
        rng: Box<dyn Rng>,
    ) -> Result<Self>;
    pub fn open_with_options(path: impl AsRef<Path>, options: StoreOptions) -> Result<Self>;

    pub fn ingest(&self, item: NewItem) -> Result<Item>;
    pub fn ingest_many<I: IntoIterator<Item = NewItem>>(&self, items: I) -> Result<Vec<Item>>;

    pub fn get(&self, id: ItemId) -> Result<Item>;
    pub fn get_optional(&self, id: ItemId) -> Result<Option<Item>>;

    pub fn list(&self) -> Result<ItemIter<'_>>;
    pub fn list_by_tags(&self, tags: &[&str]) -> Result<ItemIter<'_>>;

    pub fn revision_history(&self, id: ItemId) -> Result<Vec<Item>>;
    pub fn latest_revision(&self, id: ItemId) -> Result<Item>;

    pub fn export(&self, w: &mut dyn Write) -> Result<()>;
    pub fn format_version(&self) -> Result<&str>;
}

pub struct StoreOptions {
    pub read_only: bool,
}

pub struct ItemIter<'store> { /* opaque; impl Iterator<Item = Result<Item>> */ }
```

`ItemIter` is streaming — memory cost is O(1) per item, not O(n). The
100K-item budget plus the 1 MiB content cap means a worst-case
`list().collect()` would allocate up to 100 GB. Streaming is the
default; CLI `list` and `export` commands use it throughout.

### CLI surface

```
singularmem [OPTIONS] <COMMAND>

OPTIONS:
  --store <PATH>     Default: $XDG_DATA_HOME/singularmem/store.db
  --read-only        Open the store in read-only mode (refuses ingest)
  -h, --help
  -V, --version

SUBCOMMANDS:
  ingest      Add a new item to the store
  get         Fetch one item by ID
  list        Enumerate items, optionally filtered by tag
  revisions   Show the supersedes chain for an item
  export      Emit the entire store as JSONL on stdout
  help        Print help (clap's auto-generated subcommand)
```

Exactly one of `--content TEXT`, `--file PATH`, `--stdin` must be
present for `ingest`. Tags repeat (`--tag a --tag b`). All
read subcommands accept `--format` flags; defaults are the
human-readable shape (`text` for `get`, `table` for `list` /
`revisions`); `--format jsonl` produces machine-parseable output.

**Common output conventions:**

- `text` formats write to stdout. Errors and progress go to stderr.
- `json` / `jsonl` formats are stable: parsing them is part of the
  public contract (any field rename is a SemVer-MAJOR change to v0).
  The schema matches the `export-v1` item shape.
- Exit codes: `0` success, `1` validation/usage error, `2` not-found,
  `3` store-corruption / unsupported-format-version, `64+` reserved.

**Default store path discovery** uses the `dirs` crate
(`dirs::data_dir()`); the parent directory is created on first ingest
if missing. The `--store PATH` flag overrides for project-local stores.

### Wire (MCP, HTTP, etc.)

None in v0. Sub-project 4 introduces the MCP server.

## Error handling

The `Error` enum carries the three pieces Principle VII requires (what
operation failed, what was attempted, what state was preserved or rolled
back).

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("validation failed for {field}: {reason}")]
    Validation { field: &'static str, reason: String },

    #[error("supersedes target {id} not found in store; new item was not persisted")]
    SupersedesNotFound { id: ItemId },

    #[error("item {id} not found")]
    NotFound { id: ItemId },

    #[error("ambiguous latest revision: {} candidates", candidates.len())]
    AmbiguousLatest { candidates: Vec<ItemId> },

    #[error("store format version {found} is newer than supported maximum {max_supported}")]
    UnsupportedFormatVersion { found: String, max_supported: &'static str },

    #[error("store is opened read-only; the {operation} operation requires write access")]
    ReadOnly { operation: &'static str },

    #[error("invalid ULID: {0}")]
    InvalidId(#[from] ulid::DecodeError),

    #[error("SQLite error during {context}: {source}; rolled back")]
    Sqlite { context: &'static str, #[source] source: rusqlite::Error },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
```

Each variant names what failed and what state was preserved. The CLI
maps these to stable exit codes (Section 6) and writes the message to
stderr.

## Testing strategy

Test layout:

```
crates/singularmem-core/
├── src/
│   └── *.rs                # `#[cfg(test)] mod tests` — pure-function unit tests
├── tests/
│   ├── store_basics.rs     # Store::open, ingest, get, list — happy paths
│   ├── revisions.rs        # supersedes chains, latest_revision, AmbiguousLatest forks
│   ├── validation.rs       # every Error::Validation branch has a triggering input
│   ├── format.rs           # round-trip ingest → export → re-load → identical Items
│   ├── concurrency.rs      # parallel readers + single writer don't interfere
│   └── property.rs         # proptest-driven round-trip + invariant tests
└── benches/
    └── store_perf.rs       # criterion benches feeding the CI budget gate

tests/                      # root-crate CLI integration tests (existing harness)
└── cli.rs                  # singularmem --version, ingest/get/list/export verbs
```

**The Principle III.b end-to-end test** (`format.rs::open_core_only_round_trip`):
opens a fresh store, ingests N items (some with supersedes), `export()`s
to a temp file, reads the temp file back as JSONL, asserts every item
round-trips. This test depends ONLY on `singularmem-core` plus stdlib +
`tempfile`. If a future sub-project introduces a hidden dependency on a
proprietary component for any of {ingest, get, list, export,
revision-walk}, this test fails to compile.

**Principle VI verification:** a dedicated CI job `tests-offline` runs
in a Docker container with `--network=none`. The job runs `cargo test
--all-targets --workspace`. If any test attempts a network call, it
fails (the kernel rejects the syscall). Strongest possible "tests pass
with networking disabled" check.

**Property tests** (`property.rs` using `proptest`):

- Round-trip: for any `NewItem` that passes validation,
  `ingest(item).then(|i| get(i.id))` returns the same logical content.
- Tag idempotence: `ingest({tags: ["a", "a", "b"]})` round-trips as
  `{tags: ["a", "b"]}` (deduped).
- Determinism with injected clock+rng: ingesting the same `NewItem`
  twice with identical injected state produces identical `ItemId`s.

**Concurrency tests** (`concurrency.rs`):

- One writer + 16 reader threads for 1000 ingests; readers see
  consistent snapshots (no torn reads).
- Two writers from two `Store` handles to the same file: one wins, the
  other gets `Error::Sqlite{..}` cleanly (no panic, no corruption).

## Performance budgets in CI

A new CI job `perf-budgets` on `ubuntu-latest` enforces all four
budgets from Principle X. Median-of-N (N=5 for cold start; criterion's
built-in bootstrap for benches) absorbs typical runner variance.

```bash
# .github/scripts/perf-check.sh — runs in CI; exits non-zero on regression

cargo build --release --bin singularmem
BIN=./target/release/singularmem

# 1. Binary size budget: < 150 MB
SIZE_BYTES=$(stat -c %s "$BIN")
test "$SIZE_BYTES" -lt 157286400 || exit 11

# 2. CLI cold start budget: < 200 ms (median of 5 runs)
COLD_START_P50=$(.github/scripts/median.sh 5 -- "$BIN" --version)
test "$COLD_START_P50" -lt 200 || exit 12

# 3. Ingest throughput: >= 50 items/s
THROUGHPUT=$(cargo bench -p singularmem-core --bench store_perf -- ingest_throughput --output-format=bencher | awk '/items_per_sec/ {print $3}')
awk -v v="$THROUGHPUT" 'BEGIN { exit !(v >= 50) }' || exit 13

# 4. Point-read query latency p95: < 100 ms
QUERY_P95=$(cargo bench -p singularmem-core --bench store_perf -- get_p95 --output-format=bencher | awk '/p95_ms/ {print $3}')
awk -v v="$QUERY_P95" 'BEGIN { exit !(v < 100) }' || exit 14

echo "All perf budgets satisfied: size=${SIZE_BYTES}B cold=${COLD_START_P50}ms ingest=${THROUGHPUT}/s p95=${QUERY_P95}ms"
```

Fixed exit codes (11–14) name which budget broke. The benches use
`criterion` 0.5; criterion writes structured output that the script
parses.

If a budget flakes more than ~2% of runs in the first month, we tighten
the thresholds with explicit headroom rather than relax the budget —
the constitution requires amendment to relax (Principle X).

**CI workflow update.** `.github/workflows/ci.yml` gains two new jobs
on `ubuntu-latest`:

- `tests-offline` — Docker container with `--network=none`, runs
  `cargo test --all-targets --workspace`.
- `perf-budgets` — runs `.github/scripts/perf-check.sh`. Blocks merge.

Both inherit to `macos-advisory` (still non-blocking).

## Open questions

The implementation must resolve, but they are operational rather than
design:

1. **Final dependency versions.** I named major versions (`rusqlite`
   0.32, `clap` 4, `serde` 1, etc.) but minor versions are pinned at
   plan-write time using whatever is current; the plan will name the
   exact `=X.Y.Z` versions.
2. **`dirs` vs `directories` crate.** Both expose XDG-style data-dir
   discovery. `dirs` is smaller and standard in the Rust ecosystem;
   `directories` is more featureful. Plan picks `dirs`.
3. **`actionlint` job in CI.** Optional addition that catches
   workflow-syntax errors before push. Cheap and useful; plan may add
   it as a non-blocking advisory job.

## Acceptance criteria

Memory Store v0 is done when *all* of these are observable on `main`:

1. **`crates/singularmem-core/`** exists as a workspace member with the
   eight modules from Section "Architecture"; `cargo doc -p
   singularmem-core` builds and every `pub` item in `lib.rs` carries a
   doc comment.
2. **`docs/formats/store-v1.md`** committed with the full DDL from
   Section "On-disk format", the `singularmem_meta` key registry, the
   JSONL `export-v1` schema, and the migration ratchet rules.
3. **The five CLI verbs work** end-to-end against a fresh store:
   - `singularmem ingest --content "hello" --tag greeting` prints a
     ULID.
   - `singularmem get <id>` prints `hello` to stdout.
   - `singularmem list --tag greeting --format=jsonl` includes the
     item.
   - `singularmem ingest --content "corrected hello" --supersedes <id>`
     succeeds; `singularmem revisions <new-id>` shows the chain
     newest-first.
   - `singularmem export | head -1` is a JSON object with
     `_singularmem_format = "export-v1"`.
4. **Validation surfaces honestly per Principle VII**: ingesting empty
   content, oversized content, an invalid `metadata` JSON shape, or a
   non-existent `--supersedes` ID each produces a stderr message naming
   what failed + what was attempted + that no state changed; exit code
   is non-zero.
5. **Round-trip test passes**
   (`crates/singularmem-core/tests/format.rs::open_core_only_round_trip`)
   — the Principle III.b end-to-end test demonstrating ingest → list →
   export → re-import with only `singularmem-core` + stdlib.
6. **Format version recorded**: `singularmem_meta` table contains
   `('format_version', '1')` after `Store::open` on a fresh path.
   Verified by a SQL query in the integration tests.
7. **Network-free tests pass**: the `tests-offline` CI job (Docker
   container with `--network=none`) runs `cargo test --all-targets
   --workspace` and exits 0.
8. **Perf budgets enforced**: the `perf-budgets` CI job exits 0 on
   `ubuntu-latest` against the four numeric budgets from Principle X
   (binary size < 150 MB, CLI cold start < 200 ms median-of-5, ingest
   throughput ≥ 50 items/s, point-read p95 < 100 ms).
9. **CI green** on the v0 PR — every blocking job from bootstrap (fmt,
   clippy, check, build, test, audit, dco) plus the two new jobs
   (`tests-offline`, `perf-budgets`).
10. **Version bump** to `0.1.0` in the workspace `Cargo.toml`;
    `singularmem --version` prints `singularmem 0.1.0`. Tag `v0.1.0`
    pushed after merge.
11. **No `[PLACEHOLDER]` strings** in any committed file under
    `docs/formats/` or in the new public API doc comments.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | The store is a local SQLite file. The lib makes zero network calls; rusqlite is bundled. Default store path is per-user XDG data dir, not a remote service. |
| **II — Provider-Agnostic by Contract** | No LLM provider integration in this sub-project. First relevance is sub-project 3. |
| **III — Open Core with a Stable Boundary** | Wholly open. The on-disk format spec at `docs/formats/store-v1.md` satisfies "third parties MUST be able to read a memory store without running our binary" — anyone with a SQLite tool and the spec can write a loader. III.b is verified by `open_core_only_round_trip` and by the dependency graph (no proprietary crates in `singularmem-core`). III.c (paid-tier exit) is N/A — there is no paid tier yet, but `singularmem export` already provides the open-format dump that III.c will eventually require. |
| **IV — CLI-First, GUI-Visible** | Every library capability has a corresponding CLI verb (Section "Interfaces"). No GUI exists yet. |
| **V — Composable Library Architecture** | `crates/singularmem-core` is a standalone library with a documented public API and its own test suite. The root-level binary is the thin shell pattern the bootstrap design promised. Sub-project 4's MCP server, sub-project 5's TS binding, and the eventual proprietary GUI all consume this same library through the public API. |
| **VI — Deterministic and Offline-Testable** | `Clock` and `Rng` are injected via traits with default implementations. The `tests-offline` CI job runs the full test suite in a network-disabled container — the strongest possible "offline" guarantee. Property tests verify determinism with injected state. |
| **VII — Honest Failure Modes** | The `Error` enum's variants each carry the three Principle VII pieces (operation, attempt, preserved state). CLI exit codes are stable and documented. No silent fallbacks: missing IDs error with non-zero exit; ambiguous-latest forks return `AmbiguousLatest` rather than guessing. |
| **VIII — Privacy Telemetry Boundary** | No telemetry. Nothing transmits. |
| **IX — Accessible by Default (WCAG 2.2 AA)** | CLI-only surface. clap's auto-generated `--help` is plain text — screen-reader friendly. Color output respects `NO_COLOR` (clap's default). No animations or motion to suppress. |
| **X — Performance Budgets, Enforced in CI** | This is the sub-project where Principle X actually engages. The `perf-budgets` CI job enforces all four numeric budgets from the constitution against `ubuntu-latest`. Bootstrap deferred this; sub-project 1 closes the gap. |
