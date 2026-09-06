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
├── journal.bin                 ← append-only vectors since the last compaction (new in v2; created lazily on first append)
└── lock                        ← advisory lock file held for the duration of a commit (exclusive) or a load (shared) (new in v2)
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
| `format_version` | `"2"` (string) | Sidecar layout version. `"1"` directories (no `journal.bin`) open unchanged and are upgraded to `"2"` on their first commit — see "v1 → v2 upgrade" below. A value other than `"1"` or `"2"` fails open with `Error::IndexCorrupted`, whose reason names the unsupported version — a forward guard against a directory written by a newer build being silently misread as one of the two layouts this build understands. |
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
    generation: u64,
    next_key: u64,
    forward: BTreeMap<u64, ItemId>,
    reverse: HashMap<ItemId, u64>,
}
```

Field order on disk is exactly the declaration order above: `generation`,
`next_key`, `forward`, `reverse` — bincode 1.x has no field names on the
wire, so a reader must decode fields in this order.

`generation` is new in `format_version "2"`; **`"1"` keymaps have no
`generation` field at all** and are decoded through the separate `KeymapV1`
struct (`next_key`, `forward`, `reverse` — the same three fields, no
`generation`), selected by `.meta.json`'s `format_version` saying `"1"`. A
`KeymapV1` is promoted to a `Keymap` with `generation: 0` once loaded. (The
other layout is always tried as a fallback if the expected one fails to
decode: a crash between the `.meta.json` upgrade to `"2"` and the first v2
keymap rename leaves a `"2"` meta beside a still-v1 keymap, and that
directory must still open.)

`generation` is bumped by one on every successful compaction and is what
lets a `VectorIndex` handle tell that another handle — in this process or a
different one — has compacted since this handle last loaded: the handle
records the generation it loaded (`loaded_generation`), and before a
compaction may save it re-reads `keymap.bin`'s generation and reloads from
disk first if the two differ (see "Commit and compaction rules" below).
Without this, a long-lived handle could save its own, now-stale in-memory
graph over a newer on-disk one and silently drop the other handle's
vectors.

`ItemId` is a `#[serde(transparent)]` wrapper around a ULID, and ULID's
`Serialize` impl (from the `ulid` crate) emits its **26-character canonical
string**, not the raw 16 bytes used in `journal.bin` — this is the one
surprising divergence in the format and worth calling out explicitly: two
sidecar files encode the same identifier two different ways.

Byte layout (every integer little-endian, every collection length-prefixed
as a `u64` element count — bincode 1.x defaults, no varint compaction):

```
keymap.bin :=
  generation       u64                                  bumped by one on every successful compaction ("2" only)
  next_key         u64                                  next free USearch key
  forward_len      u64                                  element count
  forward_entry*   (u64 key, item_id)                    ascending key order (BTreeMap)
  reverse_len      u64                                  element count
  reverse_entry*   (item_id, u64 value)                  HashMap iteration order (unspecified)

item_id :=
  str_len   u64        always 26 for a well-formed ULID
  str_bytes [u8; str_len]   ASCII, Crockford base32, uppercase, e.g. "01ARZ3NDEKTSV4RRFFQ69G5FAV"
```

A `"1"` keymap omits the leading `generation` field entirely — its layout
starts directly at `next_key`.

### Worked example

A `format_version "2"` `Keymap` with `generation = 0`, `next_key = 1`, and
one entry (USearch key `0` ↔ the ULID whose integer value is `1`, canonical
string `"00000000000000000000000001"`) serialises to exactly this 116-byte
sequence:

