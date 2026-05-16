# Singularmem Store Format — v1

This document specifies the on-disk format of a Singularmem memory store
at `format_version = 1`. **A third-party tool that reads this document and
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
    id          TEXT PRIMARY KEY NOT NULL,
    content     TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    supersedes  TEXT,
    source      TEXT,
    metadata    TEXT NOT NULL DEFAULT '{}',
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

**`item_tags.tag`** — Free-form text, ≤ 64 bytes, no `\0`. Tags are
stored case-sensitively. The `(item_id, tag)` primary key dedupes within
an item.

### `singularmem_meta` key registry

| Key | Type | Required? | Purpose |
|---|---|---|---|
| `format_version` | string (`"1"`) | yes | Format version marker. Loaders MUST refuse to operate on a value they do not recognise. |
| `created_at` | RFC 3339 | yes | Wall-clock time the store file was first created. |

Future format versions may add keys; readers MUST ignore unknown keys
within their own format version.

## Migration ratchet

A store at `format_version = N` that is opened by a binary supporting
maximum version `M`:

- `N == M` → open succeeds, no migration.
- `N < M` → loader runs migrators `N → N+1 → ... → M` in a single
  transaction; failure rolls back and surfaces the original `N`.
- `N > M` → loader MUST refuse with an "unsupported format version"
  error. It MUST NOT attempt to operate on a newer format.

The Singularmem reference implementation in `crates/singularmem-core` at
v0.1.0 supports maximum version `1`.

## Export format — `export-v1`

The `singularmem export` CLI verb (and `Store::export` library method)
emit JSONL on stdout. Format:

```jsonl
{"_singularmem_format":"export-v1","_kind":"meta","store_format_version":"1","exported_at":"2026-05-16T12:34:56.000000000Z"}
{"_kind":"item","id":"01J...","content":"...","created_at":"2026-05-16T...","supersedes":null,"source":null,"tags":["work","decision"],"metadata":{"project":"alpha"}}
{"_kind":"item","id":"01J...","content":"...","created_at":"...","supersedes":"01J...","source":"claude-conversation:abc","tags":[],"metadata":{}}
```

Rules:

- The first line is always a meta record naming the format
  (`"_singularmem_format":"export-v1"`).
- Each subsequent line is one item, encoded as a single-line JSON object.
- UTF-8 throughout. Unix line endings (`\n`). No trailing comma.
- Items are emitted in `created_at` ascending order. Given a
  deterministic store, the export is byte-identical across runs.
- `tags` is always present; empty array `[]` if the item has no tags.
- `metadata` is always present; empty object `{}` if the item has none.

## Writing a third-party loader (walkthrough)

1. Open the SQLite file.
2. Read `singularmem_meta.format_version`. If not present, the file is
   not a Singularmem store. If not `"1"`, refuse — see the migration
   ratchet above.
3. To list items, `SELECT id, content, created_at, supersedes, source,
   metadata FROM items ORDER BY created_at ASC`.
4. For each item, fetch its tags: `SELECT tag FROM item_tags WHERE item_id
   = ? ORDER BY tag ASC`.
5. To follow a supersedes chain, recursively `SELECT supersedes FROM
   items WHERE id = ?` from a starting ID.
6. Parse `metadata` as JSON. The validity is guaranteed by the schema's
   `CHECK` constraint.

A loader that follows these steps interoperates with any Singularmem
store at `format_version = 1` regardless of which binary wrote it.
