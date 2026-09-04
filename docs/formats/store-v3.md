# Singularmem Store Format — v3

This document specifies the on-disk format of a Singularmem memory store
at `format_version = 3`. **A third-party tool that reads this document and
has access to a SQLite library can write a complete loader without
referencing any Singularmem source code.** That property is a
constitutional requirement (Principle III.b).

## File layout

A store is a single SQLite 3 database file (default name: `store.db`),
opened with WAL journaling. Two sidecar files are created automatically
by SQLite when the database is open:

- `store.db-wal` — write-ahead log
- `store.db-shm` — shared memory index for the WAL

The sidecars are recreated on next open and **do not** need to be backed
up. Backing up just `store.db` after a clean shutdown (which any clean
process exit performs) is sufficient.

## Schema

```sql
CREATE TABLE singularmem_meta (
    key    TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
) STRICT;

CREATE TABLE items (
    id           TEXT PRIMARY KEY NOT NULL,
    content      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    supersedes   TEXT,
    source       TEXT,
    metadata     TEXT NOT NULL DEFAULT '{}',
    external_id  TEXT,
    scope        TEXT,
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
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;
```

### Column semantics

**`items.id`** — 26-character ULID in Crockford base32. Uppercase
representation. Case-insensitive when parsed; emitted as uppercase.

**`items.content`** — UTF-8 text. Non-empty. Maximum length 1,048,576
bytes (1 MiB). Enforced by both the application and the SQL `CHECK`
constraint.

**`items.created_at`** — RFC 3339 timestamp with nanosecond precision and
UTC timezone (`Z` suffix). Example: `2026-05-16T12:34:56.123456789Z`.
String-sortable in ISO order matches chronological order, which the
`idx_items_created_at` index relies on.

**`items.supersedes`** — Nullable. When non-null, MUST reference an
existing `items.id`. The FK is `DEFERRABLE INITIALLY DEFERRED` so a
single transaction may insert multiple items that supersede each other in
any insertion order.

**`items.source`** — Nullable. Free-form text label, ≤ 256 bytes.

**`items.metadata`** — TEXT column holding a JSON object. The `CHECK`
constraint enforces that the value is valid JSON AND that the top-level
type is object (not array, not scalar). Default is `'{}'`.

**`items.external_id`** — Nullable. Optional caller-supplied stable
identity for idempotent bulk ingest, ≤ 512 bytes, UTF-8, no `\0`. Unique
across the store when present (`idx_items_external_id` is a partial
unique index that ignores NULLs, so any number of items may have no
`external_id`). `None`/NULL for items ingested without one.

Callers are free to choose any non-empty string, but two conventions are
established by the reference implementation:

| Convention | Example | Used by |
|---|---|---|
| `claude-code:<sessionId>:<uuid>[#n]` | `claude-code:8f3a...:c2b1...#2` | Bulk transcript ingestion keyed by conversation turn. The optional `#n` suffix disambiguates multiple items minted from the same turn. |
| `file:<abs path>[#n]` | `file:/Users/alice/notes.md#1` | Ingestion keyed by a source file path. The optional `#n` suffix disambiguates multiple items from the same file. |

**`item_tags.tag`** — Free-form text, ≤ 64 bytes, no `\0`. Tags are
stored case-sensitively. The `(item_id, tag)` primary key dedupes within
an item.

**`items.scope`** — Nullable. Optional hierarchical scope path, e.g.
`claude-code/singularmem` or `files/repo`. `NULL` for unscoped items.

Validation and normalisation (`singularmem_core::scope::validate`):

- Leading/trailing `/` are stripped; segments are lowercased.
- At least one segment, at most 8 segments (`/`-separated).
- Each segment is 1–64 bytes, matching `[a-z0-9._-]` after lowercasing;
  `.` and `..` segments are rejected (no directory-traversal-style
  paths), as are empty segments (`a//b`).
- The normalised path (segments joined by a single `/`, no
  leading/trailing slash) is at most 512 bytes total.
- Any violation returns `Error::Validation { field: "scope", .. }`
  describing the first rule broken; the item is not persisted.

A **descendant-inclusive** scope filter over `path` matches any row whose
`scope` equals `path` or begins with `path` followed by `/`. As SQL
against a bound parameter `?1` (already normalised):

```sql
scope = ?1 OR scope LIKE ?1 || '/%'
```