```
offset  bytes                                             field
0       00 00 00 00 00 00 00 00                           generation = 0 (u64 LE)
8       01 00 00 00 00 00 00 00                           next_key = 1 (u64 LE)
16      01 00 00 00 00 00 00 00                           forward_len = 1 (u64 LE)
24      00 00 00 00 00 00 00 00                           forward[0].key = 0 (u64 LE)
32      1A 00 00 00 00 00 00 00                           forward[0].value: str_len = 26
40      30 30 30 30 30 30 30 30 30 30 30 30 30 30 30 30   "0000000000000000" (16 of 26 chars)
56      30 30 30 30 30 30 30 30 30 31                     "0000000001" (remaining 10 chars)
66      01 00 00 00 00 00 00 00                           reverse_len = 1 (u64 LE)
74      1A 00 00 00 00 00 00 00                           reverse[0].key: str_len = 26
82      30 30 30 30 30 30 30 30 30 30 30 30 30 30 30 30   "0000000000000000"
98      30 30 30 30 30 30 30 30 30 31                     "0000000001"
108     00 00 00 00 00 00 00 00                           reverse[0].value = 0 (u64 LE)
```

Total: 116 bytes (108 for the equivalent `"1"` layout, which has no
`generation` field — the same bytes starting from offset 8 above). A loader
that only needs USearch-key → `ItemId` translation can decode `generation`
(if present), `next_key`, and `forward`, and ignore the trailing `reverse`
map (it is redundant — a `HashMap` built from `forward` — kept because the
in-process code needs O(1) lookup in both directions, not because the file
needs it).

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

