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
2. If the journal now holds more than `COMPACT_THRESHOLD = 256`
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
6. **`journal.bin` creation, reversed back to lazy during the Task 4 fix
   wave.** This bullet originally recorded the opposite choice — eager
   creation by `Journal::open` inside every `VectorIndex::open` call,
   including read-only opens — and the cost that came with it: opening a
   vector directory on a read-only filesystem failed where it previously
   succeeded. The fix wave (`4d26431`,
   `fix(search): journal laziness, lock-file open mode, temp cleanup, lock
   tests`) reversed this: `Journal::open` now writes nothing when
   `journal.bin` is absent, and the file (with its header) is created only
   by the first `Journal::append`. A fully compacted directory therefore
   opens read-only without creating or touching any new file, restoring
   the pre-journal behaviour. Guarded by
   `open_creates_nothing_until_the_first_append` in
   `crates/singularmem-search/src/vector_journal.rs` and
   `open_of_a_complete_v2_directory_writes_nothing` in
   `tests/vector_index_journal.rs` (mutation-checked: making `Journal::open`
   eager fails the latter).
7. **fs4 API path**: `fs4 = "0.8"` (default `sync` feature) exports the
   lock trait at `fs4::FileExt`, not `fs4::fs_std::FileExt` (the 0.9+
   path this design's text didn't pin a version for).
8. **Perf-check.sh exit codes renumbered.** The two new gates
   (`ingest_with_indexes`, single-item ingest — originally
   `ingest_single_with_indexes`, later split per deviation 9 below into
   `ingest_single_with_vector_index` for the gate itself and the ungated
   `ingest_single_with_both_hooks`) take exit codes 14 and 15, placed right
   after the existing `ingest_one` gate (13); the pre-existing
   query/semantic/hybrid gates shift from 14/15/16 to 16/17/18 to make
   room, since the numbers this design's Part 3 text suggested for the two
   new gates (15, 16) collided with those two already-assigned codes.
9. **The single-item gate was split into a gated vector-only bench and an
   ungated combined-hooks bench, resolving the concern originally recorded
   here.** As specified (both a Tantivy `Index` hook and an `EmbedderIndex`
   in one `MultiHook`), the single-item gate measured a figure dominated by
   Tantivy's pre-existing per-item commit cost (~88 ms on the measurement
   machine), not the vector index's own cost (~6.75 ms, isolated) — the
   combined figure would very likely have exceeded the 20 ms budget in CI
   even though the vector-index improvement it was meant to verify is real
   and measured. This is not a sub-project 17 regression: Tantivy's
   per-item commit cost predates all of Tasks 1–4 and this design lists
   "Changing the Tantivy sidecar" as a non-goal.
   Resolved during reconciliation (sub-project 17) by renaming the
   combined-hooks bench to `ingest_single_with_both_hooks` (kept ungated,
   reported informationally by `.github/scripts/perf-check.sh`'s summary
   line) and adding `ingest_single_with_vector_index` — same 20,000
   pre-seeded corpus, but opened with **only** the `EmbedderIndex` hook —
   which is what the ≤ 20 ms gate now measures (exit code 15, unchanged).
   See `docs/benchmarks/ingest.md` § "Why the gate is vector-only" for the
   isolated numbers, and that doc's benchmarks table for both current
   medians. Tantivy's per-item `commit` cost is listed there as the next
   performance follow-up.
10. **`Keymap` gained a `generation: u64` field**, not in this design's
    original text, bumped by one on every successful compaction and
    serialised as the first field (before `next_key`) in `format_version
    "2"`'s `keymap.bin`. A handle records the generation it loaded; when
    the on-disk keymap has moved past it, another handle has compacted and
    this handle's in-memory graph is stale relative to disk. `"1"` keymaps
    have no `generation` field and are read through the separate `KeymapV1`
    struct, converted with `generation: 0`. Needed so `compact_locked` can
    detect a stale handle and reload before saving over another handle's
    work (deviation 11 below); without it, the concurrency tests recorded
    `left: 40, right: 80` instead of both handles' vectors surviving.
11. **`open` takes `<dir>/lock` in SHARED mode** (not just `commit`),
    with the same five-attempt `50/100/200/400 ms` backoff, and holds it
    for the whole load; several readers may hold the shared lock at once,
    only a commit's exclusive lock excludes them. This stops a load from
    ever observing one of a compaction's two renames half-done. If the
    load then fails with a `Usearch` error or a `keymap.bin` deserialize
    error, the lock is dropped, re-taken, and the load retried exactly
    once — the shape a load racing a compaction that started just before
    the lock was taken can still produce. Not in this design's original
    text, which only locked `commit`.
