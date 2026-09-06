# Ingest throughput (sub-project 17) — design

**Date:** 2026-09-06
**Status:** approved design, awaiting plan
**Programme:** post-parity performance work (after 16 MCP/Node surface)

## Goal

Make ingest with the vector index attached clear the constitution's
Principle X floor (≥ 50 items/s) with margin, and make single-item
ingest cost independent of index size, without changing the
synchronous semantics: when an ingest returns, its vectors are on disk
and searchable.

## Measurements that drive the design (Apple M2 Max, release build)

| Case | Today |
|---|---|
| Store only, one transaction per item | ~12,700 items/s |
| Real model, short text (~80 chars), one at a time | 400 texts/s |
| Real model, short text, batches of 128 | 977 texts/s |
| Real model, realistic turns (~1,500 chars), one at a time | 42 texts/s |
| Real model, realistic turns, batches of 32 | 62 texts/s |
| USearch `save` at 1k / 10k / 50k items | 2 ms / 11 ms / 58 ms (1 / 16 / 84 MB) |
| End-to-end ingest with vectors (LongMemEval run) | ~56 items/s |

Two costs dominate: the vector hook embeds one item at a time, and
every commit rewrites the whole USearch file. The model itself is
compute-bound, so batching alone yields ~1.5× on realistic text and
~2.4× on short text; the rewrite is what makes single-item ingest
(CLI `ingest`, MCP `memory_ingest`, editor hooks) scale with store
size.

## Non-goals

- Deferred or asynchronous embedding (rejected: semantic search must
  be current when ingest returns).
- Hardware acceleration (CoreML, thread tuning) or model changes.
- Changing the Tantivy sidecar.

## Part 1 — batch hook

### `IndexHook::on_ingest_batch`

```rust
pub trait IndexHook: Send + Sync {
    fn on_ingest(&self, item: &Item) -> Result<()>;
    fn on_reindex(&self, item: &Item) -> Result<()>;
    fn commit(&self) -> Result<()>;

    /// Index a batch. Default: `on_ingest` per item, in order.
    fn on_ingest_batch(&self, items: &[Item]) -> Result<()> {
        items.iter().try_for_each(|i| self.on_ingest(i))
    }
}
```

