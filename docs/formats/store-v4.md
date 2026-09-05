# Singularmem Store Format — v4

This document specifies the on-disk format of a Singularmem memory store
at `format_version = 4`. **A third-party tool that reads this document and
has access to a SQLite library can write a complete loader without
referencing any Singularmem source code.** That property is a
constitutional requirement (Principle III.b).

Format v4 adds the temporal knowledge graph — the `entities` and `facts`
tables — on top of everything `items`/`item_tags` already provided. The
graph is append-only: a fact is never mutated after insertion, only
superseded by a new revision (see "Revisions and the two time axes"
below). `facts.scope` is the only scope on the graph — entities are
store-global, so the same entity name is one node everywhere.

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

### Graph tables

Added in `format_version = 4`. DDL verbatim from `crates/singularmem-core/src/schema.rs`:

```sql
CREATE TABLE entities (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    normalised_name  TEXT NOT NULL,
    kind             TEXT,
    created_at       TEXT NOT NULL,
    CHECK (length(name) > 0)
) STRICT;
CREATE UNIQUE INDEX idx_entities_identity ON entities(normalised_name);

CREATE TABLE facts (
    id              TEXT PRIMARY KEY NOT NULL,
    subject_id      TEXT NOT NULL,
    predicate       TEXT NOT NULL,
    object_id       TEXT,
    object_value    TEXT,
    valid_from      TEXT,
    valid_to        TEXT,
    confidence      REAL NOT NULL DEFAULT 1.0,
    source_item_id  TEXT,
    scope           TEXT,
    supersedes      TEXT,
    recorded_at     TEXT NOT NULL,
    FOREIGN KEY (subject_id) REFERENCES entities(id),
    FOREIGN KEY (object_id) REFERENCES entities(id),
    FOREIGN KEY (source_item_id) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (supersedes) REFERENCES facts(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((object_id IS NULL) <> (object_value IS NULL)),
    CHECK (confidence >= 0.0 AND confidence <= 1.0),
    CHECK (valid_to IS NULL OR valid_from IS NULL OR valid_to >= valid_from)
) STRICT;
CREATE INDEX idx_facts_subject   ON facts(subject_id);
CREATE INDEX idx_facts_object    ON facts(object_id) WHERE object_id IS NOT NULL;
CREATE INDEX idx_facts_predicate ON facts(predicate);
CREATE INDEX idx_facts_supersedes ON facts(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_facts_scope     ON facts(scope) WHERE scope IS NOT NULL;
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

**`entities.id`** — 26-character ULID (same shape as `items.id`), minted
when the entity is first created.

**`entities.name`** — UTF-8 text, non-empty (`CHECK (length(name) > 0)`).
The display form: the name exactly as first written.

**`entities.normalised_name`** — The entity's identity. Two writes that
normalise to the same string resolve to one row; `idx_entities_identity`
is a unique index on this column, so it is impossible for two entities to
share a normalised name. Entities are **store-global**: there is no scope
column here, so `tantivy` written under any scope is the same node.
Normalisation (`singularmem_core::graph::normalise::entity_name`): NFC,
trim, lowercase, internal whitespace runs collapsed to a single `_`,
apostrophes (`'`) stripped; the result must be 1–256 bytes, else
`Error::Validation { field: "entity" }` and nothing is written.

**`entities.kind`** — Nullable free-form text. Set on first creation
(e.g. `--subject-kind`/`--object-kind`) and immutable afterwards: a later
write that names a *different* kind for an existing entity is
`Error::Validation { field: "kind" }`; a later write that omits the kind
is accepted and leaves the stored kind unchanged. Entities are never
deleted.

**`entities.created_at`** — RFC 3339 timestamp, same shape as
`items.created_at`.

**`facts.id`** — 26-character ULID. Identifies one specific *revision*,
not a stable "fact slot" — see "Revisions and the two time axes" below.

**`facts.subject_id`** / **`facts.object_id`** — Foreign keys into
`entities.id`. `object_id` is `NULL` when the fact's object is a literal
value rather than another entity (see `object_value` below); the `CHECK`
constraint enforces that exactly one of `object_id`/`object_value` is
set.