12. **Lock acquisition downgrades to unlocked on a read-only directory.**
    If `<dir>/lock` cannot be *opened* because the directory is read-only
    (`PermissionDenied` / `ReadOnlyFilesystem`), `open`'s load proceeds
    without the lock (logged at `debug`) instead of failing — otherwise
    deviation 11's shared-lock-on-open change would have made a read-only
    vector directory unopenable, which used to work.
13. **Replay's key-collision guard uses `usearch::Index::contains`, not a
    recorded `max_key`.** The brief that drove the Task 4 fix wave
    suggested `next_key = max(next_key, highest key in the graph + 1)`, or
    recording `max_key` in the keymap; neither works, because `usearch`
    2.15 has no key-enumeration API and a `max_key` recorded in the keymap
    is stale in exactly the torn-compaction case the check exists for (the
    on-disk keymap being read is the *old* one). Instead, each key is
    checked with `index.contains(key)` as it is issued during replay; a
    hit means `index.usearch` is ahead of `keymap.bin` (a torn
    compaction), the stale occupant is evicted with `index.remove(key)`
    first, and the record being replayed — the authoritative value for
    that key — is inserted. Exact for any gap pattern (including gaps left
    by `remove`), at the cost of one hash lookup per insert.
14. **Directory-fsync failure now propagates as an error, not just a
    debug log.** Compaction's `sync_dir` call — which fsyncs `<dir>` after
    the `index.usearch`/`keymap.bin` renames so their durability ordering
    survives a power cut — returns `Err` on failure everywhere except
    Windows (which has no directory handle to sync, so a failure to open
    one there is tolerated, not reported). The renamed pair is already
    consistent on failure; what's lost is only the guarantee that ordering
    survives a crash immediately after, which is worth surfacing rather
    than swallowing.
15. **The `count == 0` compaction path goes through the same
    temp-file-plus-rename as the non-empty path**, not a direct write: the
    keymap is written to `keymap.bin.tmp`, fsynced, and renamed over
    `keymap.bin`, stamping the new (empty) generation. Pinned by
    `compacting_an_empty_index_renames_the_keymap_and_drops_index_usearch`,
    which replaces a `chmod 0o444` `keymap.bin` (only possible via
    temp+rename, not a direct overwrite) and confirms `index.usearch` is
    gone afterward. **The ordering of the two mutations was reversed during
    the whole-branch review wave — see deviation 18 below.**
16. **A re-added `ItemId` evicts its previous USearch key.**
    `VectorIndex::add` is documented as "add **or replace**", but
    `insert_entries` issued a fresh sequential key for every entry and never
    dropped the id's old one, so a second vector simply piled up beside the
    first. The visible bug: `singularmem reindex --with-embeddings` *without*
    `--reset-vectors` doubled the graph on every run and `search` returned
    the same id twice, once per key. Fixed by removing the id's existing key
    from the graph (`index.remove`) and from both keymap directions before
    the new key is issued. The old key is **not** reused — `next_key` only
    moves forward and the vacated key stays a hole — because the sequential
    numbering is what journal replay reproduces and what the
    `index.contains(key)` torn-compaction guard (deviation 13) reasons
    about. Pinned by
    `re_adding_an_id_replaces_its_vector_instead_of_duplicating_it`
    (`tests/vector_index_journal.rs`: one id, two vectors, `len() == 1`, the
    *new* vector's self-similarity, and the same after journal replay and
    after a compaction + reopen) and, at the CLI level, by
    `reindex_with_embeddings_twice_does_not_double_the_vector_index`
    (`tests/cli.rs`). Both mutation-checked: deleting the eviction fails
    both.
17. **`open` resets a keymap that names vectors with no `index.usearch` to
    hold them.** An absent graph file beside a non-empty keymap is a state no
    successful compaction produces — it can only be a crash inside the
    two-step empty-index compaction — so the keymap is the stale half and is
    reset to empty (keeping `generation`, which other handles still compare
    against, and `next_key`, because keys are never reused), logged at
    `warn`, before the journal is replayed. Without this, the directory
    reported documents that no query could ever return. Pinned by
    `an_absent_index_beside_a_non_empty_keymap_is_reset_on_open`;
    mutation-checked.
