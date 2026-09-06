# Singularmem Vector Sidecar Format — v2

This document specifies the on-disk layout of the USearch vector sidecar at
`format_version = "2"`, introduced by sub-project 17 (ingest throughput). It
supersedes the `format_version = "1"` layout documented in
[store-v2.md § "USearch vector sidecar"](store-v2.md#usearch-vector-sidecar-optional-format-unstable-across-usearch-versions)
by adding an append-only `journal.bin` and a `lock` file; everything else
(`.meta.json`, `index.usearch`, `keymap.bin`) keeps the same role. **A
third-party tool that reads this document, together with a USearch client
library and a bincode 1.x decoder, can write a complete loader without
referencing any Singularmem source code.** That property is a constitutional
requirement (Principle III.b).

Like the Tantivy sidecar, the vector sidecar is **additive** — it does not
bump the SQLite store's `format_version` and a loader that only reads
`store.db` is unaffected by its presence or absence. It is created by
`singularmem reindex --with-embeddings`, or automatically once an
`EmbedderIndex` hook is wired into ingest.

## Directory layout

```
<store_path>.vectors/          ← sidecar root (e.g. store.db.vectors/)
├── .meta.json                 ← VectorIndexMeta (JSON, stable schema)
├── index.usearch               ← USearch HNSW graph (binary, version-pinned)
├── keymap.bin                  ← Keymap (bincode; u64 key ↔ ItemId)
├── journal.bin                 ← append-only vectors since the last compaction (new in v2)
└── lock                        ← advisory lock file held for the duration of a commit (new in v2)
```

The path convention is unchanged from v1: `<store_path>.vectors/` next to
the store (e.g. `/data/store.db` → `/data/store.db.vectors/`).

## `.meta.json`

A single JSON object, unchanged in shape from v1 except `format_version`:

```json
{
  "format_version": "2",
  "model_id": "sentence-transformers/all-MiniLM-L6-v2@v1",
  "dim": 384,
  "distance": "cosine",
  "hnsw_m": 16,
  "hnsw_ef_construction": 128,
  "created_at": "2026-05-17T12:00:00.000000000Z"
}
```

| Field | Type | Purpose |
|---|---|---|
| `format_version` | `"2"` (string) | Sidecar layout version. `"1"` directories (no `journal.bin`) open unchanged and are upgraded to `"2"` on their first commit — see "v1 → v2 upgrade" below. |
| `model_id` | string | Stable embedding model identifier, e.g. `"sentence-transformers/all-MiniLM-L6-v2@v1"`. Must match both `index.usearch`'s embedder and `journal.bin`'s header (see below); a mismatch on open fails with `Error::ModelMismatch` naming both values. |
| `dim` | integer | Embedding dimension. Must match the model's output dimension, `index.usearch`'s vectors, and `journal.bin`'s header `dim`. |
| `distance` | `"cosine"` | Distance metric used in the HNSW graph. |
| `hnsw_m` | integer | HNSW connectivity parameter `M`. Fixed at first creation; not reconfigurable without a full rebuild. |
| `hnsw_ef_construction` | integer | HNSW `ef` at build time. Fixed at first creation. |
| `created_at` | RFC 3339 timestamp | Wall-clock time the sidecar was first created. Not touched by later commits or the v1→v2 upgrade. |

`expansion_search` (query-time `ef`) is a runtime option, not persisted here;
a loader can pick its own value freely without affecting compatibility.

## `keymap.bin` — the `Keymap` struct

`keymap.bin` is a [bincode 1.x](https://docs.rs/bincode/1/) serialisation
(default `bincode::serialize` options: little-endian, fixed-width integers,
no length-prefix compaction) of:

```rust
struct Keymap {
    next_key: u64,
    forward: BTreeMap<u64, ItemId>,
    reverse: HashMap<ItemId, u64>,
}
```

`ItemId` is a `#[serde(transparent)]` wrapper around a ULID, and ULID's
`Serialize` impl (from the `ulid` crate) emits its **26-character canonical
string**, not the raw 16 bytes used in `journal.bin` — this is the one
surprising divergence in the format and worth calling out explicitly: two
sidecar files encode the same identifier two different ways.

Byte layout (every integer little-endian, every collection length-prefixed
as a `u64` element count — bincode 1.x defaults, no varint compaction):

```
keymap.bin :=
  next_key         u64                                  next free USearch key
  forward_len      u64                                  element count
  forward_entry*   (u64 key, item_id)                    ascending key order (BTreeMap)
  reverse_len      u64                                  element count
  reverse_entry*   (item_id, u64 value)                  HashMap iteration order (unspecified)

item_id :=
  str_len   u64        always 26 for a well-formed ULID
  str_bytes [u8; str_len]   ASCII, Crockford base32, uppercase, e.g. "01ARZ3NDEKTSV4RRFFQ69G5FAV"
```

### Worked example

A `Keymap` with `next_key = 1` and one entry (USearch key `0` ↔ the ULID
whose integer value is `1`, canonical string
`"00000000000000000000000001"`) serialises to exactly this 108-byte
sequence:

```
offset  bytes                                             field
0       01 00 00 00 00 00 00 00                           next_key = 1 (u64 LE)
8       01 00 00 00 00 00 00 00                           forward_len = 1 (u64 LE)
16      00 00 00 00 00 00 00 00                           forward[0].key = 0 (u64 LE)
24      1A 00 00 00 00 00 00 00                           forward[0].value: str_len = 26
32      30 30 30 30 30 30 30 30 30 30 30 30 30 30 30 30   "0000000000000000" (16 of 26 chars)
48      30 30 30 30 30 30 30 30 30 31                     "0000000001" (remaining 10 chars)
58      01 00 00 00 00 00 00 00                           reverse_len = 1 (u64 LE)
66      1A 00 00 00 00 00 00 00                           reverse[0].key: str_len = 26
74      30 30 30 30 30 30 30 30 30 30 30 30 30 30 30 30   "0000000000000000"
90      30 30 30 30 30 30 30 30 30 31                     "0000000001"
100     00 00 00 00 00 00 00 00                           reverse[0].value = 0 (u64 LE)
```

Total: 108 bytes. A loader that only needs USearch-key → `ItemId`
translation can decode just `next_key` and `forward` and ignore the trailing
`reverse` map (it is redundant — a `HashMap` built from `forward` — kept
because the in-process code needs O(1) lookup in both directions, not
because the file needs it).

## `index.usearch`

The USearch HNSW graph, in USearch's own native binary format, rewritten
only on compaction (see below) — a v2 directory with a small journal and a
large `index.usearch` can go many commits without touching this file at
all. Version-pinned exactly as in v1: `usearch = "=2.15.3"` (see
[`Cargo.lock`](../../Cargo.lock)). The binary format is owned by the USearch
project and is **not** guaranteed stable across USearch major/minor bumps;
after a Singularmem release changes the pin, run
`singularmem reindex --with-embeddings --reset-vectors --force` to rebuild
from SQLite (this deletes the whole `.vectors/` directory, journal
included, before rebuilding).

An empty index (`size() == 0`) never has an `index.usearch` file on disk —
loading a USearch file saved from an empty index can segfault on some
platforms, so compaction removes any stale file instead of writing an empty
one.

## `journal.bin`

Append-only log of `(ItemId, Vec<f32>)` records added since the last
compaction of `index.usearch`.

```
journal.bin :=
  header
  record*        (zero or more, each appended via one write_all + one fsync)

header :=
  magic         [u8; 4]   = "SMVJ"
  version       u16 LE    = 1
  dim           u32 LE
  model_id_len  u16 LE
  model_id      [u8; model_id_len]   UTF-8, no NUL terminator, capped at 512 bytes

record :=
  id            [u8; 16]   ItemId's ULID, big-endian (ulid::Ulid::to_bytes — NOT the
                            26-character string form keymap.bin uses; see above)
  vector        [f32; dim] each f32 little-endian (4 bytes)
```

`header.dim` and `header.model_id` must equal `.meta.json`'s `dim` and
`model_id`; a mismatch fails open with the same `DimMismatch` /
`ModelMismatch` errors as a `.meta.json`/embedder mismatch, naming both
values. Record size is fixed once the header is known: `16 + dim * 4`
bytes.

### Worked example

`journal.bin` opened fresh with `dim = 2`, `model_id = "m@v1"` writes this
16-byte header:

```
offset  bytes                    meaning
0       53 4D 56 4A              magic "SMVJ"
4       01 00                    version = 1 (LE u16)
6       02 00 00 00              dim = 2 (LE u32)
10      04 00                    model_id_len = 4 (LE u16)
12      6D 40 76 31              model_id = "m@v1" (UTF-8)
```

Appending one record — id = the ULID whose integer value is `1`
(big-endian 16-byte form: fifteen `0x00` bytes then `0x01`), vector
`[1.0, 2.0]` — appends this 24-byte record at offset 16:

```
offset  bytes                                            meaning
16      00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01  ULID (big-endian)
32      00 00 80 3F                                      1.0f32 (LE)
36      00 00 00 40                                      2.0f32 (LE)
```

Total file after one append: 40 bytes. A record is one `write_all`
followed by one `fsync`, so a crash never leaves a record half its correct
length mid-file *except* at the very tail (see "Crash safety" below).

### Replay rules

On open, and again at the start of every compaction, the journal is
replayed into the in-memory HNSW graph:

1. Read every complete record after the header. **A trailing byte range
   shorter than one full record is a crash remnant and is silently dropped
   — not an error.** (`bytes.len() % record_len != 0` logs at `debug` and
   truncates to the last complete record.)
2. For each record whose id is **not already present** in the current
   in-memory keymap, assign the next sequential `u64` key and add the
   vector to the graph. A record whose id **is** already in the keymap is
   skipped — this is what makes replay idempotent (see "Crash safety").
3. A record whose id has been locally removed since the journal was last
   truncated is also skipped (see "Known limitation: removals" below).

## `lock`

An empty file used purely as an advisory-lock handle
(`fs4::FileExt::try_lock_exclusive`/`unlock`), never containing meaningful
bytes. A commit (append-then-maybe-compact) takes this lock for its entire
duration; readers (`search`, `contains`) never take it. Lock acquisition
retries up to 5 times with delays `50, 100, 200, 400` ms (doubling, no
sleep after the last attempt); exhaustion surfaces as an "index busy" error
to the caller. Two `VectorIndex` handles — in one process or two — that
both try to commit at the same time serialise through this file; a handle
whose `flock` request would block waits rather than corrupting the other's
write.

## Commit and compaction rules

A **commit** is: append every vector queued since the last commit to
`journal.bin` (one `write_all` + one `fsync` for the whole batch), then
decide whether to **compact**:

- Compact if the journal now holds **more than 1,000 records**
  (`COMPACT_THRESHOLD = 1_000`; strictly greater — a journal holding
  exactly 1,000 has not triggered compaction yet), **or**
- Compact unconditionally if this commit closes a bulk batch (a single
  `Store::ingest_many` call ends with exactly one compacting commit,
  regardless of journal size).

A single-item ingest that doesn't cross the threshold therefore costs one
small `journal.bin` append (a few kilobytes) instead of a full
`index.usearch` rewrite — this is the whole point of the format change (see
`docs/benchmarks/ingest.md`).

**Compaction always replays the journal first**, even though the compacting
handle already has every vector it itself queued in memory. This is not
redundant: a *different* handle (another process, or another handle in the
same process) may have appended vectors to the shared journal since this
handle last read it, and this handle's in-memory graph knows nothing about
them. Compacting straight from a stale in-memory view and then truncating
the journal would silently delete that other writer's vectors. So
compaction is always: replay → save → truncate, never save → truncate
directly. (This rule is not in the original design write-up; it was added
during implementation and is recorded in the spec's Deviations section.)

Once replay is done, compaction:

1. Serialises the in-memory HNSW graph to `index.usearch.tmp`, `fsync`s it,
   and renames it over `index.usearch` (skipped entirely, with the old
   file removed instead, if the graph is empty — see above).
2. Serialises the keymap to `keymap.bin.tmp`, `fsync`s it, and renames it
   over `keymap.bin`.
3. Best-effort `fsync`s the directory itself, so the two renames are
   durable (not every platform allows opening a directory for this; a
   failure here costs durability of an already-consistent state, not
   correctness).
4. Truncates `journal.bin` back to just its header.

## Crash safety

| Failure point | Resulting state |
|---|---|
| Crash mid-`write_all` of a journal record | The record is a partial tail; replay drops it silently. Every record before it is valid and durable. |
| Crash between the `index.usearch.tmp`/`keymap.bin.tmp` rename and the journal truncate | The directory has the **new** index and keymap, but the **full** (pre-compaction) journal. On next open, every record replays — but every id in it is already in the freshly-written keymap, so replay is a no-op. Idempotent by construction. |
| Crash before either rename | The directory has the **old** index and keymap and the full journal — as if the compaction never started. Next open replays the journal from scratch. |
| Journal append fails (disk full, permission) | The commit fails; the queued vectors are restored to the front of the pending queue (ahead of anything queued in the meantime) so a later commit can still make them durable. `ingest_many` surfaces "item is durably stored but un-searchable; run `singularmem reindex`". |
| Lock not acquired after 5 attempts | "index busy" error; no partial write. |

At every point the directory is one of exactly two consistent states: (old
index + full journal) or (new index + empty journal). There is no state in
between that a loader could observe as corrupt, other than a dropped
partial tail record, which is by design, not corruption.

## v1 → v2 upgrade

A directory with `.meta.json`'s `"format_version": "1"` (no `journal.bin`,
no `lock`) opens unchanged: the in-memory metadata is promoted to `"2"`
immediately, but `.meta.json` on disk is **not** rewritten by a read-only
open — only the first *commit* writes the upgraded `.meta.json`, under the
commit lock, alongside creating `journal.bin` (which `Journal::open`
creates with its header the moment any `VectorIndex::open` runs, v1 or v2
— so a fresh `journal.bin` header can appear before the first commit even
though `.meta.json` still says `"1"` until that commit lands). After the
first commit: `.meta.json` says `"2"`, `journal.bin` exists, and
`index.usearch`/`keymap.bin` are unchanged from whatever v1 last wrote
(compaction only fires on that same commit if the batch/threshold rule
above says so). No data loss and no explicit migration command — the
upgrade is purely a side effect of continuing to use the store.

## Known limitation: removals are not journaled

`journal.bin` has no tombstone record — it can only say "this vector
exists," never "this vector was removed." A removed id is tracked only in
an **in-memory** `HashSet` on the removing `VectorIndex` handle, cleared
once the compaction that truncates the journal completes. This is enough
to keep a single handle's own `remove()` from being resurrected by its own
next compaction's replay, but it does **not** propagate to any other
handle: a second handle that replays the journal (on open, or its own
compaction) before the first handle's removal-clearing compaction lands
will still see and re-add the "removed" vector. Fixing this properly needs
an on-disk tombstone record in the journal format — a spec change, not
implemented here. Today's blast radius is nil: `IndexHook` has no delete
verb, and `VectorIndex::remove` has no production caller — only tests
exercise it.

## Writing a third-party vector loader

1. Confirm `<store_path>.vectors/.meta.json` exists; if absent, there is no
   vector sidecar.
2. Read `.meta.json`. If `format_version == "1"`, there is no
   `journal.bin`/`lock` to read — treat as fully compacted and follow
   store-v2.md's v1 walkthrough instead. If `"2"`, continue below. Note
   `model_id`, `dim`, `distance`.
3. If `journal.bin` exists, read its header and confirm `dim`/`model_id`
   match `.meta.json`; then replay every complete record (drop a trailing
   partial one) per the "Replay rules" above, remembering which ids you
   already loaded from `keymap.bin` so you skip duplicates.
4. Read `keymap.bin` with a bincode 1.x deserialiser as a
   `{ next_key: u64, forward: BTreeMap<u64, String>, reverse: HashMap<String, u64> }`
   — the `forward` map alone is enough to translate USearch keys to ULID
   strings.
5. Open `index.usearch` with USearch `=2.15.3` (or the version pinned in
   the Singularmem release you are targeting). Construct an index with the
   same `dim` and `distance` as `.meta.json`, then `index.load(path)` — or
   skip this step entirely if the graph is empty (no `index.usearch` file
   exists in that case).
6. Assign the journal-replayed vectors (step 3) fresh sequential keys
   continuing from `keymap.bin`'s `next_key`, and add them to the loaded
   graph the same way the journal records were embedded (unit-length f32
   vectors, cosine distance).
7. Issue KNN queries: `index.search(query_vector, k)` returns `(keys,
   distances)`; translate `keys` back to `ItemId`s via the forward map (and
   your own journal-assigned keys from step 6).

## HNSW parameters (unchanged from v1)

| Parameter | Value | Notes |
|---|---|---|
| `hnsw_m` | 16 | Connectivity. Increase to 32–64 for higher recall on large collections. |
| `hnsw_ef_construction` | 128 | Build-time `ef`. Increase to 256 for higher recall at slower build. |
| `expansion_search` | 64 | Query-time `ef`, not persisted. Increase to 128 for higher recall at ~2× query time. |
| Distance metric | Cosine | Vectors are L2-normalised before insertion; cosine similarity reduces to dot product. |
| Scalar type | f32 | 32-bit floats. |