**`facts.predicate`** — Normalised text identifying the relationship,
e.g. `uses`. Normalisation
(`singularmem_core::graph::normalise::predicate`) applies the same rule
as entity names (NFC, trim, lowercase, whitespace runs → `_`, apostrophes
stripped) plus the additional constraint that the result matches
`[a-z0-9_]+`, 1–64 bytes; anything else is
`Error::Validation { field: "predicate" }`.

**`facts.object_value`** — Nullable text: the fact's object when it is a
literal value, not an entity. Stored trimmed, so surrounding whitespace on
one write cannot fork a triple's identity from another's.

**`facts.valid_from`** / **`facts.valid_to`** — Nullable RFC 3339
timestamps bounding the fact's *validity window* (not when the row was
written — that is `recorded_at`). `NULL valid_from` means "since
unknown"; `NULL valid_to` means "still valid" (open). Input may be a bare
date (`2026-05-16`, expanded to `T00:00:00Z`) or a full timestamp; stored
canonical. `CHECK (valid_to IS NULL OR valid_from IS NULL OR valid_to >=
valid_from)` rejects an inverted window at the database level; the
application also rejects it before ever preparing the statement.

**`facts.confidence`** — `REAL`, `[0.0, 1.0]` (`CHECK` and
application-level validation both enforce the range), default `1.0`.

**`facts.source_item_id`** — Nullable foreign key into `items.id`
(`DEFERRABLE INITIALLY DEFERRED`), the item this fact was extracted from,
if any. A write naming an item that does not exist in the store is
`Error::Validation { field: "source_item_id" }`.

**`facts.scope`** — Nullable hierarchical scope path, same validation and
normalisation as `items.scope`. This is the **only** scope on the graph:
entities are store-global (no `entities.scope` column), so scope narrows
which *facts* a query sees, never which entity a name resolves to.

**`facts.supersedes`** — Nullable foreign key into `facts.id`
(`DEFERRABLE INITIALLY DEFERRED`). Non-null on every revision except the
first in a chain; see "Revisions and the two time axes" below.

**`facts.recorded_at`** — RFC 3339 timestamp: when this specific revision
was appended (append time — the second of the two time axes, distinct
from `valid_from`/`valid_to`).

### Revisions and the two time axes

A fact's **chain** is the sequence of rows linked by `supersedes`; the
**head** of a chain is its newest revision — the one no other row's
`supersedes` points at. `invalidate`/`supersede` never modify an existing
row: they append a new one pointing `supersedes` at the row they close.
Every read in the reference implementation builds its `WHERE` clause from
one shared predicate (`fact_where` in
`crates/singularmem-core/src/graph/read.rs`) so these rules live in
exactly one place; the SQL fragments below are copied from it verbatim.

**Head** (no `recorded_at` given — "believed now"):

```sql
(NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id))
```

**Believed at `R`** (`--recorded-at R`): for each chain, the newest
revision recorded at or before `R`; chains whose first revision is after
`R` are invisible:

```sql
(f.recorded_at <= ? AND NOT EXISTS
 (SELECT 1 FROM facts g WHERE g.supersedes = f.id AND g.recorded_at <= ?))
```

(`?` is bound to `R` both times.)

**Open fact** (default, no `--as-of`): a head with `valid_to IS NULL`:

```sql
(f.valid_to IS NULL)
```

**Valid at `T`** (`--as-of T`): a head matching the half-open window
`[valid_from, valid_to)`, with `NULL` meaning "since unknown" /
"still valid":

```sql
((f.valid_from IS NULL OR f.valid_from <= ?)
 AND (f.valid_to IS NULL OR ? < f.valid_to))
```

(`?` is bound to `T` both times.) A revision whose `valid_from` is `NULL`
is valid at *any* instant before its `valid_to` — `NULL` means "since
unknown", not "not yet", so it satisfies `valid_from IS NULL` regardless
of how far back `T` reaches.

The head clause and the open/as-of clause are combined with `AND`;
`--recorded-at` replaces the head clause with the believed-at-`R` form
above, and — if `--as-of` is *also* given — the as-of clause still
applies on top of it, otherwise the open-fact clause does. A `facts.scope`
filter (same descendant-inclusive/exact-match shapes as `items.scope`,
rebound to the `facts` table) narrows further, `AND`-joined with whichever
of the above applies.

### `singularmem_meta` key registry