18. **The `count == 0` compaction now removes `index.usearch` *before* it
    renames the empty `keymap.bin` into place** — the reverse of the order
    deviation 15 originally recorded. Either order can be interrupted; the
    question is which leftover state is recoverable. Removing the graph first
    leaves (no index + old keymap), which deviation 17's rule resets cleanly.
    Renaming the keymap first leaves (old index + empty keymap): the removed
    vectors are still in the graph, now unnamed, and every later compaction
    serialises them out again — a permanent leak of exactly the data the user
    asked to delete, with no way to identify it afterwards.
19. **A torn pair with no journal is reported rather than repaired, and
    `doc_count()` counts the keymap.** An end-of-batch commit skips the
    journal (see the vectors-v2 doc's "Commit and compaction rules"), so a
    crash between compaction's two renames can leave a new `index.usearch`
    beside an old `keymap.bin` with nothing on disk to reconstruct the
    difference from: the missing `ItemId`s lived only in the keymap that
    never landed, and USearch 2.15 cannot enumerate a graph's keys. `open`
    now logs a `warn` naming both counts and the recovery command
    (`singularmem reindex --with-embeddings --reset-vectors --force`), and
    `doc_count()`/`len()` return the **keymap's** entry count — the
    searchable count, since `search` filters every hit through
    `keymap.forward` — instead of `usearch::Index::size()`, which would
    over-report the corpus for the life of the directory. Pinned by
    `a_torn_pair_with_no_journal_reports_the_keymap_count`;
    mutation-checked.
20. **Windows lock contention is recognised by raw OS error, not by
    `ErrorKind`.** `acquire_lock_at` retried only
    `io::ErrorKind::WouldBlock`. On Windows, `LockFileEx` reports an
    already-held lock as `ERROR_LOCK_VIOLATION` (raw OS error 33), which Rust
    maps to the unstable `ErrorKind::Uncategorized` — unmatchable, so every
    genuinely contended commit failed immediately instead of backing off.
    `is_lock_contention` now also accepts `raw_os_error() == Some(33)` under
    `cfg(windows)`. Not testable from this workspace (no Windows runner in
    the test matrix); documented on the function and in the vectors-v2 doc's
    "`lock`" section.
21. **`Journal::create_file` fsyncs the parent directory.** It fsynced the
    new file's contents but not the directory entry naming it, so the first
    `commit(false)` in a directory's life could return `Ok` and lose the
    whole journal to a power cut — bytes on the platter, no name pointing at
    them. `sync_file`/`sync_dir` moved out of `vector_index.rs` into a shared
    `crate::fsync` module, since both the journal's creation and
    compaction's renames need them. The fsync itself is not observable from
    a test; `create_file_writes_the_header_and_survives_a_relative_path`
    covers the behaviour around it (including the bare-filename case, where
    there is no parent directory to open) and the guarantee is stated in the
    function's doc comment.
22. **Documentation corrections.** `docs/formats/vectors-v2.md`'s claim that
    "at every point the directory is one of exactly two consistent states"
    was false — deviations 17-19 name three states it did not cover. It is
    replaced by a twelve-row crash-point table listing every intermediate
    state and what `open` does with it, with the torn pair (row 6) marked
    explicitly as the one hole, plus two new "Known limitation" sections
    beside the existing in-memory-tombstone one.
    `docs/benchmarks/ingest.md` gains a "Read-side cost" section with the
    open-latency measurement the write-side numbers were silent about
    (11.9 ms with an empty journal vs 638.8 ms with 999 records at the original threshold of 1,000, ~165 ms with 255 records at the final threshold of 256, at 20,000
    vectors) and the two things that bound it. `crates/singularmem-core/`
    `src/hook.rs`'s module doc said `IndexHook` has "three methods"; it has
    four (`on_ingest`, `on_reindex`, `commit`, and the defaulted
    `on_ingest_batch`, added by this sub-project).
23. **`COMPACT_THRESHOLD` lowered from 1,000 to 256.** The design chose
   1,000 by write-side reasoning only. The open-latency measurement showed
   readers replaying a near-full journal at ~0.6 ms per record (638.8 ms at
   999 records), which every short-lived CLI search or MCP retrieve would
   pay. At 256 the worst case is ~165 ms; compaction (~60 ms at 50,000
   vectors) every 256 single-item commits adds under a millisecond per item
   amortised. `docs/formats/vectors-v2.md` and `docs/benchmarks/ingest.md`
   carry both numbers.