**`journal.bin` is created lazily, on the first append — not by `open`.**
`Journal::open` derives the header from `dim`/`model_id` and validates an
*existing* file's header against them, but writes nothing when the file is
absent; the file (header included) is created only inside the first call to
`Journal::append`. Practically: opening a fully compacted vector directory
— the common case for a search-only process, and the only case that can
work on a read-only filesystem — creates no file at all, `journal.bin`
included. `replay`, `len`, and `clear` all treat an absent file as an empty
journal, and `clear` (called at the end of a compaction) never creates the
file either — only `append` does.

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
   in-memory keymap (checked with `HashMap::contains_key` against the
   keymap's `reverse` map) and has not been locally removed since the
   journal was last truncated (see "Known limitation: removals" below), a
   sequential `u64` key is assigned from `keymap.next_key`.
3. **That key is itself checked against the graph with
   `usearch::Index::contains` before the vector is added.** A hit means
   `index.usearch` is *ahead* of `keymap.bin` for that key — the signature
   of a torn compaction (a crash between the `index.usearch.tmp` rename and
   the `keymap.bin.tmp` rename: the new graph is in place but the old
   keymap, with a lower `next_key`, is still being read). When this
   happens, the stale occupant is evicted with `index.remove(key)` and the
   record being replayed — the authoritative value, since it is the very
   record the crashed compaction had already folded into the graph under
   that key — is inserted in its place. This recovers a torn rename pair
   cleanly for *any* gap pattern (including gaps left by `remove`), at the
   cost of one hash lookup per insert; a recorded `max_key` cannot do this,
   because in the torn case the keymap being read is the stale one and its
   `max_key` is stale too.
4. A record whose id **is** already in the keymap (and was not evicted by
   step 3) is skipped outright — this is what makes replay idempotent (see
   "Crash safety").

## `lock`

An empty file used purely as an advisory-lock handle (`fs4::FileExt`'s
`try_lock_exclusive`/`try_lock_shared`/`unlock`), never containing
meaningful bytes. A commit (append-then-maybe-compact) takes this lock
**exclusively** for its entire duration. Lock acquisition retries up to 5
times with delays `50, 100, 200, 400` ms (doubling, no sleep after the last
attempt); exhaustion surfaces as an "index busy" error (`Error::Usearch`
with context `"acquiring vector index lock"`) to the caller. Two
`VectorIndex` handles — in one process or two — that both try to commit at
the same time serialise through this file; a handle whose `flock` request
would block waits rather than corrupting the other's write.

**`open` also takes this lock, in SHARED mode, for the whole of a load**
(same five-attempt backoff). Several readers may hold the shared lock at
once — only a commit's exclusive lock excludes them — which is what stops
a load from ever observing one of a compaction's two renames half done:
neither (new `index.usearch` + old `keymap.bin`, which used to make replay
collide on keys the graph already held) nor the reverse (vectors silently
invisible until the next open). While holding the lock, `open` also sweeps
any leftover `index.usearch.tmp` / `keymap.bin.tmp` files from a crashed
compaction (best effort; safe precisely because no compaction can be in
flight under the shared lock). If the load still fails afterward with a
`Usearch` error or a `keymap.bin` deserialize error — the shape a load
racing a compaction that started *just before* the lock was taken can
produce — the lock is dropped, re-taken, and the load is retried exactly
once before the error is returned to the caller.

**A read-only directory downgrades to no lock, not a failed open.** If
`<dir>/lock` cannot be *opened* at all because the directory is read-only
(`PermissionDenied` / `ReadOnlyFilesystem`), the load proceeds unlocked
(logged at `debug`) rather than failing — this is the only way a read-only
vector directory can be opened at all, since it has no lock file to create
and nothing else is writing to it concurrently in that scenario.

## Commit and compaction rules

A **commit** is: take the exclusive `<dir>/lock`, then either

- **end the batch by compacting directly** (see below), if `end_of_batch`
  is set (a single `Store::ingest_many` call closes with exactly one such
  commit) — in which case the queued vectors are **never appended to
  `journal.bin` at all**, because the compaction that follows makes them
  durable in `index.usearch` inside the same locked section. Journalling
  first would write every vector twice (once to the journal, once into the
  graph) and buffer the whole batch a second time for no benefit — a crash
  before the compaction loses the batch either way, and the caller never
  saw `Ok` — or
- **append every vector queued since the last commit to `journal.bin`**
  (in chunks of at most 1,024 records per `Journal::append` call, so a
  large drain never buffers the whole queue twice), then decide whether to
  **compact**: only if the journal now holds **more than 1,000 records**
  (`COMPACT_THRESHOLD = 1_000`; strictly greater — a journal holding
  exactly 1,000 has not triggered compaction yet).

A single-item ingest that doesn't cross the threshold therefore costs one
small `journal.bin` append (a few kilobytes) instead of a full
`index.usearch` rewrite — this is the whole point of the format change (see
`docs/benchmarks/ingest.md`).

**Compaction always reloads a stale handle from disk, then replays the
journal, before it may save.** Two things can be true of the on-disk state
that this handle's in-memory copy does not yet reflect:

1. **Another handle may have compacted since this handle last loaded.**
   `keymap.bin`'s `generation` (see above) is checked against the
   generation this handle loaded; if they differ, this handle's in-memory
   graph and keymap are replaced with a freshly-loaded `index.usearch` +
   `keymap.bin` from disk, and anything still queued in this handle's
   `pending` (never journalled, so the reloaded graph lacks it) is
   re-inserted into the reloaded graph under fresh keys. Tombstones survive
   the reload. Skipping this step lets a long-lived handle save its
   pre-reload graph over a newer on-disk one and silently drop the other
   handle's vectors — this is not in the original design write-up, and is
   recorded in the spec's Deviations section.
2. **A different handle may have appended vectors to the shared journal
   since this handle last absorbed it**, whether or not step 1 fired. This
   handle's in-memory graph knows nothing about those vectors, so
   compacting straight from a stale view and then truncating the journal
   would silently delete them. So compaction is always: reload-if-stale →
   replay → save → truncate, never save → truncate directly.

Once both are done, compaction:

1. Snapshots the in-memory graph and drains `pending` together, under the
   inner lock, so a concurrent `add` is either already reflected in the
   snapshot or still queued afterward — never in neither.
2. If the graph holds at least one vector, serialises it to
   `index.usearch.tmp`, `fsync`s it, and renames it over `index.usearch`.
   An empty graph skips this step entirely — no `index.usearch.tmp` is
   written.
3. **Stamps `keymap.generation = loaded_generation + 1`**, serialises the
   keymap, writes `keymap.bin.tmp`, `fsync`s it, and renames it over
   `keymap.bin`. This happens whether or not the graph was empty — **the
   `count == 0` path goes through this same temp-file-plus-rename, not a
   direct overwrite.**
4. Only *now*, if the graph was empty and a stale `index.usearch` still
   exists on disk, removes it. Doing this after step 3 rather than before
   or concurrently means the pair on disk is never observed as (old graph +
   a keymap already renamed to reflect zero items) — by the time the graph
   file disappears, the keymap naming the empty state is already durable.
5. `fsync`s the directory itself, so the renames in steps 2–4 are durable
   in order. **This failure now propagates as an error** on every platform
   except Windows (which has no directory handle to sync, so a failure to
   *open* one there is tolerated instead of reported) — the renamed pair is
   already internally consistent if this fails, but the guarantee that the
   ordering survives an immediately-following crash is lost, which is
   worth surfacing rather than swallowing.
6. Truncates `journal.bin` back to just its header (an absent journal stays
   absent — this never creates the file) and clears the in-memory
   tombstone set: the journal no longer holds the vectors it was
   suppressing. Stores the new generation as this handle's
   `loaded_generation`.

Any failure between the drain in step 1 and the end of step 6 requeues the
drained records at the front of `pending`, so the next commit still
journals them.

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
no `lock`) opens with its in-memory metadata promoted to `"2"` immediately,
but `.meta.json` on disk is **not** rewritten by a read-only open — only
the first *commit* writes the upgraded `.meta.json`, under the commit
lock. `journal.bin` itself is created lazily (see "`journal.bin` is
created lazily" above), so opening a v1 directory never creates it, on a
read-only filesystem or otherwise. The one file `open` *can* create on a
writable v1 directory that did not exist before this sub-project is
`lock` itself, since the shared load lock is now taken (and, if the
directory is writable, created) for every open, not just a commit — a
read-only directory downgrades to no lock instead (see "`lock`" above), so
a genuinely read-only v1 directory still creates nothing at all. Only once
the first *write* happens (an `add` followed by a `commit`) does
`.meta.json` get rewritten to `"2"` and does `journal.bin` get its header
written (by that commit's first journal append, or skipped entirely if the
commit closes a bulk batch and compacts directly — see "Commit and
compaction rules" below). After the first commit: `.meta.json` says `"2"`,
and `index.usearch`/`keymap.bin` are unchanged from whatever v1 last wrote
unless that commit also compacted. No data loss and no explicit migration
command — the upgrade is purely a side effect of continuing to use the
store.

## Cross-handle visibility

A `VectorIndex` handle sees another handle's journal-only vectors at
exactly two moments: **its own `open`** (which absorbs whatever is in
`journal.bin` at that point) and **its own compaction** (`reload_if_stale`
followed by `absorb_journal` — see "Commit and compaction rules" above).
Between those two moments, a handle's `search` and `contains` calls answer
only from its own in-memory graph and keymap; a vector another handle
added and journalled, but has not yet compacted, is invisible to this
handle's queries until this handle next opens or compacts. This is a
consequence of the journal existing at all — the whole reason the format
changed was to avoid rewriting `index.usearch` on every single-item
commit, and that means a handle's in-memory graph is not automatically
kept current with every other handle's uncompacted writes.

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
   `{ generation: u64, next_key: u64, forward: BTreeMap<u64, String>, reverse: HashMap<String, u64> }`
   — the `forward` map alone is enough to translate USearch keys to ULID
   strings. `generation` only exists once `format_version` is `"2"`; a
   loader that doesn't need to detect concurrent compactions can skip it.
5. Open `index.usearch` with USearch `=2.15.3` (or the version pinned in
   the Singularmem release you are targeting). Construct an index with the
   same `dim` and `distance` as `.meta.json`, then `index.load(path)` — or
   skip this step entirely if the graph is empty (no `index.usearch` file
   exists in that case).
6. Assign the journal-replayed vectors (step 3) fresh sequential keys
   continuing from `keymap.bin`'s `next_key`, and add them to the loaded
   graph the same way the journal records were embedded (unit-length f32
   vectors, cosine distance). **Check each key against the loaded graph
   before adding** (USearch's own key-existence check, e.g. `contains`):
   a hit means the directory was mid-compaction when it was read (a torn
   rename between `index.usearch.tmp` and `keymap.bin.tmp`), and the
   record being replayed — not the graph's existing occupant — is the
   authoritative value for that key (see "Replay rules" above).
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