| Key | Type | Required? | Purpose |
|---|---|---|---|
| `format_version` | string (`"4"`) | yes | Format version marker. Loaders MUST refuse to operate on a value they do not recognise. |
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
supports maximum version `4` from v0.19.0 onward.

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

## Migration 3 → 4

A store opened writable at `format_version = 3` is migrated in place to
`4` by executing exactly the "Graph tables" DDL above (`CREATE TABLE
entities`, its unique index, `CREATE TABLE facts`, and its four indexes)
followed by:

```sql
UPDATE singularmem_meta SET value = '4' WHERE key = 'format_version';
```

all in a single transaction. Transactional shape is identical to
Migrations 1 → 2 and 2 → 3: `BEGIN IMMEDIATE` (taking the write lock up
front), then `format_version` is re-read immediately after the lock is
acquired — if a concurrent writer already moved the store past `3` while
this connection was waiting, the migration is a no-op (rolled back,
nothing to commit, `Ok(())` returned); otherwise the DDL and the meta
update run and commit together. Any failure (e.g. a pre-existing object
with a colliding name) rolls the whole transaction back, leaving the
store at `format_version = 3` with neither `entities` nor `facts`
present.

A store found at `format_version = 1` is migrated through the full chain
— `1 → 2`, then `2 → 3`, then `3 → 4` — as three migrations run back to
back by the loader, each its own transaction.

A **read-only** open of a store still at `format_version = 1`, `2`, or `3`
MUST NOT migrate (a read-only connection cannot take a write lock) — it
fails with a migration-required error. The store must be opened writable
at least once to migrate; after that, subsequent read-only opens succeed
against the now-`4` store.

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

`entities` and `facts` add no further sanctioned `UPDATE`s: `entities`
gets exactly one (`kind`, filling in a previously-`NULL` value — see
"`entities.kind`" above) and `facts` gets none at all. Every fact is
append-only, full stop: `invalidate`/`supersede` always `INSERT` a new
revision, never touch an existing row. A loader can therefore treat every
`facts` row as immutable once observed.

## Export format — `export-v2`

The `singularmem export` CLI verb (and `Store::export` library method)
emit JSONL on stdout. Format:

```jsonl
{"_singularmem_format":"export-v2","_kind":"meta","store_format_version":"4","exported_at":"2026-09-05T12:34:56.000000000Z"}
{"_kind":"item","id":"01J...","content":"...","created_at":"2026-05-16T...","tags":["work","decision"],"metadata":{"project":"alpha"}}
{"_kind":"item","id":"01J...","content":"...","created_at":"...","supersedes":"01J...","source":"claude-conversation:abc","external_id":"file:/a.rs","scope":"claude-code/singularmem"}
{"_kind":"entity","id":"01J...","name":"singularmem","normalised_name":"singularmem","created_at":"2026-09-05T12:00:00.000000000Z"}
{"_kind":"entity","id":"01J...","name":"tantivy","normalised_name":"tantivy","kind":"crate","created_at":"2026-09-05T12:00:00.000000000Z"}
{"_kind":"fact","id":"01J...","subject":{"id":"01J...","name":"singularmem"},"predicate":"uses","object":{"entity":{"id":"01J...","name":"tantivy"}},"confidence":1.0,"scope":"claude-code/singularmem","recorded_at":"2026-09-05T12:00:00.000000000Z"}
{"_kind":"fact","id":"01J...","subject":{"id":"01J...","name":"singularmem"},"predicate":"confidence_note","object":{"value":"battle-tested"},"confidence":0.9,"source_item_id":"01J...","recorded_at":"2026-09-05T12:01:00.000000000Z"}
```

Rules:

- The first line is always a meta record naming the format
  (`"_singularmem_format":"export-v2"`); the export format itself is
  unversioned by the store's `format_version` — only `store_format_version`
  inside the meta line changes, and it is now `"4"`.
- Line kinds, in order: `meta` (exactly one), `item` (zero or more),
  `entity` (zero or more), `fact` (zero or more — one line per fact
  **revision**, not one per chain: an invalidated or superseded fact
  contributes every revision in its chain, each as its own line).
- **Loaders MUST ignore any `_kind` they do not recognise.** This is how
  the format grows: a v1-style loader that only understands `meta`/`item`
  still reads every item out of an `export-v2` file correctly, simply
  skipping the `entity` and `fact` lines it does not understand.