Because `_` is both a legal scope byte and a SQL `LIKE` wildcard, the
reference implementation binds the `LIKE` pattern with `_` (and `%` and
`\`) backslash-escaped and appends `ESCAPE '\'` to the clause, rather than
interpolating `?1` directly into the pattern as shown above (which is
illustrative only). An **exact-match** filter instead compares
`scope = ?1` with no `LIKE` at all.

### `singularmem_meta` key registry

| Key | Type | Required? | Purpose |
|---|---|---|---|
| `format_version` | string (`"3"`) | yes | Format version marker. Loaders MUST refuse to operate on a value they do not recognise. |
| `created_at` | RFC 3339 | yes | Wall-clock time the store file was first created. |

Future format versions may add keys; readers MUST ignore unknown keys
within their own format version.

## Migration ratchet

A store at `format_version = N` that is opened by a binary supporting
maximum version `M`:

- `N == M` → open succeeds, no migration.
- `N < M` → loader runs migrators `N → N+1 → ... → M` in a single
  transaction per step; failure rolls back that step and surfaces the
  version it started from.
- `N > M` → loader MUST refuse with an "unsupported format version"
  error. It MUST NOT attempt to operate on a newer format.

The Singularmem reference implementation in `crates/singularmem-core`
supports maximum version `3` from v0.18.0 onward.

## Migration 1 → 2

A store opened writable at `format_version = 1` is migrated in place to
`2` by executing exactly these statements in a single transaction:

```sql
ALTER TABLE items ADD COLUMN external_id TEXT;
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
UPDATE singularmem_meta SET value = '2' WHERE key = 'format_version';
```

The reference implementation opens this transaction with `BEGIN
IMMEDIATE` (taking the write lock up front, rather than deferring it to
the first write) and re-reads `format_version` immediately after
acquiring the lock: if a concurrent writer already moved the store past
`1` while this connection was waiting for the lock, the migration is a
no-op — the transaction is rolled back (there is nothing left to commit)
and `Ok(())` is returned. Otherwise the three statements above run and
commit together. Any failure rolls the whole transaction back, leaving
the store at `format_version = 1`.

## Migration 2 → 3

A store opened writable at `format_version = 2` is migrated in place to
`3` by executing exactly these statements in a single transaction:

```sql
ALTER TABLE items ADD COLUMN scope TEXT;
CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;
UPDATE singularmem_meta SET value = '3' WHERE key = 'format_version';
```

Transactional shape is identical to Migration 1 → 2: `BEGIN IMMEDIATE`,
re-check `format_version` after acquiring the lock (no-op if the store
already moved past `2`), apply the statements, commit; any failure rolls
back and leaves the store at `format_version = 2`.

A store found at `format_version = 1` is migrated through the full chain
— `1 → 2`, then `2 → 3` — as two migrations run back to back by the
loader, not a single combined transaction.

A **read-only** open of a store still at `format_version = 1` or `2` MUST
NOT migrate (a read-only connection cannot take a write lock) — it fails
with a migration-required error. The store must be opened writable at
least once to migrate; after that, subsequent read-only opens succeed
against the now-`3` store.

## In-place mutation: the two sanctioned `UPDATE`s

Every other row in `items` is append-only once inserted — this format
allows exactly two deliberate exceptions, both performed by the reference
implementation:

1. **`external_id` transfer** — `Store::ingest_replacing` moves an
   `external_id` from a superseded item to its successor. Inside a single
   transaction:

   ```sql
   UPDATE items SET external_id = NULL WHERE id = <old>;
   -- then, in the same transaction:
   INSERT INTO items (id, content, created_at, supersedes, source, metadata, external_id, scope)
   VALUES (<new>, ..., <old>, ..., ..., <external_id>, ...);
   ```

   The successor row carries `supersedes = <old>` and the `external_id`
   the old row just gave up.

2. **`set_scope`** — available from v0.18.0 (this format version).
   `Store::set_scope` reassigns an item's scope after ingest:

   ```sql
   UPDATE items SET scope = ? WHERE id = ?;
   ```

   `set_scope` updates only the SQLite row; it does not touch the Tantivy
   sidecar. The lexical index keeps indexing the item under its old scope
   until `singularmem reindex` runs, so between a `set_scope` call and the
   next reindex a hybrid search may rank the item under its old scope on
   the lexical side while the semantic side (post-filtered against the
   store) already sees the new one.

A third-party loader that only ever reads should treat both as part of
the normal supersedes chain and column semantics respectively —
`get_by_external_id` always resolves to the current holder of an id, and
the old item remains in the store (readable by its own `id`, just with
`external_id = NULL`); a scope change is simply the row's current value
of `items.scope`.

## Export format — `export-v1`

The `singularmem export` CLI verb (and `Store::export` library method)
emit JSONL on stdout. Format:

```jsonl
{"_singularmem_format":"export-v1","_kind":"meta","store_format_version":"3","exported_at":"2026-05-16T12:34:56.000000000Z"}
{"_kind":"item","id":"01J...","content":"...","created_at":"2026-05-16T...","tags":["work","decision"],"metadata":{"project":"alpha"}}
{"_kind":"item","id":"01J...","content":"...","created_at":"...","supersedes":"01J...","source":"claude-conversation:abc","external_id":"file:/a.rs","scope":"claude-code/singularmem"}
```

Rules:

- The first line is always a meta record naming the format
  (`"_singularmem_format":"export-v1"`); the export format itself is
  unversioned by the store's `format_version` — only `store_format_version`
  inside the meta line changes, and it is now `"3"`.
- Each subsequent line is one item, encoded as a single-line JSON object.
- UTF-8 throughout. Unix line endings (`\n`). No trailing comma.
- Items are emitted in `created_at` ascending order. Given a
  deterministic store, the export is byte-identical across runs.
- Only `_kind`, `id`, `content` and `created_at` are always present.
- `supersedes`, `source`, `tags`, `metadata`, `external_id`, and `scope`
  are **omitted** when they carry no information: `supersedes`, `source`,
  `external_id` and `scope` when null, `tags` when the item has no tags,
  `metadata` when it is the empty object. A reader MUST treat an absent
  field as that empty value (`null`, `[]`, `{}`) rather than as an error
  — the first item line above has no `supersedes` and no `source`, the
  second has no `tags` and no `metadata`.
- `external_id` and `scope` are therefore present only on items that have
  one; readers of `export-v1` written by a v1 or v2 store simply never
  see those fields, so these are backward-compatible additions.
- Object key order in `metadata` follows insertion order from v0.18.0
  (previously alphabetical); loaders must not depend on key order.

## Known limitations

Two gaps in the bulk-ingest path are documented here rather than
papered over. Both are tracked for sub-project 12.

1. **Superseded items stay in the search indexes until `reindex`.**
   `IndexHook` has an `on_ingest` but no removal path, so when
   `ingest_replacing` supersedes an item the old document remains in the
   Tantivy and `USearch` sidecars. Repeated `ingest-dir` runs over a
   changing tree therefore accumulate stale search hits pointing at
   superseded revisions. `singularmem reindex` rebuilds the sidecars
   from `SQLite` and clears them.

2. **A change in chunk count orphans the previous items.** The
   `external_id` for a file is `file:<path>` when it produces exactly one
   chunk and `file:<path>#<n>` when it produces several. If an edit moves
   a file across that boundary — or changes how many chunks it splits
   into — the new items carry ids the store has never seen, so they are
   inserted fresh and the previous item(s) are orphaned (still present,
   still holding their old ids) rather than superseded.

## Writing a third-party loader (walkthrough)

1. Open the SQLite file.
2. Read `singularmem_meta.format_version`. If not present, the file is
   not a Singularmem store. Accept `"1"`, `"2"`, or `"3"`; refuse anything
   else — see the migration ratchet above. When the value is `"2"` or
   `"3"`, the `items.external_id` column exists (and the associated
   partial unique index); when `"3"`, `items.scope` also exists (and its
   partial index); when `"1"`, neither exists.
3. To list items, `SELECT id, content, created_at, supersedes, source,
   metadata FROM items ORDER BY created_at ASC` (add `external_id` to the
   column list at `format_version = 2` or higher; add `scope` at
   `format_version = 3`).
4. For each item, fetch its tags: `SELECT tag FROM item_tags WHERE item_id
   = ? ORDER BY tag ASC`.
5. To follow a supersedes chain, recursively `SELECT supersedes FROM
   items WHERE id = ?` from a starting ID.
6. Parse `metadata` as JSON. The validity is guaranteed by the schema's
   `CHECK` constraint.

A loader that follows these steps interoperates with any Singularmem
store at `format_version = 1`, `2`, or `3` regardless of which binary
wrote it.

## Tantivy sidecar index (optional, format unstable across Tantivy versions)

Singularmem v0.2.0+ creates an optional Tantivy index in a sidecar
directory next to the SQLite store. The sidecar is **additive** — it does
NOT bump `format_version` and a third-party loader that only reads SQLite
is unaffected by its presence or absence.

### Path convention

Default: `<store_path>.tantivy/` (e.g. `store.db.tantivy/`).
Configurable via `StoreOptions.index_path` in the Rust library; the CLI's
`--store PATH` flag implies `PATH.tantivy/` and there is no separate
override at v0.2.0.

### Schema (Tantivy 0.22.1), sidecar schema v0.3.0

| Field name        | Type     | Options                  | Purpose |
|-------------------|----------|--------------------------|---------|
| `content`         | text     | TEXT + STORED            | Searchable item text; default-search field. |
| `tags`            | text     | STRING + STORED          | Exact-match tag queries via `tags:value`. |
| `source`          | text     | TEXT + STORED            | Tokenized provenance label; default-search field. |
| `id`              | text     | STRING + STORED          | ULID for hit→Item lookup. |
| `created_at`      | date     | INDEXED + STORED + FAST  | Range filtering (reserved for v0.3+). |
| `supersedes`      | text     | STRING + STORED          | Revision pointer (reserved for v0.3+). |
| `scope`           | text     | STRING + STORED          | The item's own scope path; backs an exact scope filter. |
| `scope_ancestors` | text     | STRING                   | Multi-valued: one value per prefix of `scope` (`a`, `a/b`, `a/b/c` for scope `a/b/c`); backs a descendant-inclusive scope filter as a single term lookup. Not stored — derivable from `scope`. |

Unscoped items carry neither field. A scope filter is therefore also an
"is scoped" filter: an item with no scope never matches one.

`metadata` is intentionally NOT indexed.

### Schema version and pre-v0.18.0 sidecars

The `scope` and `scope_ancestors` fields were added in v0.18.0 (sidecar
schema v0.3.0). Tantivy refuses to open a directory whose stored schema
differs from the one supplied, so a sidecar written by an earlier release
fails to open with `Error::IndexSchemaMismatch { path }`; the message
directs the user to `singularmem reindex`, which rebuilds the sidecar from
`SQLite`. There is no in-place migration — the sidecar is a derived
artefact and rebuilding is always safe.

### Rebuild from SQLite

The Tantivy sidecar can be deleted at any time. The next `Store::open_with_hook`
auto-rebuilds it from a full SQLite iteration on first ingest (one-time
cost), or the user can run `singularmem reindex` to rebuild ahead of time.

### Tantivy on-disk format compatibility

The Tantivy index directory's on-disk format is owned by the Tantivy
project (`tantivy = 0.22.1` in v0.2.0). The format is NOT guaranteed
stable across Tantivy major version bumps; a future Singularmem release
that upgrades Tantivy may require `singularmem reindex` (or auto-trigger
one) on first open. See Tantivy's upstream documentation for the
canonical format reference.

## USearch vector sidecar (optional, format unstable across USearch versions)

Singularmem v0.3.0+ creates an optional USearch vector index in a sidecar
directory next to the SQLite store. Like the Tantivy sidecar, this is
**additive** — it does NOT bump `format_version` and a third-party loader
that only reads SQLite is unaffected by its presence or absence. The vector
sidecar is **opt-in**: it is only created when the user runs
`singularmem reindex --with-embeddings`.

### Directory layout

```
<store_path>.vectors/          ← sidecar root (e.g. store.db.vectors/)
├── .meta.json                 ← VectorIndexMeta (JSON, stable schema)
├── index.usearch              ← USearch HNSW graph (binary, version-pinned)
└── keymap.bin                 ← BTreeMap<u64, ItemId> forward map (bincode)
```

The path convention is `<store_path>.vectors/` (e.g. if the store is at
`/data/store.db`, the vector sidecar is at `/data/store.db.vectors/`).

### `.meta.json` — VectorIndexMeta schema

The metadata file is a single JSON object with the following fields:

```json
{
  "format_version": "1",
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
| `format_version` | `"1"` (string) | Metadata schema version. Currently always `"1"`. |
| `model_id` | string | Stable embedding model identifier, e.g. `"sentence-transformers/all-MiniLM-L6-v2@v1"`. The `@v1` suffix anchors to a specific weight revision; future weight updates use a new suffix and trigger a reindex prompt. |
| `dim` | integer | Embedding dimension. Must match the model's output dimension (e.g. 384 for all-MiniLM-L6-v2). |
| `distance` | `"cosine"` | Distance metric used in the HNSW graph. Currently always `"cosine"`. |
| `hnsw_m` | integer | HNSW connectivity parameter M. Default: 16. Higher values improve recall at the cost of build time and memory. |
| `hnsw_ef_construction` | integer | HNSW ef parameter during construction. Default: 128. Higher values improve recall at the cost of build time. |
| `created_at` | RFC 3339 | Wall-clock time the sidecar was first created. |

When opening an existing sidecar, Singularmem reads `.meta.json` and
compares `model_id` and `dim` against the current embedder. If either
differs, `Error::ModelMismatch` is returned and the user must run
`singularmem reindex --with-embeddings --reset-vectors --force` to rebuild.

### `keymap.bin` — forward keymap schema

`keymap.bin` is a [bincode](https://docs.rs/bincode/1/) serialisation of
the `Keymap` struct, which contains a `BTreeMap<u64, ItemId>` (forward map:
USearch key → ULID) and a parallel reverse map. The canonical persisted
shape (the one a third-party loader needs to read) is the forward map only:

```
BTreeMap<u64, ItemId>
  key   — sequential u64 assigned at insertion time, starting at 0.
  value — 26-character ULID string (Crockford base32, uppercase).
```

Bincode encoding: little-endian, variable-length integers disabled (bincode
1.x defaults). The map is preceded by its length as a `u64` element count,
followed by `(u64_key, [u8; 26])` pairs in ascending key order.

A third-party loader that only needs to translate USearch result keys to
item IDs can deserialise the forward map with any bincode 1.x-compatible
library.

### HNSW parameters (v0.3.0 defaults)

| Parameter | Value | Notes |
|---|---|---|
| `hnsw_m` | 16 | Connectivity. Increase to 32–64 for higher recall on large collections. |
| `hnsw_ef_construction` | 128 | Build-time ef. Increase to 256 for higher recall at slower build. |
| `expansion_search` | 64 | Query-time ef. Increase to 128 for higher recall at ~2× query time. |
| Distance metric | Cosine | Vectors are L2-normalised before insertion; cosine similarity = dot product. |
| Scalar type | f32 | 32-bit floats. |

### USearch version pin and upgrade path

The `index.usearch` binary format is owned by the USearch project and is
**NOT guaranteed stable across USearch major or minor version bumps**.
Singularmem v0.3.0 pins `usearch = "=2.15.3"`. If a future Singularmem
release upgrades USearch (e.g. to `=3.x`), the binary format may change
and existing `index.usearch` files will not load correctly.

**Version-bump → reindex requirement:** After a Singularmem upgrade that
includes a USearch version bump, run:

```bash
singularmem reindex --with-embeddings --reset-vectors --force
```

This deletes the existing `index.usearch` (and `keymap.bin`) and rebuilds
from SQLite using the new USearch library. The `.meta.json` is rewritten
with the same `model_id` (assuming the embedding model was not also
changed). If both USearch and the embedding model change simultaneously,
use the same command — `--reset-vectors` clears the entire sidecar
directory.

### Writing a third-party vector loader

A third-party tool that wants to read Singularmem's vector index without
linking against the Singularmem crate can follow these steps:

1. Confirm `<store_path>.vectors/.meta.json` exists. If absent, the store
   has no vector sidecar (opt-in feature not activated).
2. Read `.meta.json`. Validate `format_version == "1"`. Note `model_id`,
   `dim`, and `distance`.
3. Read `keymap.bin` with a bincode 1.x deserialiser as
   `BTreeMap<u64, String>` (the value is the ULID string, 26 ASCII bytes).
4. Open `index.usearch` with USearch `=2.15.3` (or the version in the
   Singularmem release you are targeting). Construct an index with the same
   `dim` and `distance` as in `.meta.json`, then call `index.load(path)`.
5. Issue KNN queries: `index.search(query_vector, k)` returns `(keys, distances)`.
   Translate `key → ItemId` via the forward keymap from step 3.
6. Look up the full item in SQLite using the `ItemId` (see the "Writing a
   third-party loader" section above for the SQLite walkthrough).

**Important:** `index.usearch` was written by USearch `=2.15.3`. Using a
different USearch version to open it may segfault or return corrupt data.
If you need to load the data with a different version, re-embed from SQLite
and build a new index.