Every existing implementation keeps compiling. `MultiHook` forwards
the batch to each child in order (same warning-per-hook behaviour as
today). `Store::ingest_many` calls `on_ingest_batch` once per SQLite
batch (the driver's `BATCH_SIZE` of 500) and then `commit` once;
`Store::ingest` (single item) is unchanged. `reindex` rebuilds through
the same batch path.

### `EmbedderIndex::on_ingest_batch`

Embeds `items` in chunks of `EMBED_CHUNK = 64` via
`Embedder::embed_batch`, then adds every vector under one keymap/index
lock acquisition. On an error mid-way, vectors already added stay
added; the error propagates and `ingest_many`'s existing warning path
("item is durably stored but un-searchable; run `singularmem reindex`")
applies. `Index` (Tantivy) keeps the default per-item loop; its cost is
already one commit per batch.

## Part 2 — vector journal (vectors format v2)

### Directory layout

```
<store>.vectors/
  .meta.json      format_version "2", model_id, dim, distance, hnsw_m
  index.usearch   HNSW graph, rewritten only on compaction
  keymap.bin      bincode {forward, reverse, next_key}, written with index.usearch
  journal.bin     append-only vectors since the last compaction (new)
  lock            advisory lock file (new)
```

### `journal.bin`

Header: magic `SMVJ` (4 bytes), `u16` format version = 1, `u32 dim`,
`u16` model-id length, model-id bytes (UTF-8). Then records:
`[16-byte ULID big-endian][dim × f32 little-endian]`. A record is one
`write_all` followed by `fsync`. On open, a trailing partial record is
discarded as a crash remnant; anything before it is valid.

### Commit

`EmbedderIndex::commit`:

1. Append every vector added since the last commit to `journal.bin`.
2. If the journal now holds more than `COMPACT_THRESHOLD = 1_000`
   records, or this commit ends a bulk batch (flag set by
   `on_ingest_batch`), **compact**: write `index.usearch` and
   `keymap.bin` to temp files in the same directory, `fsync`, rename
   over the originals, then truncate the journal to its header.

Single-item ingests append a few kilobytes; bulk ingests rewrite once
per batch, as today.

### Open

`VectorIndex::open` loads `index.usearch` and `keymap.bin` as now, then
replays `journal.bin`: for each record whose id is not already in the
keymap, assign the next key and add the vector. The journal header's
`dim` and model id must equal `.meta.json`'s; otherwise open fails
with the existing model-mismatch error naming both values. A v1
directory (no journal, `format_version "1"`) opens unchanged and is
rewritten as v2 on its first commit. `reindex` deletes the whole
directory, journal included.

### Concurrency

Commit (append and compaction) takes an exclusive advisory lock on
`<store>.vectors/lock` with the bounded retry the Tantivy writer lock
already uses in `src/commands/index.rs` (5 attempts, doubling delay
from the same base). Lock exhaustion surfaces as the existing
"index busy" error. Readers do not lock; a reader whose load fails
because a compaction renamed files underneath it retries the load
once.

### Crash safety

- Append: a partial last record is dropped on replay.
- Compaction: temp-file-and-rename means the directory is always
  either (old index + full journal) or (new index + empty journal).
- Keymap and index are written together; a journal record whose id is
  already in the keymap is skipped on replay, which makes replay
  idempotent after a crash between the rename and the truncate.

## Part 3 — measurement

- New Criterion bench in `crates/singularmem-search/benches/search_perf.rs`:
  `ingest_throughput/ingest_with_indexes` — temp store with the Tantivy
  hook and an `EmbedderIndex` over `MockEmbedder`, ingesting batches of
  100 items of ~1,500 chars through `ingest_many`. `.github/scripts/perf-check.sh`
  reads its median and applies the 50 items/s floor. This gates every
  cost except the model.
- `ingest_throughput/ingest_single_with_indexes` — one `Store::ingest`
  at a time into a store pre-seeded with 20,000 vectors; gate: median
  ≤ 20 ms per item. This asserts the size-proportional rewrite is gone.
- `docs/benchmarks/ingest.md` records real-model before/after numbers
  for the three text lengths above, from this machine, with commit and
  CPU noted.

## Error handling

| Situation | Behaviour |
|---|---|
| Journal append fails (disk full, permission) | commit fails; `ingest_many` logs "stored but un-searchable, run reindex" |
| Lock not acquired after retries | existing "index busy" error |
| Journal header ≠ `.meta.json` | open fails naming both values |
| Partial trailing record | dropped silently on replay (logged at debug) |
| Crash during compaction | old or new state, never half-written |

## Testing (all offline, `MockEmbedder`)

- Unit: default `on_ingest_batch` equals per-item; `MultiHook` forwards;
  `EmbedderIndex::on_ingest_batch` yields vectors identical to per-item
  ingest; journal header/record round trip; truncated tail dropped;
  header mismatch rejected; compaction fires at the threshold and at
  batch end, not otherwise; replay skips ids already in the keymap;
  compaction uses temp-and-rename (assert no `index.usearch.tmp` left
  and the old file survives a simulated failure before rename).
- Integration: two processes committing concurrently (reuse the
  concurrent-hooks pattern from sub-project 13) end with every vector
  present and searchable; `reindex` removes the journal; a v1 directory
  opens and becomes v2 after one commit; semantic results on a fixed
  corpus are identical before and after compaction; the CLI/MCP/hook
  single-ingest paths append rather than rewrite (assert file sizes).

## Documentation

- `docs/formats/vectors-v2.md`: directory, journal layout, replay,
  compaction, crash guarantees — enough for a third-party loader.
- `docs/benchmarks/ingest.md` as above.
- README search section: one line on the journal; `docs/hooks.md`: a
  note that session saves no longer rewrite the vector index.

## Acceptance criteria

1. `ingest_with_indexes` median ≥ 200 items/s on the CI runner (mock
   embedder); the perf script gates it at 50.
2. `ingest_single_with_indexes` median ≤ 20 ms at 20,000 pre-seeded
   vectors.
3. Real-model bulk ingest of realistic turns on this machine ≥ 1.4×
   the pre-change figure, recorded in `docs/benchmarks/ingest.md`.
4. Semantic search results identical before/after compaction and
   after a simulated crash mid-compaction.
5. Workspace fmt, clippy (pedantic + nursery, `-D warnings`), tests
   clean; a v1 vectors directory from v0.20.0 opens without `reindex`.

## Deviations

Recorded during Tasks 3–5 (see `.superpowers/sdd/task-3-report.md`,
`task-4-report.md`, `task-5-report.md` for full detail):

1. **Compaction always replays the journal first**, even when the
   compacting handle already has every vector it queued in memory. Not in
   this design's original text: a *different* handle may have appended to
   the shared journal since this handle last read it, and compacting from
   a stale view before replaying would silently delete that writer's
   vectors on truncate. The invariant: a handle may only truncate the
   journal after replaying, under the lock, every record whose id is not
   already in its keymap. Guarded by
   `compaction_from_a_stale_handle_keeps_the_other_handles_vectors` in
   `tests/vector_index_concurrency.rs`; mutation-checked (removing the
   replay call fails exactly that test).
2. **`VectorIndex::add_batch`** added beyond the single-item `add` this
   design describes, so `EmbedderIndex::on_ingest_batch` can assign
   keymap keys and reserve `USearch` capacity once per batch instead of
   once per item.
3. **`VectorIndex::save()`** kept as a compatibility alias for
   `compact()` (flush pending + rewrite unconditionally), so pre-journal
   callers keep working without a rename.
4. **`ItemId::to_bytes`/`from_bytes`** added (as `const fn`, per
   `clippy::missing_const_for_fn`) to give `journal.bin` records a raw
   16-byte big-endian encoding. Added to the shared `ulid_id!` macro, so
   `EntityId`/`FactId` get them too.
5. **In-memory tombstones for `remove()`.** `journal.bin` has no
   tombstone record, so replaying it after a `remove()` would resurrect
   the removed vector on the next compaction. Fixed with an in-process
   `HashSet<ItemId>` that `absorb_journal` consults and that clears once
   the compaction that truncates the journal completes. **Known gap:** a
   *different* handle that replays the journal before the removing
   handle's compaction lands still resurrects the vector — the tombstone
   doesn't cross handles. Fixing this needs an on-disk tombstone record,
   which is a further format change, not implemented here. Documented as
   a known limitation in `docs/formats/vectors-v2.md`. Live impact is nil
   today: `IndexHook` has no delete verb and `VectorIndex::remove` has no
   production caller.
6. **`journal.bin` is created eagerly, not lazily**, by `Journal::open`
   inside every `VectorIndex::open` call (read-only opens included) —
   simpler than deferring creation to the first append, at the cost that
   opening a vector directory on a read-only filesystem now fails where
   it previously succeeded (no test or product path does that today).
7. **fs4 API path**: `fs4 = "0.8"` (default `sync` feature) exports the
   lock trait at `fs4::FileExt`, not `fs4::fs_std::FileExt` (the 0.9+
   path this design's text didn't pin a version for).
8. **Perf-check.sh exit codes renumbered.** The two new gates
   (`ingest_with_indexes`, `ingest_single_with_indexes`) take exit codes
   14 and 15, placed right after the existing `ingest_one` gate (13); the
   pre-existing query/semantic/hybrid gates shift from 14/15/16 to
   16/17/18 to make room, since the numbers this design's Part 3 text
   suggested for the two new gates (15, 16) collided with those two
   already-assigned codes.
9. **The `ingest_single_with_indexes` gate, as specified (both a Tantivy
   `Index` hook and an `EmbedderIndex` in one `MultiHook`), measures a
   figure dominated by Tantivy's pre-existing per-item commit cost
   (~88 ms on the measurement machine), not the vector index's own cost
   (~6.6 ms, isolated). The combined figure is very likely to exceed the
   20 ms budget in CI. This is not a sub-project 17 regression — Tantivy's
   per-item commit cost predates all of Tasks 1–4 and this design lists
   "Changing the Tantivy sidecar" as a non-goal — but it means the literal
   acceptance criterion 2 gate can fail even though the vector-index
   improvement it was meant to verify is real and measured. See
   `docs/benchmarks/ingest.md` § "A caveat on the single-item gate" for
   the isolated numbers, and `task-5-report.md` for the recommendation.