- UTF-8 throughout. Unix line endings (`\n`). No trailing comma.
- Items are emitted in `created_at` ascending order; entities in
  `created_at` ascending order, then `id`; fact revisions in
  `recorded_at` ascending order, then `id`. Given a deterministic store,
  the export is byte-identical across runs (modulo `exported_at`).
- Item shape is unchanged from `export-v1`; see its field-omission rules
  above (`supersedes`, `source`, `tags`, `metadata`, `external_id`,
  `scope` are omitted when they carry no information).
- **Entity line** fields: `_kind` (`"entity"`), `id`, `name`,
  `normalised_name`, `created_at` are always present; `kind` is
  **omitted** when the entity has none (the first entity line above has
  no `kind`, the second does).
- **Fact line** fields: `_kind` (`"fact"`), `id`, `subject` (`{"id",
  "name"}`), `predicate`, `object`, `confidence`, `recorded_at` are
  always present. `object` is `{"entity": {"id", "name"}}` when the
  object is another entity, or `{"value": "<text>"}` when it is a literal
  — exactly one of the two keys is present. `valid_from`, `valid_to`,
  `source_item_id`, `scope`, and `supersedes` are **omitted** when null;
  a reader MUST treat an absent field as `null` rather than as an error.
  A revision with `supersedes` present is a closing or replacing
  revision, not the first in its chain.
- Object key order in `metadata` follows insertion order from v0.18.0
  (previously alphabetical); loaders must not depend on key order.
- A store with no facts (e.g. one migrated from v1–v3 that has never
  called into the graph) exports zero `entity` and zero `fact` lines —
  `export-v2` degrades to exactly the `export-v1` shape plus a bumped
  `store_format_version`.

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
   not a Singularmem store. Accept `"1"`, `"2"`, `"3"`, or `"4"`; refuse
   anything else — see the migration ratchet above. When the value is
   `"2"` or higher, `items.external_id` exists (and its partial unique
   index); when `"3"` or higher, `items.scope` also exists (and its
   partial index); when `"4"`, `entities` and `facts` also exist; when
   `"1"`, none of these do.
3. To list items, `SELECT id, content, created_at, supersedes, source,
   metadata FROM items ORDER BY created_at ASC` (add `external_id` to the
   column list at `format_version = 2` or higher; add `scope` at
   `format_version = 3` or higher).
4. For each item, fetch its tags: `SELECT tag FROM item_tags WHERE item_id
   = ? ORDER BY tag ASC`.
5. To follow a supersedes chain, recursively `SELECT supersedes FROM
   items WHERE id = ?` from a starting ID.
6. Parse `metadata` as JSON. The validity is guaranteed by the schema's
   `CHECK` constraint.

At `format_version = 4`, the graph adds:

7. To list entities: `SELECT id, name, normalised_name, kind, created_at
   FROM entities ORDER BY created_at ASC, id ASC`.
8. To read the **currently open facts** (the common case — "what does the
   store believe right now"):

   ```sql
   SELECT f.id, s.id, s.name, f.predicate, f.object_id, o.name, f.object_value,
          f.valid_from, f.valid_to, f.confidence, f.source_item_id,
          f.scope, f.supersedes, f.recorded_at
   FROM facts f
   JOIN entities s ON f.subject_id = s.id
   LEFT JOIN entities o ON f.object_id = o.id
   WHERE (NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id))
     AND (f.valid_to IS NULL)
   ```

   `f.object_id`/`o.name` are `NULL` when the fact's object is a literal
   value — read `f.object_value` instead; the schema's `CHECK` guarantees
   exactly one of the two is set. See "Revisions and the two time axes" above for
   the as-of and recorded-at variants of the `WHERE` clause, and to
   understand why a fact's `id` identifies one revision, not a stable
   "fact slot".
9. To walk one fact's full history: start from any revision's `id`,
   follow `supersedes` backward (`SELECT supersedes FROM facts WHERE id =
   ?`) to the root, and forward (`SELECT id FROM facts WHERE supersedes =
   ? ORDER BY recorded_at ASC, id ASC`) to the newest revision. More than
   one row supersedes the same one only in a hand-edited or
   externally-written store — the reference implementation's own writes
   never fork a chain.

A loader that follows these steps interoperates with any Singularmem
store at `format_version = 1` through `4` regardless of which binary
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
