---
title: Transcript ingestion (Sub-project 11)
date: 2026-09-03
status: merged
sub-project: 11-transcript-ingestion
spec: ../specs/2026-09-03-transcript-ingestion-11-design.md
---

# Transcript Ingestion (Sub-project 11) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bulk, idempotent ingestion of Claude Code JSONL transcripts and source trees via two new CLI verbs, backed by store format v2 (`external_id` column) and a new `singularmem-ingest` crate.

**Architecture:** `singularmem-core` gains `external_id` on `Item`/`NewItem`, a 1→2 migration, and three store methods (`get_by_external_id`, `existing_external_ids`, `ingest_replacing`). A new crate `singularmem-ingest` defines a `Source` trait, a Claude transcript parser, a `.gitignore`-aware directory walker, a shared chunker, and an `ingest_source` driver that dedups against the store. The CLI adds `ingest-transcript` and `ingest-dir` as thin shells with the same hook wiring as `ingest`.

**Tech Stack:** Rust 1.80, rusqlite 0.32 (bundled SQLite), serde_json, `ignore` 0.4, `sha2` 0.10, jiff, proptest, assert_cmd.

## Global Constraints

- Workspace lints: `clippy::pedantic` + `clippy::nursery` at warn; CI runs `cargo clippy --all-targets --all-features -- -D warnings`. Every new file must be clippy-clean.
- `cargo fmt --all -- --check` must pass.
- No network in any test (Principle VI). Use `SINGULARMEM_TEST_EMBEDDER=mock` when a vector index is involved.
- `#![forbid(unsafe_code)]` in every new crate.
- Every commit is signed off: `git commit -s`.
- Store format v2 constant is the string `"2"`. Export format marker stays `"export-v1"`.
- `external_id` limits: non-empty, ≤ 512 bytes, no NUL.
- Chunk size default: 4096 bytes.
- Batch size for `ingest_many`: 500.
- Ingest throughput budget ≥ 50 items/s (CI `perf-budgets`).
- Branch: `transcript-ingestion-11`. One PR at the end.

## File Structure

| Path | Responsibility |
|---|---|
| `crates/singularmem-core/src/item.rs` | `external_id` field on `Item` and `NewItem`; validation of the new field |
| `crates/singularmem-core/src/schema.rs` | `DDL_V2`, `migrate_1_to_2` |
| `crates/singularmem-core/src/format.rs` | `FORMAT_VERSION = "2"` |
| `crates/singularmem-core/src/store.rs` | open path runs migration; refuses to migrate read-only |
| `crates/singularmem-core/src/error.rs` | `ExternalIdConflict`, `Migration` variants |
| `crates/singularmem-core/src/ingest.rs` | insert `external_id`; map unique violation; `ingest_replacing` |
| `crates/singularmem-core/src/query.rs` | load `external_id`; `get_by_external_id`; `existing_external_ids` |
| `crates/singularmem-core/tests/migration.rs` | v1 fixture → v2 migration test; v3 refusal |
| `crates/singularmem-core/tests/external_id.rs` | dedup, conflict, replace tests |
| `docs/formats/store-v2.md` | format spec for v2 |
| `crates/singularmem-ingest/Cargo.toml`, `src/lib.rs`, `src/error.rs` | crate scaffold, `Source` trait, error type |
| `crates/singularmem-ingest/src/chunk.rs` | `chunk_text` |
| `crates/singularmem-ingest/src/claude.rs` | `ClaudeTranscript`, `discover_transcripts` |
| `crates/singularmem-ingest/src/dir.rs` | `DirectoryWalker` |
| `crates/singularmem-ingest/src/driver.rs` | `ingest_source`, `Report` |
| `crates/singularmem-ingest/tests/fixtures/session.jsonl` | fixture covering every line type |
| `src/main.rs` | two new subcommands |
| `tests/cli.rs` | CLI integration tests |
| `.github/workflows/publish-cargo.yml` | add the new crate to publish order |

---

### Task 1: Store format v2 — `external_id` column, migration, model field

**Files:**
- Modify: `crates/singularmem-core/src/format.rs`
- Modify: `crates/singularmem-core/src/schema.rs`
- Modify: `crates/singularmem-core/src/store.rs:163-178`
- Modify: `crates/singularmem-core/src/error.rs`
- Modify: `crates/singularmem-core/src/item.rs`
- Modify: `crates/singularmem-core/src/ingest.rs`
- Modify: `crates/singularmem-core/src/query.rs:273-345`
- Modify: `crates/singularmem-search/tests/embedder_index.rs:15-23`, `crates/singularmem-search/tests/index_basics.rs:43-51`
- Modify: `crates/singularmem-node/src/types.rs:62-75,224-230,240-248,291-310`
- Create: `crates/singularmem-core/tests/migration.rs`
- Modify: `crates/singularmem-core/tests/format.rs`

**Interfaces:**
- Produces: `Item.external_id: Option<String>`, `NewItem.external_id: Option<String>`, `singularmem_core::FORMAT_VERSION == "2"`, `Error::Migration { from: String, to: &'static str, reason: String }`, `Error::ExternalIdConflict { external_id: String }`.

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none
**Blocks:** Task 2

- [ ] **Step 1: Write the failing migration test**

Create `crates/singularmem-core/tests/migration.rs`:

```rust
//! Store format 1 → 2 migration. Builds a v1 store with raw SQL (the exact
//! DDL from `docs/formats/store-v1.md`), opens it with the current binary,
//! and asserts it is upgraded in place with all data intact.

use rusqlite::Connection;
use singularmem_core::{Error, Store, StoreOptions};
use tempfile::TempDir;

const V1_DDL: &str = "
CREATE TABLE singularmem_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;
CREATE TABLE items (
    id TEXT PRIMARY KEY NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL,
    supersedes TEXT, source TEXT, metadata TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY (supersedes) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK (length(content) > 0), CHECK (length(content) <= 1048576),
    CHECK (json_valid(metadata) AND json_type(metadata) = 'object')
) STRICT;
CREATE TABLE item_tags (item_id TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (item_id, tag),
    FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE) STRICT;
CREATE INDEX idx_items_created_at ON items(created_at);
CREATE INDEX idx_items_supersedes ON items(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_item_tags_tag ON item_tags(tag);
INSERT INTO singularmem_meta VALUES ('format_version', '1');
INSERT INTO singularmem_meta VALUES ('created_at', '2026-01-01T00:00:00Z');
INSERT INTO items (id, content, created_at, source, metadata)
  VALUES ('01ARZ3NDEKTSV4RRFFQ69G5FAV', 'legacy item', '2026-01-01T00:00:01Z', 'legacy', '{\"k\":1}');
INSERT INTO item_tags VALUES ('01ARZ3NDEKTSV4RRFFQ69G5FAV', 'old');
";

fn make_v1(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("store.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(V1_DDL).unwrap();
    path
}

#[test]
fn v1_store_migrates_to_v2_on_open() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);

    let store = Store::open(&path).expect("open migrates");
    assert_eq!(store.format_version().unwrap(), "2");

    let item = store.get("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()).unwrap();
    assert_eq!(item.content, "legacy item");
    assert_eq!(item.tags, vec!["old"]);
    assert_eq!(item.external_id, None);
    drop(store);

    // Third-party loader check: the column and unique index exist.
    let conn = Connection::open(&path).unwrap();
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('items')").unwrap()
        .query_map([], |r| r.get(0)).unwrap()
        .collect::<Result<_, _>>().unwrap();
    assert!(cols.contains(&"external_id".to_string()));
    let idx: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_items_external_id'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(idx, 1);
}

#[test]
fn v1_store_read_only_refuses_to_migrate() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);
    let err = Store::open_with_options(&path, StoreOptions { read_only: true }).unwrap_err();
    assert!(matches!(err, Error::Migration { .. }), "got {err:?}");
    // Still at v1 on disk.
    let conn = Connection::open(&path).unwrap();
    let v: String = conn
        .query_row("SELECT value FROM singularmem_meta WHERE key='format_version'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "1");
}

#[test]
fn newer_store_still_refused() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE singularmem_meta SET value='3' WHERE key='format_version'", []).unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(matches!(err, Error::UnsupportedFormatVersion { .. }));
}

#[test]
fn fresh_store_is_v2_with_external_id_column() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fresh.db");
    let store = Store::open(&path).unwrap();
    assert_eq!(store.format_version().unwrap(), "2");
    let mut item = singularmem_core::NewItem::text("x");
    item.external_id = Some("test:1".into());
    let stored = store.ingest(item).unwrap();
    assert_eq!(stored.external_id.as_deref(), Some("test:1"));
    assert_eq!(store.get(stored.id).unwrap().external_id.as_deref(), Some("test:1"));
}
```

Add to `crates/singularmem-core/Cargo.toml` `[dev-dependencies]`: `rusqlite = { workspace = true }` (it is already a normal dependency, so this line is only needed if not already visible to tests — it is; skip if `cargo test` compiles without it).

- [ ] **Step 2: Run it, expect compile failure**

Run: `cargo test -p singularmem-core --test migration`
Expected: FAIL — `no field external_id on type Item`, `no variant Migration`.

- [ ] **Step 3: Format constant and error variants**

`crates/singularmem-core/src/format.rs`: change `pub const FORMAT_VERSION: &str = "1";` to `"2"` and update the doc comment to reference `docs/formats/store-v2.md`.

`crates/singularmem-core/src/error.rs`, add after `UnsupportedFormatVersion`:

```rust
    /// An in-place format migration failed or was refused. The store is left
    /// at the `from` version; nothing was partially applied.
    #[error("migrating store format {from} -> {to} failed: {reason}; store left at {from}")]
    Migration {
        /// Version found on disk.
        from: String,
        /// Version the binary tried to reach.
        to: &'static str,
        /// Why the migration could not complete.
        reason: String,
    },

    /// A `NewItem.external_id` collides with an existing item's. The new
    /// item was not persisted.
    #[error("external_id {external_id:?} already exists in store; new item was not persisted")]
    ExternalIdConflict {
        /// The colliding external id.
        external_id: String,
    },
```

- [ ] **Step 4: Schema v2 and migration**

In `crates/singularmem-core/src/schema.rs` rename `DDL_V1` → keep it, and add:

```rust
/// The full v2 DDL for a fresh store: v1 plus the nullable, unique
/// `external_id` column. Spec: `docs/formats/store-v2.md`.
const DDL_V2: &str = "
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
";

/// Migration 1 → 2. Runs in one transaction; on failure the store stays at 1.
const MIGRATE_1_TO_2: &str = "
BEGIN;
ALTER TABLE items ADD COLUMN external_id TEXT;
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
UPDATE singularmem_meta SET value = '2' WHERE key = 'format_version';
COMMIT;
";

/// Apply the 1 → 2 migration.
pub fn migrate_1_to_2(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(MIGRATE_1_TO_2).map_err(|e| {
        // execute_batch stops at the failing statement; ensure no open tx.
        let _ = conn.execute_batch("ROLLBACK;");
        Error::Migration {
            from: "1".to_string(),
            to: "2",
            reason: e.to_string(),
        }
    })
}
```

Rename `apply_v1` → `apply_current` and make it execute `DDL_V2` (keep the meta inserts). Delete `DDL_V1` if nothing else references it (grep first). Update the module doc line to "SQL DDL for `format_version = 2` and the migration runner."

- [ ] **Step 5: Store open path**

In `crates/singularmem-core/src/store.rs`, replace the read-only branch body (lines 151-161) with:

```rust
            let version =
                schema::read_format_version(&conn)?.ok_or(Error::UnsupportedFormatVersion {
                    found: "<missing>".to_string(),
                    max_supported: FORMAT_VERSION,
                })?;
            if version == "1" {
                return Err(Error::Migration {
                    from: version,
                    to: FORMAT_VERSION,
                    reason: "store is opened read-only; open it writable once to migrate".to_string(),
                });
            }
            if version != FORMAT_VERSION {
                return Err(Error::UnsupportedFormatVersion {
                    found: version,
                    max_supported: FORMAT_VERSION,
                });
            }
```

and the write branch `match` (lines 164-176) with:

```rust
            match schema::read_format_version(&conn)? {
                None => {
                    let now = clock.now().to_string();
                    schema::apply_current(&conn, &now)?;
                }
                Some(v) if v == FORMAT_VERSION => { /* already current */ }
                Some(v) if v == "1" => schema::migrate_1_to_2(&conn)?,
                Some(other) => {
                    return Err(Error::UnsupportedFormatVersion {
                        found: other,
                        max_supported: FORMAT_VERSION,
                    });
                }
            }
```

- [ ] **Step 6: Model field and validation**

In `crates/singularmem-core/src/item.rs`:

Add to `Item` after `metadata`:

```rust
    /// Caller-supplied stable identity for idempotent bulk ingest
    /// (e.g. `claude-code:<session>:<uuid>`). Unique across the store when
    /// present. `None` for items ingested without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
```

Add to `NewItem` after `metadata`:

```rust
    /// Optional stable identity, ≤ 512 bytes, unique across the store.
    pub external_id: Option<String>,
```

In `NewItem::text` add `external_id: None,`.

Add constant: `pub(crate) const MAX_EXTERNAL_ID_BYTES: usize = 512;`

In `validate`, after the `source` check:

```rust
    if let Some(ext) = &item.external_id {
        if ext.is_empty() {
            return Err(Error::Validation {
                field: "external_id",
                reason: "must be non-empty when present".to_string(),
            });
        }
        if ext.len() > MAX_EXTERNAL_ID_BYTES {
            return Err(Error::Validation {
                field: "external_id",
                reason: format!("exceeds {MAX_EXTERNAL_ID_BYTES}-byte cap (got {} bytes)", ext.len()),
            });
        }
        if ext.contains('\0') {
            return Err(Error::Validation {
                field: "external_id",
                reason: "must not contain NUL bytes".to_string(),
            });
        }
    }
```

Add unit tests in the same file's `mod tests`:

```rust
    #[test]
    fn empty_external_id_rejected() {
        let mut item = NewItem::text("hello");
        item.external_id = Some(String::new());
        assert!(matches!(validate(&item), Err(Error::Validation { field: "external_id", .. })));
    }

    #[test]
    fn long_external_id_rejected() {
        let mut item = NewItem::text("hello");
        item.external_id = Some("e".repeat(MAX_EXTERNAL_ID_BYTES + 1));
        assert!(matches!(validate(&item), Err(Error::Validation { field: "external_id", .. })));
    }
```

- [ ] **Step 7: Persist and load the column**

`crates/singularmem-core/src/ingest.rs`: in both `ingest` and `ingest_many`, change the INSERT to

```rust
            "INSERT INTO items (id, content, created_at, supersedes, source, metadata, external_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
```

with `item.external_id` (a `&Option<String>` — pass `item.external_id.as_deref()`) as `?7`. Replace the `.map_err(|e| Error::Sqlite { context: "inserting item row", ... })` on those two INSERTs with a call to a new helper:

```rust
/// Map an INSERT error: a unique violation on `external_id` becomes
/// `ExternalIdConflict`; anything else is `Sqlite`.
fn map_insert_err(e: rusqlite::Error, external_id: Option<&str>, context: &'static str) -> Error {
    if let rusqlite::Error::SqliteFailure(ffi, Some(ref msg)) = e {
        if ffi.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            && msg.contains("items.external_id")
        {
            return Error::ExternalIdConflict {
                external_id: external_id.unwrap_or_default().to_string(),
            };
        }
    }
    Error::Sqlite { context, source: e }
}
```

Every `Item { ... }` literal in `ingest.rs` (three sites) gets `external_id: item.external_id.clone()` (or moved, for the final return values). Note that in `ingest`, the tx must be rolled back before returning the conflict error: rusqlite's `Transaction` rolls back on drop, so returning `Err` is sufficient.

`crates/singularmem-core/src/query.rs` `load_item`: SELECT `content, created_at, supersedes, source, metadata, external_id`, read column 5 as `Option<String>`, and add `external_id` to the returned `Item`.

- [ ] **Step 8: Fix every other `Item` literal**

Add `external_id: None,` to the literals at:
- `crates/singularmem-search/tests/embedder_index.rs:15`
- `crates/singularmem-search/tests/index_basics.rs:43`
- `crates/singularmem-node/src/types.rs:240`, `:291`, `:304`
- `crates/singularmem-node/src/types.rs:224` (`NewItem` literal — add `external_id: None,`)

Do not add `external_id` to the napi `Item`/`NewItem` types in this sub-project (SDK exposure is a non-goal); the node `From` impl simply drops the field.

- [ ] **Step 9: Extend the open-core round-trip test**

In `crates/singularmem-core/tests/format.rs` after `correction`, add:

```rust
    let mut keyed = NewItem::text("keyed note");
    keyed.external_id = Some("test:keyed".into());
    let _keyed = store.ingest(keyed).unwrap();
```

Update `assert_eq!(originals.len(), 4)` → `5`, `lines.len() == 5` → `6`, and `store_format_version` assertion to `"2"`. Also assert `parsed_items.iter().any(|i| i.external_id.as_deref() == Some("test:keyed"))`.

- [ ] **Step 10: Run the whole workspace**

Run: `cargo test --workspace --all-targets 2>&1 | tail -20`
Expected: all green, including `migration` (4 tests). Then `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all -- --check`.

- [ ] **Step 11: Commit**

```bash
git add -A crates/singularmem-core crates/singularmem-search/tests crates/singularmem-node/src/types.rs
git commit -s -m "feat(core): store format v2 with unique external_id column and 1->2 migration"
```

---

### Task 2: Store lookups by external id and `ingest_replacing`

**Files:**
- Modify: `crates/singularmem-core/src/query.rs`
- Modify: `crates/singularmem-core/src/ingest.rs`
- Create: `crates/singularmem-core/tests/external_id.rs`
- Create: `docs/formats/store-v2.md`
- Modify: `docs/formats/store-v1.md` (add a "superseded by v2" note at the top)

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `Store::get_by_external_id(&self, external_id: &str) -> Result<Option<Item>>`
  - `Store::existing_external_ids(&self, ids: &[&str]) -> Result<HashSet<String>>`
  - `Store::ingest_replacing(&self, item: NewItem, replaces: ItemId) -> Result<Item>`

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Task 1
**Blocks:** Task 6

- [ ] **Step 1: Write failing tests**

Create `crates/singularmem-core/tests/external_id.rs`:

```rust
use std::collections::HashSet;

use singularmem_core::{Error, NewItem, Store};
use tempfile::TempDir;

fn store() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let s = Store::open(dir.path().join("s.db")).unwrap();
    (dir, s)
}

fn keyed(content: &str, ext: &str) -> NewItem {
    let mut n = NewItem::text(content);
    n.external_id = Some(ext.to_string());
    n
}

#[test]
fn duplicate_external_id_is_conflict_and_nothing_persists() {
    let (_d, s) = store();
    s.ingest(keyed("a", "x:1")).unwrap();
    let err = s.ingest(keyed("b", "x:1")).unwrap_err();
    assert!(matches!(err, Error::ExternalIdConflict { ref external_id } if external_id == "x:1"));
    assert_eq!(s.list().unwrap().count(), 1);
}

#[test]
fn bulk_conflict_rolls_back_whole_batch() {
    let (_d, s) = store();
    s.ingest(keyed("a", "x:1")).unwrap();
    let err = s.ingest_many(vec![keyed("b", "x:2"), keyed("c", "x:1")]).unwrap_err();
    assert!(matches!(err, Error::ExternalIdConflict { .. }));
    assert_eq!(s.list().unwrap().count(), 1, "x:2 must not persist");
}

#[test]
fn get_by_external_id_round_trips() {
    let (_d, s) = store();
    let a = s.ingest(keyed("a", "x:1")).unwrap();
    assert_eq!(s.get_by_external_id("x:1").unwrap().unwrap().id, a.id);
    assert!(s.get_by_external_id("nope").unwrap().is_none());
}

#[test]
fn existing_external_ids_returns_only_present() {
    let (_d, s) = store();
    s.ingest(keyed("a", "x:1")).unwrap();
    s.ingest(keyed("b", "x:2")).unwrap();
    let got = s.existing_external_ids(&["x:1", "x:3", "x:2"]).unwrap();
    let want: HashSet<String> = ["x:1", "x:2"].iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);
    assert!(s.existing_external_ids(&[]).unwrap().is_empty());
}

#[test]
fn ingest_replacing_moves_external_id_and_supersedes() {
    let (_d, s) = store();
    let old = s.ingest(keyed("v1", "file:/a.rs")).unwrap();
    let new = s.ingest_replacing(keyed("v2", "file:/a.rs"), old.id).unwrap();
    assert_eq!(new.supersedes, Some(old.id));
    assert_eq!(new.external_id.as_deref(), Some("file:/a.rs"));
    assert_eq!(s.get(old.id).unwrap().external_id, None, "old item's id is freed");
    assert_eq!(s.get_by_external_id("file:/a.rs").unwrap().unwrap().id, new.id);
    let hist = s.revision_history(new.id).unwrap();
    assert_eq!(hist.iter().map(|i| i.id).collect::<Vec<_>>(), vec![new.id, old.id]);
}

#[test]
fn ingest_replacing_unknown_target_is_supersedes_not_found() {
    let (_d, s) = store();
    let bogus = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let err = s.ingest_replacing(keyed("v2", "file:/a.rs"), bogus).unwrap_err();
    assert!(matches!(err, Error::SupersedesNotFound { .. }));
    assert_eq!(s.list().unwrap().count(), 0);
}

#[test]
fn ingest_replacing_refused_read_only() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("s.db");
    let old_id = {
        let s = Store::open(&p).unwrap();
        s.ingest(keyed("v1", "k")).unwrap().id
    };
    let ro = Store::open_with_options(&p, singularmem_core::StoreOptions { read_only: true }).unwrap();
    assert!(matches!(ro.ingest_replacing(keyed("v2", "k"), old_id), Err(Error::ReadOnly { .. })));
}
```

- [ ] **Step 2: Run, expect compile failure**

Run: `cargo test -p singularmem-core --test external_id`
Expected: FAIL — methods not found.

- [ ] **Step 3: Implement lookups in `query.rs`**

Add to `impl Store` in `crates/singularmem-core/src/query.rs`:

```rust
    /// Fetch the item carrying `external_id`, if any.
    ///
    /// # Errors
    /// Returns `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn get_by_external_id(&self, external_id: &str) -> Result<Option<Item>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let id_text: Option<String> = conn
            .query_row(
                "SELECT id FROM items WHERE external_id = ?1",
                params![external_id],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(Error::Sqlite {
                    context: "looking up external_id",
                    source: other,
                }),
            })?;
        match id_text {
            None => Ok(None),
            Some(t) => load_item(&conn, t.parse::<ItemId>()?).map(Some),
        }
    }

    /// Return the subset of `ids` that already exist as `external_id` values.
    /// One indexed point query per id.
    ///
    /// # Errors
    /// Returns `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn existing_external_ids(&self, ids: &[&str]) -> Result<HashSet<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn
            .prepare_cached("SELECT 1 FROM items WHERE external_id = ?1")
            .map_err(|e| Error::Sqlite {
                context: "preparing external_id existence query",
                source: e,
            })?;
        let mut out = HashSet::with_capacity(ids.len());
        for id in ids {
            let hit = stmt.exists(params![id]).map_err(|e| Error::Sqlite {
                context: "checking external_id existence",
                source: e,
            })?;
            if hit {
                out.insert((*id).to_string());
            }
        }
        Ok(out)
    }
```

Add `use std::collections::HashSet;` at the top.

- [ ] **Step 4: Implement `ingest_replacing` in `ingest.rs`**

```rust
    /// Ingest `item` as the successor of `replaces`, transferring
    /// `item.external_id` from the old item to the new one in the same
    /// transaction. This is the only in-place mutation the store performs
    /// (`items.external_id` on the old row is set to NULL); see
    /// `docs/formats/store-v2.md`.
    ///
    /// `item.supersedes` is overwritten with `replaces`.
    ///
    /// # Errors
    /// `Error::ReadOnly`, `Error::Validation`, `Error::SupersedesNotFound`
    /// if `replaces` is unknown, `Error::ExternalIdConflict` if the id is
    /// held by a third item, `Error::Sqlite` otherwise. On any error
    /// nothing changes.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    #[allow(clippy::significant_drop_tightening)]
    pub fn ingest_replacing(&self, mut item: NewItem, replaces: ItemId) -> Result<Item> {
        self.assert_writable("ingest_replacing")?;
        item.supersedes = Some(replaces);
        let normalised_tags = validate(&item)?;
        let now = self.clock.now();
        let id = mint_ulid(self, now)?;

        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting ingest_replacing transaction",
            source: e,
        })?;

        let freed = tx
            .execute(
                "UPDATE items SET external_id = NULL WHERE id = ?1",
                params![replaces.to_string()],
            )
            .map_err(|e| Error::Sqlite {
                context: "clearing external_id on replaced item",
                source: e,
            })?;
        if freed == 0 {
            return Err(Error::SupersedesNotFound { id: replaces });
        }

        let metadata_text = serde_json::to_string(&item.metadata).map_err(|e| Error::Json {
            context: "serialising item metadata",
            source: e,
        })?;
        let id_text = id.to_string();
        tx.execute(
            "INSERT INTO items (id, content, created_at, supersedes, source, metadata, external_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id_text,
                item.content,
                now.to_string(),
                replaces.to_string(),
                item.source,
                metadata_text,
                item.external_id.as_deref(),
            ],
        )
        .map_err(|e| map_insert_err(e, item.external_id.as_deref(), "inserting replacing item row"))?;
        for tag in &normalised_tags {
            tx.execute(
                "INSERT INTO item_tags (item_id, tag) VALUES (?1, ?2)",
                params![id_text, tag],
            )
            .map_err(|e| Error::Sqlite {
                context: "inserting replacing item tag",
                source: e,
            })?;
        }
        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing ingest_replacing transaction",
            source: e,
        })?;
        drop(conn);

        let stored = Item {
            id,
            content: item.content,
            created_at: now,
            supersedes: Some(replaces),
            tags: normalised_tags,
            source: item.source,
            metadata: item.metadata,
            external_id: item.external_id,
        };
        self.fire_hook(&stored);
        Ok(stored)
    }
```

Extract the hook-firing block that `ingest` already contains into a private helper so both use it:

```rust
    /// Run `on_ingest` + `commit` on the attached hook, warning on failure
    /// (the SQLite write is already durable; Principle VII).
    fn fire_hook(&self, item: &Item) {
        if let Some(hook) = self.hook.lock().expect("store hook mutex poisoned").as_ref() {
            if let Err(e) = hook.on_ingest(item) {
                tracing::warn!(item_id = %item.id, error = %e,
                    "IndexHook::on_ingest failed; item is durably stored in SQLite but un-searchable. Run `singularmem reindex` to recover.");
            } else if let Err(e) = hook.commit() {
                tracing::warn!(item_id = %item.id, error = %e,
                    "IndexHook::commit failed after on_ingest; item may or may not be searchable until next commit succeeds. Run `singularmem reindex` to be sure.");
            }
        }
    }
```

and replace the inline block in `ingest` with `self.fire_hook(&stored)` after building the return `Item` (build it once, fire, return it). The `ingest` function must release the connection lock (`drop(conn)`) before calling `fire_hook` — it already does implicitly because `tx.commit()` consumed the borrow, but add an explicit `drop(conn);` for clarity.

- [ ] **Step 5: Run tests**

Run: `cargo test -p singularmem-core`
Expected: PASS, including 7 tests in `external_id`.

- [ ] **Step 6: Write `docs/formats/store-v2.md`**

Copy `docs/formats/store-v1.md` to `docs/formats/store-v2.md` and edit:
- Title and every `format_version = 1` → `2`; the reference implementation line: "at v0.17.0 supports maximum version `2`".
- In the `items` DDL add `external_id TEXT,` and the `CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;` line.
- Add a "Column semantics" row: `external_id` — optional caller-supplied identity, ≤ 512 bytes, unique when present. Conventions table (copy from the spec: `claude-code:<sessionId>:<uuid>[#n]`, `file:<abs path>[#n]`).
- Add section **"Migration 1 → 2"** with the exact three statements and the rule: read-only opens of a v1 store fail with `Migration`; a writable open migrates in one transaction.
- Add section **"In-place mutation: replacing an externally-keyed item"** stating the single allowed UPDATE: `UPDATE items SET external_id = NULL WHERE id = <old>` inside the same transaction that inserts the successor (which carries the id and `supersedes = <old>`). Everything else remains append-only.
- Export section: item lines MAY carry `"external_id"`; `store_format_version` is `"2"`; `_singularmem_format` remains `export-v1`.
- Third-party loader step 2: accept `"1"` or `"2"`; when `"2"` the `external_id` column exists.

At the top of `docs/formats/store-v1.md` add: `> Superseded by [store-v2.md](store-v2.md) as of v0.17.0. Kept for readers of v1 stores.`

Update the doc-comment paths in `crates/singularmem-core/src/lib.rs`, `format.rs`, `export.rs`, `schema.rs` from `store-v1.md` to `store-v2.md`.

- [ ] **Step 7: Lint and commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`

```bash
git add crates/singularmem-core docs/formats
git commit -s -m "feat(core): external_id lookups, ingest_replacing, and store-v2 format doc"
```

---

### Task 3: `singularmem-ingest` crate scaffold and `chunk_text`

**Files:**
- Create: `crates/singularmem-ingest/Cargo.toml`
- Create: `crates/singularmem-ingest/src/lib.rs`
- Create: `crates/singularmem-ingest/src/error.rs`
- Create: `crates/singularmem-ingest/src/chunk.rs`
- Create: `crates/singularmem-ingest/tests/chunk_property.rs`
- Modify: `.github/workflows/publish-cargo.yml:14,68` (add `singularmem-ingest` after `singularmem-retrieve`)

**Interfaces:**
- Produces:
  - `pub trait Source { fn name(&self) -> String; fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_>; fn filtered_count(&self) -> usize { 0 } }`
  - `pub fn chunk_text(text: &str, max_bytes: usize) -> Vec<String>`
  - `pub const DEFAULT_CHUNK_BYTES: usize = 4096;`
  - `pub enum Error { Core(singularmem_core::Error), Io { path: PathBuf, source: std::io::Error }, Json { path: PathBuf, line: usize, source: serde_json::Error }, NotFound { path: PathBuf } }`
  - `pub type Result<T> = std::result::Result<T, Error>;`

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none (independent of Tasks 1-2 at compile level, but merge after them)
**Blocks:** Tasks 4, 5, 6

- [ ] **Step 1: Crate manifest**

`crates/singularmem-ingest/Cargo.toml`:

```toml
[package]
name = "singularmem-ingest"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Bulk, idempotent ingestion sources for Singularmem: Claude Code transcripts and source trees."

[lints]
workspace = true

[dependencies]
singularmem-core = { path = "../singularmem-core", version = "0.16.0" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
jiff = { workspace = true }
ignore = "0.4"
sha2 = "0.10"

[dev-dependencies]
tempfile = { workspace = true }
proptest = { workspace = true }
```

- [ ] **Step 2: Error type**

`crates/singularmem-ingest/src/error.rs`:

```rust
//! Error type for ingestion sources. Every variant names the file involved.

use std::path::PathBuf;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors this crate surfaces.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The store rejected or failed an operation.
    #[error(transparent)]
    Core(#[from] singularmem_core::Error),

    /// Reading a path failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path being read.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A JSONL line could not be parsed. The line is skipped.
    #[error("malformed JSON at {path}:{line}: {source}")]
    Json {
        /// The transcript file.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Underlying error.
        #[source]
        source: serde_json::Error,
    },

    /// A path given by the caller does not exist.
    #[error("path not found: {path}")]
    NotFound {
        /// The missing path.
        path: PathBuf,
    },
}
```

- [ ] **Step 3: Failing chunk tests**

`crates/singularmem-ingest/tests/chunk_property.rs`:

```rust
use proptest::prelude::*;
use singularmem_ingest::chunk_text;

proptest! {
    #[test]
    fn chunks_are_nonempty_bounded_and_reassemble(
        paras in prop::collection::vec("[a-zA-Z0-9 .,!?]{0,300}", 0..12),
        max in 16usize..512,
    ) {
        let text = paras.join("\n\n");
        let chunks = chunk_text(&text, max);
        for c in &chunks {
            prop_assert!(!c.trim().is_empty(), "empty chunk");
            prop_assert!(c.len() <= max, "chunk {} > {max}", c.len());
        }
        // Reassembly: joining chunks and re-normalising equals the normalised input.
        let norm = |s: &str| s.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n");
        let rejoined: String = chunks.join("\n\n");
        // Hard splits inside a paragraph remove no characters, so the
        // concatenation of chunks without separators must contain every
        // non-whitespace char of the input in order.
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        prop_assert_eq!(strip(&rejoined), strip(&norm(&text)));
    }
}

#[test]
fn short_text_is_one_chunk() {
    assert_eq!(chunk_text("hello world", 4096), vec!["hello world"]);
}

#[test]
fn empty_text_is_no_chunks() {
    assert!(chunk_text("   \n\n  ", 4096).is_empty());
}

#[test]
fn splits_on_blank_lines_greedily() {
    let text = "aaaa\n\nbbbb\n\ncccc";
    assert_eq!(chunk_text(text, 10), vec!["aaaa\n\nbbbb", "cccc"]);
}

#[test]
fn hard_splits_oversized_paragraph_on_char_boundary() {
    let text = "ééééé"; // 10 bytes, 2 per char
    let chunks = chunk_text(text, 5);
    assert_eq!(chunks, vec!["éé", "éé", "é"]);
}
```

- [ ] **Step 4: Run, expect failure**

Run: `cargo test -p singularmem-ingest --test chunk_property`
Expected: FAIL — crate has no `lib.rs` yet / `chunk_text` unresolved.

- [ ] **Step 5: Implement `chunk.rs` and `lib.rs`**

`crates/singularmem-ingest/src/chunk.rs`:

```rust
//! Paragraph-aware text chunking for embedding-friendly item sizes.

/// Default chunk cap in bytes.
pub const DEFAULT_CHUNK_BYTES: usize = 4096;

/// Split `text` into chunks of at most `max_bytes` bytes. Paragraphs
/// (separated by a blank line) are packed greedily; a paragraph larger than
/// `max_bytes` is hard-split at the last char boundary that fits. Chunks
/// are trimmed and never empty. Whitespace-only input yields no chunks.
#[must_use]
pub fn chunk_text(text: &str, max_bytes: usize) -> Vec<String> {
    let max_bytes = max_bytes.max(4);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        let t = current.trim();
        if !t.is_empty() {
            chunks.push(t.to_string());
        }
        current.clear();
    };

    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if para.len() > max_bytes {
            flush(&mut current, &mut chunks);
            for piece in hard_split(para, max_bytes) {
                chunks.push(piece.to_string());
            }
            continue;
        }
        let needed = if current.is_empty() { para.len() } else { current.len() + 2 + para.len() };
        if needed > max_bytes {
            flush(&mut current, &mut chunks);
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }
    flush(&mut current, &mut chunks);
    chunks
}

/// Split `s` into pieces of at most `max_bytes` bytes on char boundaries.
fn hard_split(s: &str, max_bytes: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + max_bytes).min(s.len());
        while end > start && !s.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            // A single char wider than max_bytes; take it whole.
            end = s[start..].char_indices().nth(1).map_or(s.len(), |(i, _)| start + i);
        }
        let piece = s[start..end].trim();
        if !piece.is_empty() {
            out.push(piece);
        }
        start = end;
    }
    out
}
```

`crates/singularmem-ingest/src/lib.rs`:

```rust
//! Bulk, idempotent ingestion sources for Singularmem.
//!
//! A [`Source`] yields `NewItem`s; [`ingest_source`] writes them to a
//! `Store`, skipping anything whose `external_id` is already present.
//! Spec: `docs/superpowers/specs/2026-09-03-transcript-ingestion-11-design.md`.

#![forbid(unsafe_code)]

pub mod chunk;
pub mod error;

pub use chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
pub use error::{Error, Result};

use singularmem_core::NewItem;

/// Something that can be turned into memory items.
pub trait Source {
    /// Human-readable label for progress output (usually a path).
    fn name(&self) -> String;

    /// Yield items in source order. Per-item errors are yielded as `Err`
    /// and the iterator continues.
    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_>;

    /// Number of inputs the source deliberately skipped (not errors), valid
    /// after the iterator from [`Source::items`] has been exhausted.
    fn filtered_count(&self) -> usize {
        0
    }
}
```

- [ ] **Step 6: Run tests, lint**

Run: `cargo test -p singularmem-ingest && cargo clippy -p singularmem-ingest --all-targets -- -D warnings`
Expected: PASS (5 tests).

- [ ] **Step 7: Publish order**

In `.github/workflows/publish-cargo.yml` add `#   singularmem-ingest` after the `singularmem-retrieve` comment line and `publish_one singularmem-ingest` after `publish_one singularmem-retrieve`.

- [ ] **Step 8: Commit**

```bash
git add crates/singularmem-ingest .github/workflows/publish-cargo.yml Cargo.lock
git commit -s -m "feat(ingest): new singularmem-ingest crate with Source trait and chunk_text"
```

---

### Task 4: Claude Code transcript parser

**Files:**
- Create: `crates/singularmem-ingest/src/claude.rs`
- Create: `crates/singularmem-ingest/tests/fixtures/session.jsonl`
- Create: `crates/singularmem-ingest/tests/claude_parse.rs`
- Modify: `crates/singularmem-ingest/src/lib.rs` (add `pub mod claude; pub use claude::{ClaudeTranscript, discover_transcripts};`)

**Interfaces:**
- Consumes: `Source`, `chunk_text`, `Error`.
- Produces:
  - `pub struct ClaudeTranscript { pub path: PathBuf, pub include_sidechains: bool, pub project_filter: Option<PathBuf>, .. }`
  - `ClaudeTranscript::open(path) -> Result<Self>`; `impl Source`
  - `pub fn discover_transcripts(root: impl AsRef<Path>) -> Result<Vec<PathBuf>>`
  - `pub fn strip_system_reminders(s: &str) -> String` (pub for tests)

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Task 3
**Blocks:** Task 6

- [ ] **Step 1: Fixture**

Create `crates/singularmem-ingest/tests/fixtures/session.jsonl` with exactly these 14 lines (one JSON object per line; `S` = `11111111-2222-3333-4444-555555555555`):

```
{"type":"last-prompt","lastPrompt":"x","sessionId":"11111111-2222-3333-4444-555555555555"}
{"type":"user","uuid":"u1","parentUuid":null,"sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:00.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"message":{"role":"user","content":"How do I run the tests?"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:01.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"message":{"role":"assistant","content":[{"type":"thinking","thinking":"secret"},{"type":"text","text":"Run cargo test."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}
{"type":"user","uuid":"u2","parentUuid":"a1","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:02.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}
{"type":"assistant","uuid":"a2","parentUuid":"u2","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:03.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/x"}}]}}
{"type":"assistant","uuid":"a3","parentUuid":"a2","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:04.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"message":{"role":"assistant","content":[{"type":"thinking","thinking":"only"}]}}
{"type":"user","uuid":"u3","parentUuid":"a3","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:05.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"message":{"role":"user","content":[{"type":"text","text":"<system-reminder>ignore me</system-reminder>Thanks, that worked."}]}}
{"type":"user","uuid":"u4","parentUuid":"u3","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:06.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":false,"isMeta":true,"message":{"role":"user","content":"meta line"}}
{"type":"user","uuid":"u5","parentUuid":"u4","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:07.000Z","cwd":"/home/me/proj","gitBranch":"main","isSidechain":true,"message":{"role":"user","content":"sidechain prompt"}}
{"type":"assistant","uuid":"a4","parentUuid":"u5","sessionId":"11111111-2222-3333-4444-555555555555","timestamp":"2026-09-01T10:00:08.000Z","cwd":"/home/me/other","gitBranch":"dev","isSidechain":false,"message":{"role":"assistant","content":[{"type":"text","text":"From another project."}]}}
{"type":"attachment","uuid":"x1","sessionId":"11111111-2222-3333-4444-555555555555","attachment":{"type":"file"}}
{"type":"system","uuid":"x2","sessionId":"11111111-2222-3333-4444-555555555555","content":"sys"}
{"type":"file-history-snapshot","messageId":"m","snapshot":{}}
{this is not json}
```

- [ ] **Step 2: Failing tests**

`crates/singularmem-ingest/tests/claude_parse.rs`:

```rust
use std::path::{Path, PathBuf};

use singularmem_core::NewItem;
use singularmem_ingest::{discover_transcripts, ClaudeTranscript, Source};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session.jsonl")
}

fn parse(t: &ClaudeTranscript) -> (Vec<NewItem>, usize) {
    let mut ok = Vec::new();
    let mut errs = 0;
    for r in t.items() {
        match r {
            Ok(i) => ok.push(i),
            Err(_) => errs += 1,
        }
    }
    (ok, errs)
}

#[test]
fn keeps_exactly_the_text_messages() {
    let t = ClaudeTranscript::open(fixture()).unwrap();
    let (items, errs) = parse(&t);
    let ids: Vec<&str> = items.iter().map(|i| i.external_id.as_deref().unwrap()).collect();
    assert_eq!(
        ids,
        vec![
            "claude-code:11111111-2222-3333-4444-555555555555:u1",
            "claude-code:11111111-2222-3333-4444-555555555555:a1",
            "claude-code:11111111-2222-3333-4444-555555555555:u3",
            "claude-code:11111111-2222-3333-4444-555555555555:a4",
        ]
    );
    assert_eq!(errs, 1, "the malformed line");
    // filtered: tool_result-only u2, tool_use-only a2, thinking-only a3, meta u4, sidechain u5
    assert_eq!(t.filtered_count(), 5);
}

#[test]
fn item_shape_is_as_specified() {
    let t = ClaudeTranscript::open(fixture()).unwrap();
    let (items, _) = parse(&t);
    let a1 = &items[1];
    assert_eq!(a1.content, "Run cargo test.");
    assert_eq!(a1.source.as_deref(), Some("claude-code:11111111-2222-3333-4444-555555555555"));
    assert_eq!(a1.tags, vec!["claude-code", "role:assistant", "transcript"]);
    let m = &a1.metadata;
    assert_eq!(m["session_id"], "11111111-2222-3333-4444-555555555555");
    assert_eq!(m["uuid"], "a1");
    assert_eq!(m["parent_uuid"], "u1");
    assert_eq!(m["role"], "assistant");
    assert_eq!(m["cwd"], "/home/me/proj");
    assert_eq!(m["git_branch"], "main");
    assert_eq!(m["occurred_at"], "2026-09-01T10:00:01Z");
    assert_eq!(m["tool_names"], serde_json::json!(["Bash"]));
    assert_eq!(m["chunk_index"], 0);
    assert_eq!(m["chunk_count"], 1);

    let u3 = &items[2];
    assert_eq!(u3.content, "Thanks, that worked.", "system-reminder stripped");
    assert_eq!(u3.metadata["parent_uuid"], "a3");
}

#[test]
fn sidechains_opt_in() {
    let mut t = ClaudeTranscript::open(fixture()).unwrap();
    t.include_sidechains = true;
    let (items, _) = parse(&t);
    let u5 = items.iter().find(|i| i.metadata["uuid"] == "u5").expect("sidechain kept");
    assert!(u5.tags.contains(&"sidechain".to_string()));
}

#[test]
fn project_filter_matches_cwd() {
    let mut t = ClaudeTranscript::open(fixture()).unwrap();
    t.project_filter = Some(PathBuf::from("/home/me/proj"));
    let (items, _) = parse(&t);
    assert!(items.iter().all(|i| i.metadata["cwd"] == "/home/me/proj"));
    assert_eq!(items.len(), 3);
}

#[test]
fn long_messages_are_chunked_with_suffixed_ids() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("big.jsonl");
    let para = "word ".repeat(1000); // ~5000 bytes
    let content = format!("{para}\n\n{para}");
    let line = serde_json::json!({
        "type":"user","uuid":"big","sessionId":"s","timestamp":"2026-09-01T00:00:00Z",
        "message":{"role":"user","content":content}
    });
    std::fs::write(&p, format!("{line}\n")).unwrap();
    let t = ClaudeTranscript::open(&p).unwrap();
    let (items, _) = parse(&t);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].external_id.as_deref(), Some("claude-code:s:big#0"));
    assert_eq!(items[1].external_id.as_deref(), Some("claude-code:s:big#1"));
    assert_eq!(items[1].metadata["chunk_count"], 2);
}

#[test]
fn session_id_falls_back_to_file_stem() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("abc-123.jsonl");
    std::fs::write(&p, "{\"type\":\"user\",\"uuid\":\"u\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n").unwrap();
    let t = ClaudeTranscript::open(&p).unwrap();
    let (items, _) = parse(&t);
    assert_eq!(items[0].external_id.as_deref(), Some("claude-code:abc-123:u"));
}

#[test]
fn discover_finds_jsonl_recursively_sorted() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("b")).unwrap();
    std::fs::write(dir.path().join("b/2.jsonl"), "").unwrap();
    std::fs::write(dir.path().join("1.jsonl"), "").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "").unwrap();
    let found = discover_transcripts(dir.path()).unwrap();
    assert_eq!(found, vec![dir.path().join("1.jsonl"), dir.path().join("b/2.jsonl")]);
}

#[test]
fn open_missing_path_is_not_found() {
    assert!(matches!(
        ClaudeTranscript::open("/definitely/missing.jsonl"),
        Err(singularmem_ingest::Error::NotFound { .. })
    ));
}
```

- [ ] **Step 3: Run, expect failure**

Run: `cargo test -p singularmem-ingest --test claude_parse`
Expected: FAIL — unresolved imports.

- [ ] **Step 4: Implement `claude.rs`**

```rust
//! Claude Code JSONL transcript source.
//!
//! One `NewItem` per user/assistant message that carries text. Tool
//! payloads, tool results, and thinking blocks are skipped; tool names
//! are recorded in metadata.

use std::cell::Cell;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::Source;

/// A single Claude Code session file.
#[derive(Debug)]
pub struct ClaudeTranscript {
    /// Path to the `.jsonl` file.
    pub path: PathBuf,
    /// Keep messages flagged `isSidechain` (subagent conversations).
    pub include_sidechains: bool,
    /// When set, keep only messages whose `cwd` equals this path.
    pub project_filter: Option<PathBuf>,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    parent_uuid: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    role: Option<String>,
    content: Option<serde_json::Value>,
}

impl ClaudeTranscript {
    /// Open a transcript file. Fails with `Error::NotFound` if it is missing.
    ///
    /// # Errors
    /// `Error::NotFound` when `path` does not exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(Error::NotFound { path });
        }
        Ok(Self {
            path,
            include_sidechains: false,
            project_filter: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
        })
    }

    fn file_stem(&self) -> String {
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Convert one parsed line into items, or `None` if it is filtered.
    fn line_to_items(&self, line: Line) -> Option<Vec<NewItem>> {
        if line.kind != "user" && line.kind != "assistant" {
            return None; // structural line; not counted as filtered
        }
        let role = line.kind;
        if line.is_meta.unwrap_or(false) {
            return Some(Vec::new());
        }
        let sidechain = line.is_sidechain.unwrap_or(false);
        if sidechain && !self.include_sidechains {
            return Some(Vec::new());
        }
        if let Some(filter) = &self.project_filter {
            if line.cwd.as_deref().map(Path::new) != Some(filter.as_path()) {
                return Some(Vec::new());
            }
        }
        let Some(uuid) = line.uuid else {
            tracing::warn!(path = %self.path.display(), "message line without uuid; skipped");
            return Some(Vec::new());
        };
        let msg = line.message?;
        let (text, tool_names) = extract_text(msg.content.as_ref()?);
        let text = if role == "user" { strip_system_reminders(&text) } else { text };
        let chunks = chunk_text(&text, self.chunk_bytes);
        if chunks.is_empty() {
            return Some(Vec::new());
        }
        let _ = msg.role; // role comes from `type`; message.role is informational
        let session_id = line.session_id.unwrap_or_else(|| self.file_stem());
        let occurred_at = line
            .timestamp
            .as_deref()
            .and_then(|t| t.parse::<jiff::Timestamp>().ok())
            .map(|t| t.to_string());
        let chunk_count = chunks.len();
        let items = chunks
            .into_iter()
            .enumerate()
            .map(|(i, content)| {
                let mut tags = vec!["transcript".to_string(), "claude-code".to_string(), format!("role:{role}")];
                if sidechain {
                    tags.push("sidechain".to_string());
                }
                let external_id = if chunk_count == 1 {
                    format!("claude-code:{session_id}:{uuid}")
                } else {
                    format!("claude-code:{session_id}:{uuid}#{i}")
                };
                NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("claude-code:{session_id}")),
                    metadata: serde_json::json!({
                        "session_id": session_id,
                        "uuid": uuid,
                        "parent_uuid": line.parent_uuid,
                        "role": role,
                        "cwd": line.cwd,
                        "git_branch": line.git_branch,
                        "occurred_at": occurred_at,
                        "tool_names": tool_names,
                        "chunk_index": i,
                        "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                }
            })
            .collect();
        Some(items)
    }
}

/// Concatenate text blocks and collect tool names from a message `content`.
fn extract_text(content: &serde_json::Value) -> (String, Vec<String>) {
    match content {
        serde_json::Value::String(s) => (s.clone(), Vec::new()),
        serde_json::Value::Array(blocks) => {
            let mut texts = Vec::new();
            let mut tools = Vec::new();
            for b in blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                            texts.push(t.to_string());
                        }
                    }
                    Some("tool_use") => {
                        if let Some(n) = b.get("name").and_then(|n| n.as_str()) {
                            tools.push(n.to_string());
                        }
                    }
                    _ => {}
                }
            }
            (texts.join("\n\n"), tools)
        }
        _ => (String::new(), Vec::new()),
    }
}

/// Remove every `<system-reminder>…</system-reminder>` span (an unterminated
/// opening tag removes through end of input) and trim.
#[must_use]
pub fn strip_system_reminders(s: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        match after.find(CLOSE) {
            Some(end) => rest = &after[end + CLOSE.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

impl Source for ClaudeTranscript {
    fn name(&self) -> String {
        self.path.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(source) => {
                return Box::new(std::iter::once(Err(Error::Io { path: self.path.clone(), source })));
            }
        };
        let reader = BufReader::new(file);
        let path = self.path.clone();
        let iter = reader.lines().enumerate().flat_map(move |(idx, line)| {
            let line_no = idx + 1;
            let raw = match line {
                Ok(l) => l,
                Err(source) => return vec![Err(Error::Io { path: path.clone(), source })],
            };
            if raw.trim().is_empty() {
                return Vec::new();
            }
            let parsed: Line = match serde_json::from_str(&raw) {
                Ok(p) => p,
                Err(source) => return vec![Err(Error::Json { path: path.clone(), line: line_no, source })],
            };
            match self.line_to_items(parsed) {
                None => Vec::new(),
                Some(items) => {
                    if items.is_empty() {
                        self.filtered.set(self.filtered.get() + 1);
                    }
                    items.into_iter().map(Ok).collect()
                }
            }
        });
        Box::new(iter)
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }
}

/// Recursively find `*.jsonl` files under `root`, sorted by path.
///
/// # Errors
/// `Error::NotFound` if `root` does not exist; `Error::Io` on read failure.
pub fn discover_transcripts(root: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    if !root.exists() {
        return Err(Error::NotFound { path: root.to_path_buf() });
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|source| Error::Io { path: dir.clone(), source })?;
        for entry in entries {
            let entry = entry.map_err(|source| Error::Io { path: dir.clone(), source })?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "jsonl") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}
```

Note on `line_to_items` semantics: returning `None` means "structural line, not a message" (not counted); `Some(vec![])` means "message deliberately filtered" (counted). The `flat_map` closure borrows `self` (`&'_ self`), which is why the iterator is boxed with `+ '_`.

Add to `lib.rs`: `pub mod claude;` and `pub use claude::{discover_transcripts, strip_system_reminders, ClaudeTranscript};`.

- [ ] **Step 5: Run tests, lint**

Run: `cargo test -p singularmem-ingest && cargo clippy -p singularmem-ingest --all-targets -- -D warnings`
Expected: PASS (8 new tests). If clippy flags `too_many_lines` on `line_to_items`, add `#[allow(clippy::too_many_lines)]` on that fn only.

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-ingest
git commit -s -m "feat(ingest): Claude Code JSONL transcript source"
```

---

### Task 5: Directory walker source

**Files:**
- Create: `crates/singularmem-ingest/src/dir.rs`
- Create: `crates/singularmem-ingest/tests/dir_walk.rs`
- Modify: `crates/singularmem-ingest/src/lib.rs` (add `pub mod dir; pub use dir::DirectoryWalker;`)

**Interfaces:**
- Produces: `pub struct DirectoryWalker { pub root: PathBuf, pub max_file_bytes: u64, pub chunk_bytes: usize, .. }`, `DirectoryWalker::new(root) -> Result<Self>`, `impl Source`.
- Metadata contract used by Task 6: `metadata["sha256"]` is a hex string.

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Task 3
**Blocks:** Task 6

- [ ] **Step 1: Failing tests**

`crates/singularmem-ingest/tests/dir_walk.rs`:

```rust
use std::fs;

use singularmem_ingest::{DirectoryWalker, Source};
use tempfile::TempDir;

fn tree() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join("src")).unwrap();
    fs::create_dir_all(d.path().join("target")).unwrap();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".gitignore"), "target/\n").unwrap();
    fs::write(d.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(d.path().join("README.md"), "# hi\n").unwrap();
    fs::write(d.path().join("target/out.txt"), "ignored").unwrap();
    fs::write(d.path().join(".git/HEAD"), "ref: x").unwrap();
    fs::write(d.path().join("blob.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    fs::write(d.path().join("big.txt"), "x".repeat(2000)).unwrap();
    d
}

#[test]
fn walks_text_files_respecting_gitignore_and_hidden() {
    let d = tree();
    let mut w = DirectoryWalker::new(d.path()).unwrap();
    w.max_file_bytes = 1000;
    let items: Vec<_> = w.items().map(Result::unwrap).collect();
    let mut rels: Vec<String> = items.iter().map(|i| i.metadata["rel_path"].as_str().unwrap().to_string()).collect();
    rels.sort();
    assert_eq!(rels, vec!["README.md", "src/main.rs"]);
    assert_eq!(w.filtered_count(), 2, "blob.bin (binary) and big.txt (too large)");
}

#[test]
fn item_shape() {
    let d = tree();
    let w = DirectoryWalker::new(d.path()).unwrap();
    let item = w.items().map(Result::unwrap).find(|i| i.metadata["rel_path"] == "src/main.rs").unwrap();
    let root = d.path().canonicalize().unwrap();
    let abs = root.join("src/main.rs");
    assert_eq!(item.external_id.as_deref(), Some(format!("file:{}", abs.display()).as_str()));
    assert_eq!(item.source.as_deref(), Some(format!("dir:{}", root.display()).as_str()));
    assert_eq!(item.tags, vec!["ext:rs", "file"]);
    assert_eq!(item.content, "fn main() {}");
    assert_eq!(item.metadata["path"], abs.display().to_string());
    assert_eq!(item.metadata["size_bytes"], 13);
    assert_eq!(item.metadata["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(item.metadata["chunk_count"], 1);
}

#[test]
fn oversized_text_is_chunked() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join("a.txt"), format!("{}\n\n{}", "p".repeat(30), "q".repeat(30))).unwrap();
    let mut w = DirectoryWalker::new(d.path()).unwrap();
    w.chunk_bytes = 40;
    let items: Vec<_> = w.items().map(Result::unwrap).collect();
    assert_eq!(items.len(), 2);
    assert!(items[0].external_id.as_deref().unwrap().ends_with("a.txt#0"));
    assert!(items[1].external_id.as_deref().unwrap().ends_with("a.txt#1"));
}

#[test]
fn missing_root_is_not_found() {
    assert!(matches!(DirectoryWalker::new("/definitely/missing"), Err(singularmem_ingest::Error::NotFound { .. })));
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p singularmem-ingest --test dir_walk`
Expected: FAIL — `DirectoryWalker` unresolved.

- [ ] **Step 3: Implement `dir.rs`**

```rust
//! Source-tree walker: one item per text file, `.gitignore`-aware.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::Source;

/// Default per-file size cap (1 MiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1_048_576;

/// Walks a directory and yields one item per readable UTF-8 text file.
#[derive(Debug)]
pub struct DirectoryWalker {
    /// Canonicalised root.
    pub root: PathBuf,
    /// Files larger than this are skipped (counted as filtered).
    pub max_file_bytes: u64,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
}

impl DirectoryWalker {
    /// Create a walker rooted at `root` (must exist).
    ///
    /// # Errors
    /// `Error::NotFound` if `root` is not a directory; `Error::Io` if it
    /// cannot be canonicalised.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(Error::NotFound { path: root.to_path_buf() });
        }
        let root = root.canonicalize().map_err(|source| Error::Io { path: root.to_path_buf(), source })?;
        Ok(Self {
            root,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
        })
    }

    fn file_to_items(&self, abs: &Path) -> Result<Vec<NewItem>> {
        let meta = std::fs::metadata(abs).map_err(|source| Error::Io { path: abs.to_path_buf(), source })?;
        if meta.len() > self.max_file_bytes {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(abs).map_err(|source| Error::Io { path: abs.to_path_buf(), source })?;
        let sniff = &bytes[..bytes.len().min(8192)];
        if sniff.contains(&0) {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        }
        let Ok(text) = String::from_utf8(bytes) else {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        };
        let chunks = chunk_text(&text, self.chunk_bytes);
        if chunks.is_empty() {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        }
        let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
        let rel = abs.strip_prefix(&self.root).unwrap_or(abs);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let abs_str = abs.display().to_string();
        let ext = abs.extension().map(|e| e.to_string_lossy().to_lowercase());
        let chunk_count = chunks.len();
        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(i, content)| {
                let mut tags = vec!["file".to_string()];
                if let Some(e) = &ext {
                    tags.push(format!("ext:{e}"));
                }
                let external_id = if chunk_count == 1 { format!("file:{abs_str}") } else { format!("file:{abs_str}#{i}") };
                NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("dir:{}", self.root.display())),
                    metadata: serde_json::json!({
                        "path": abs_str,
                        "rel_path": rel_str,
                        "sha256": sha256,
                        "size_bytes": meta.len(),
                        "chunk_index": i,
                        "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                }
            })
            .collect())
    }
}

impl Source for DirectoryWalker {
    fn name(&self) -> String {
        self.root.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .sort_by_file_path(std::cmp::Ord::cmp)
            .build();
        Box::new(walker.flat_map(move |entry| match entry {
            Err(e) => vec![Err(Error::Io {
                path: self.root.clone(),
                source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
            })],
            Ok(entry) => {
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return Vec::new();
                }
                match self.file_to_items(entry.path()) {
                    Ok(items) => items.into_iter().map(Ok).collect(),
                    Err(e) => vec![Err(e)],
                }
            }
        }))
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }
}
```

Add `pub mod dir;` and `pub use dir::{DirectoryWalker, DEFAULT_MAX_FILE_BYTES};` to `lib.rs`.

- [ ] **Step 4: Run tests, lint, commit**

Run: `cargo test -p singularmem-ingest && cargo clippy -p singularmem-ingest --all-targets -- -D warnings`
Expected: PASS.

```bash
git add crates/singularmem-ingest
git commit -s -m "feat(ingest): gitignore-aware DirectoryWalker source"
```

---

### Task 6: `ingest_source` driver with dedup and replace

**Files:**
- Create: `crates/singularmem-ingest/src/driver.rs`
- Create: `crates/singularmem-ingest/tests/driver.rs`
- Modify: `crates/singularmem-ingest/src/lib.rs` (add `pub mod driver; pub use driver::{ingest_source, Report, BATCH_SIZE};`)

**Interfaces:**
- Consumes: Task 2 store methods; Task 3 `Source`.
- Produces: `pub struct Report { pub ingested: usize, pub skipped_existing: usize, pub skipped_filtered: usize, pub failed: usize }` (derives `Debug, Default, Clone, Copy, PartialEq, Eq`), `pub fn ingest_source(store: &Store, source: &dyn Source, dry_run: bool) -> Result<Report>`, `pub const BATCH_SIZE: usize = 500;`

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Tasks 2, 4, 5
**Blocks:** Task 7

- [ ] **Step 1: Failing tests**

`crates/singularmem-ingest/tests/driver.rs`:

```rust
use std::fs;

use singularmem_core::{NewItem, Store};
use singularmem_ingest::{ingest_source, DirectoryWalker, Report, Source};
use tempfile::TempDir;

struct Fixed(Vec<NewItem>);
impl Source for Fixed {
    fn name(&self) -> String { "fixed".into() }
    fn items(&self) -> Box<dyn Iterator<Item = singularmem_ingest::Result<NewItem>> + '_> {
        Box::new(self.0.iter().cloned().map(Ok))
    }
    fn filtered_count(&self) -> usize { 3 }
}

fn keyed(c: &str, k: &str) -> NewItem {
    let mut n = NewItem::text(c);
    n.external_id = Some(k.into());
    n
}

#[test]
fn second_run_ingests_nothing() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let src = Fixed(vec![keyed("a", "k:1"), keyed("b", "k:2")]);
    let r1 = ingest_source(&s, &src, false).unwrap();
    assert_eq!(r1, Report { ingested: 2, skipped_existing: 0, skipped_filtered: 3, failed: 0 });
    let r2 = ingest_source(&s, &src, false).unwrap();
    assert_eq!(r2, Report { ingested: 0, skipped_existing: 2, skipped_filtered: 3, failed: 0 });
    assert_eq!(s.list().unwrap().count(), 2);
}

#[test]
fn dry_run_writes_nothing_but_reports() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let src = Fixed(vec![keyed("a", "k:1")]);
    let r = ingest_source(&s, &src, true).unwrap();
    assert_eq!(r.ingested, 1);
    assert_eq!(s.list().unwrap().count(), 0);
}

#[test]
fn per_item_errors_are_counted_not_fatal() {
    struct Flaky;
    impl Source for Flaky {
        fn name(&self) -> String { "flaky".into() }
        fn items(&self) -> Box<dyn Iterator<Item = singularmem_ingest::Result<NewItem>> + '_> {
            Box::new(vec![
                Ok(keyed("a", "k:1")),
                Err(singularmem_ingest::Error::NotFound { path: "/x".into() }),
                Ok(keyed("b", "k:2")),
            ].into_iter())
        }
    }
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let r = ingest_source(&s, &Flaky, false).unwrap();
    assert_eq!(r.ingested, 2);
    assert_eq!(r.failed, 1);
}

#[test]
fn changed_file_supersedes_previous_item() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join("a.txt"), "version one").unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let w = DirectoryWalker::new(d.path()).unwrap();
    let r1 = ingest_source(&s, &w, false).unwrap();
    assert_eq!(r1.ingested, 1);
    let old = s.list().unwrap().next().unwrap().unwrap();

    // Unchanged → skipped.
    let r2 = ingest_source(&s, &w, false).unwrap();
    assert_eq!((r2.ingested, r2.skipped_existing), (0, 1));

    fs::write(d.path().join("a.txt"), "version two").unwrap();
    let r3 = ingest_source(&s, &w, false).unwrap();
    assert_eq!(r3.ingested, 1);
    let key = old.external_id.clone().unwrap();
    let newest = s.get_by_external_id(&key).unwrap().unwrap();
    assert_eq!(newest.content, "version two");
    assert_eq!(newest.supersedes, Some(old.id));
    assert_eq!(s.get(old.id).unwrap().external_id, None);
}

#[test]
fn large_batches_are_split() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let items: Vec<NewItem> = (0..1203).map(|i| keyed(&format!("c{i}"), &format!("k:{i}"))).collect();
    let r = ingest_source(&s, &Fixed(items), false).unwrap();
    assert_eq!(r.ingested, 1203);
    assert_eq!(s.list().unwrap().count(), 1203);
}
```

Note: the test's `Fixed` source needs `NewItem: Clone` — it already derives `Clone`. The store's `s.db` sits inside the walked temp dir in `changed_file_supersedes_previous_item`; add `fs::write(d.path().join(".gitignore"), "s.db*\n")` at the top of that test so the walker ignores the SQLite files.

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p singularmem-ingest --test driver`
Expected: FAIL — `ingest_source` unresolved.

- [ ] **Step 3: Implement `driver.rs`**

```rust
//! `ingest_source`: write a `Source` into a `Store`, idempotently.

use std::collections::HashSet;

use singularmem_core::{Error as CoreError, NewItem, Store};

use crate::error::{Error, Result};
use crate::Source;

/// Items per `ingest_many` transaction.
pub const BATCH_SIZE: usize = 500;

/// Outcome counts for one `ingest_source` run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// Items written (or, in dry-run, that would have been written).
    pub ingested: usize,
    /// Items whose `external_id` already existed with identical content hash.
    pub skipped_existing: usize,
    /// Inputs the source deliberately filtered (tool results, binaries, …).
    pub skipped_filtered: usize,
    /// Per-item source errors (malformed lines, unreadable files).
    pub failed: usize,
}

/// Ingest everything `source` yields that is not already in `store`.
///
/// Items carrying `metadata.sha256` whose stored counterpart has a
/// different hash are ingested via `Store::ingest_replacing`, superseding
/// the old item. Everything else new is written in batches of
/// [`BATCH_SIZE`]. With `dry_run`, nothing is written but counts are
/// computed as if it were.
///
/// # Errors
/// Returns `Err` only for store-level failures (read-only, SQLite). Source
/// errors are counted in `Report::failed`.
pub fn ingest_source(store: &Store, source: &dyn Source, dry_run: bool) -> Result<Report> {
    let mut report = Report::default();
    let mut candidates: Vec<NewItem> = Vec::new();
    for r in source.items() {
        match r {
            Ok(item) => candidates.push(item),
            Err(e) => {
                tracing::warn!(source = %source.name(), error = %e, "skipping item");
                report.failed += 1;
            }
        }
    }
    report.skipped_filtered = source.filtered_count();

    let ids: Vec<&str> = candidates.iter().filter_map(|i| i.external_id.as_deref()).collect();
    let existing: HashSet<String> = store.existing_external_ids(&ids)?;

    let mut fresh: Vec<NewItem> = Vec::new();
    for item in candidates {
        let Some(key) = item.external_id.clone() else {
            fresh.push(item);
            continue;
        };
        if !existing.contains(&key) {
            fresh.push(item);
            continue;
        }
        // Present: unchanged unless the content hash differs.
        let new_hash = item.metadata.get("sha256").and_then(|v| v.as_str());
        let old = store.get_by_external_id(&key)?;
        let old_hash = old.as_ref().and_then(|o| o.metadata.get("sha256").and_then(|v| v.as_str()));
        match (new_hash, old) {
            (Some(nh), Some(old)) if Some(nh) != old_hash => {
                if !dry_run {
                    store.ingest_replacing(item, old.id)?;
                }
                report.ingested += 1;
            }
            _ => report.skipped_existing += 1,
        }
    }

    if dry_run {
        report.ingested += fresh.len();
        return Ok(report);
    }

    for batch in fresh.chunks(BATCH_SIZE) {
        match store.ingest_many(batch.to_vec()) {
            Ok(written) => report.ingested += written.len(),
            Err(CoreError::ExternalIdConflict { .. }) => {
                // Race with a concurrent writer: re-filter and retry once.
                let ids: Vec<&str> = batch.iter().filter_map(|i| i.external_id.as_deref()).collect();
                let now_existing = store.existing_external_ids(&ids)?;
                let retry: Vec<NewItem> = batch
                    .iter()
                    .filter(|i| !i.external_id.as_deref().is_some_and(|k| now_existing.contains(k)))
                    .cloned()
                    .collect();
                report.skipped_existing += batch.len() - retry.len();
                let written = store.ingest_many(retry).map_err(Error::Core)?;
                report.ingested += written.len();
            }
            Err(e) => return Err(Error::Core(e)),
        }
    }
    Ok(report)
}
```

Add `pub mod driver;` and `pub use driver::{ingest_source, Report, BATCH_SIZE};` to `lib.rs`.

- [ ] **Step 4: Run tests, lint, commit**

Run: `cargo test -p singularmem-ingest && cargo clippy -p singularmem-ingest --all-targets -- -D warnings`
Expected: PASS (5 driver tests).

```bash
git add crates/singularmem-ingest
git commit -s -m "feat(ingest): ingest_source driver with external_id dedup and replace-on-change"
```

---

### Task 7: CLI verbs `ingest-transcript` and `ingest-dir`

**Files:**
- Modify: `Cargo.toml` (root `[dependencies]`: add `singularmem-ingest = { path = "crates/singularmem-ingest" }`)
- Modify: `src/main.rs` (enum `Command`, new `Args` structs, `needs_hook`, `run_command`, `CliError`, exit-code mapping, two `cmd_*` fns)
- Modify: `tests/cli.rs`
- Create: `tests/fixtures/transcripts/proj/session.jsonl` (copy of the crate fixture from Task 4)
- Modify: `README.md` (replace the stale "Status" and "Build" paragraphs; add a "Quickstart" with the two new verbs)
- Modify: `crates/singularmem-mcp/README.md` — no change; `docs/mcp-server.md` — no change.

**Interfaces:**
- Consumes: `singularmem_ingest::{ClaudeTranscript, DirectoryWalker, discover_transcripts, ingest_source, Report, Error as IngestError}`.

**Assigned skill:** `rust-best-practices`, `test-driven-development`, `verification-before-completion`
**Blocked-by:** Task 6
**Blocks:** none

- [ ] **Step 1: Failing CLI tests**

Append to `tests/cli.rs`:

```rust
fn fixture_transcripts() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts")
}

#[test]
fn ingest_transcript_is_idempotent_and_searchable() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fx = fixture_transcripts();

    singularmem()
        .args(["--store", db_s, "ingest-transcript", fx.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("ingested 4, skipped 0 existing, 5 filtered, 1 failed across 1 files"));

    singularmem()
        .args(["--store", db_s, "ingest-transcript", fx.to_str().unwrap(), "--quiet"])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 0, skipped 4 existing"));

    singularmem()
        .args(["--store", db_s, "search", "cargo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo")); // snippet may wrap the term in <mark>

    singularmem()
        .args(["--store", db_s, "list", "--tag", "role:user", "--format", "ids"])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 2));
}

#[test]
fn ingest_transcript_exit_code_reflects_failures_and_dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fx = fixture_transcripts();

    // The fixture contains one malformed line → failed=1 → exit 1.
    singularmem()
        .args(["--store", db_s, "ingest-transcript", fx.to_str().unwrap(), "--dry-run"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ingested 4"));
    singularmem()
        .args(["--store", db_s, "list", "--format", "ids"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn ingest_transcript_project_filter() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fx = fixture_transcripts();
    singularmem()
        .args(["--store", db_s, "ingest-transcript", fx.to_str().unwrap(), "--project", "/home/me/proj"])
        .assert()
        .code(1) // malformed line still counts
        .stderr(predicate::str::contains("ingested 3"));
}

#[test]
fn ingest_transcript_missing_path_is_exit_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .args(["--store", db.to_str().unwrap(), "ingest-transcript", "/definitely/missing"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("path not found"));
}

#[test]
fn ingest_dir_tracks_changes_via_supersedes() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    std::fs::write(tree.join("src/a.rs"), "fn a() {}").unwrap();
    std::fs::write(tree.join("b.md"), "# b").unwrap();

    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 2, skipped 0 existing"));

    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 0, skipped 2 existing"));

    std::fs::write(tree.join("src/a.rs"), "fn a() { changed() }").unwrap();
    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 1, skipped 1 existing"));

    let ids = singularmem()
        .args(["--store", db_s, "list", "--tag", "ext:rs", "--format", "ids"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ids = String::from_utf8(ids).unwrap();
    let newest = ids.lines().last().unwrap().to_string();
    singularmem()
        .args(["--store", db_s, "revisions", &newest, "--format", "ids"])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 2));
}

#[test]
fn ingest_dir_read_only_store_is_exit_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem().args(["--store", db_s, "ingest", "--content", "seed"]).assert().success();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("x.txt"), "x").unwrap();
    singularmem()
        .args(["--store", db_s, "--read-only", "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("read-only"));
}
```

Copy the Task 4 fixture: `mkdir -p tests/fixtures/transcripts/proj && cp crates/singularmem-ingest/tests/fixtures/session.jsonl tests/fixtures/transcripts/proj/`.

`revisions --format ids` prints one ULID per line, newest first (see `cmd_revisions` in `src/main.rs`).

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test cli ingest_transcript ingest_dir`
Expected: FAIL — unknown subcommand.

- [ ] **Step 3: Wire the CLI**

Root `Cargo.toml` `[dependencies]`: add `singularmem-ingest = { path = "crates/singularmem-ingest" }`.

`src/main.rs`:

Add to `enum Command` after `Ingest(IngestArgs)`:

```rust
    /// Bulk-ingest Claude Code JSONL transcripts (idempotent).
    IngestTranscript(IngestTranscriptArgs),
    /// Bulk-ingest a source tree, honouring .gitignore (idempotent).
    IngestDir(IngestDirArgs),
```

Add arg structs after `IngestArgs`:

```rust
#[derive(Args, Debug)]
struct IngestTranscriptArgs {
    /// Transcript files or directories (searched recursively for *.jsonl).
    /// Defaults to ~/.claude/projects.
    paths: Vec<PathBuf>,
    /// Keep only messages whose working directory equals DIR.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Keep subagent (sidechain) messages.
    #[arg(long)]
    include_sidechains: bool,
    /// Parse and report; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    quiet: bool,
}

#[derive(Args, Debug)]
struct IngestDirArgs {
    /// Root directory to walk.
    path: PathBuf,
    /// Skip files larger than this many bytes.
    #[arg(long, default_value_t = singularmem_ingest::DEFAULT_MAX_FILE_BYTES)]
    max_file_bytes: u64,
    /// Parse and report; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    quiet: bool,
}
```

In `run`, change `needs_hook` to:

```rust
    let needs_hook = matches!(
        cli.command,
        Command::Ingest(_) | Command::IngestTranscript(_) | Command::IngestDir(_)
    );
```

and, immediately before `let needs_hook`, refuse read-only bulk verbs up front:

```rust
    if cli.read_only
        && matches!(cli.command, Command::IngestTranscript(_) | Command::IngestDir(_))
    {
        return Err(CliError::StoreReadOnly);
    }
```

Add to `CliError`:

```rust
    #[error("{0}")]
    Ingest(#[from] singularmem_ingest::Error),
    #[error("store is opened read-only; bulk ingest requires write access")]
    StoreReadOnly,
    #[error("{failed} item(s) failed during bulk ingest; see warnings above")]
    IngestPartial { failed: usize },
```

In `main`'s exit-code `match`, add before the catch-all:

```rust
        Err(CliError::StoreReadOnly) => {
            eprintln!("singularmem: store is opened read-only; bulk ingest requires write access");
            ExitCode::from(2)
        }
        Err(CliError::Ingest(singularmem_ingest::Error::NotFound { ref path })) => {
            eprintln!("singularmem: path not found: {}", path.display());
            ExitCode::from(2)
        }
        Err(CliError::IngestPartial { .. }) => ExitCode::from(1),
```

In `run_command` add:

```rust
        Command::IngestTranscript(args) => cmd_ingest_transcript(store, &args),
        Command::IngestDir(args) => cmd_ingest_dir(store, &args),
```

Add the two command functions after `cmd_ingest`:

```rust
fn cmd_ingest_transcript(store: &Store, args: &IngestTranscriptArgs) -> Result<(), CliError> {
    use singularmem_ingest::{discover_transcripts, ingest_source, ClaudeTranscript, Report};

    let roots: Vec<PathBuf> = if args.paths.is_empty() {
        vec![dirs::home_dir()
            .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?
            .join(".claude")
            .join("projects")]
    } else {
        args.paths.clone()
    };

    let mut files: Vec<PathBuf> = Vec::new();
    for root in &roots {
        if root.is_dir() {
            files.extend(discover_transcripts(root)?);
        } else if root.is_file() {
            files.push(root.clone());
        } else {
            return Err(singularmem_ingest::Error::NotFound { path: root.clone() }.into());
        }
    }

    let project = args.project.as_ref().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()));
    let mut total = Report::default();
    let mut failed_files = 0usize;
    for file in &files {
        let mut src = match ClaudeTranscript::open(file) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %file.display(), error = %e, "cannot open transcript");
                failed_files += 1;
                continue;
            }
        };
        src.include_sidechains = args.include_sidechains;
        src.project_filter.clone_from(&project);
        let r = ingest_source(store, &src, args.dry_run)?;
        if !args.quiet {
            eprintln!("{}: +{} ingested, {} skipped", file.display(), r.ingested, r.skipped_existing + r.skipped_filtered);
        }
        accumulate(&mut total, r);
    }
    print_summary(&total, files.len());
    if total.failed > 0 || failed_files > 0 {
        return Err(CliError::IngestPartial { failed: total.failed + failed_files });
    }
    Ok(())
}

fn cmd_ingest_dir(store: &Store, args: &IngestDirArgs) -> Result<(), CliError> {
    use singularmem_ingest::{ingest_source, DirectoryWalker};

    let mut src = DirectoryWalker::new(&args.path)?;
    src.max_file_bytes = args.max_file_bytes;
    let r = ingest_source(store, &src, args.dry_run)?;
    if !args.quiet {
        eprintln!("{}: +{} ingested, {} skipped", src.root.display(), r.ingested, r.skipped_existing + r.skipped_filtered);
    }
    print_summary(&r, 1);
    if r.failed > 0 {
        return Err(CliError::IngestPartial { failed: r.failed });
    }
    Ok(())
}

fn accumulate(total: &mut singularmem_ingest::Report, r: singularmem_ingest::Report) {
    total.ingested += r.ingested;
    total.skipped_existing += r.skipped_existing;
    total.skipped_filtered += r.skipped_filtered;
    total.failed += r.failed;
}

fn print_summary(r: &singularmem_ingest::Report, files: usize) {
    eprintln!(
        "ingested {}, skipped {} existing, {} filtered, {} failed across {} files",
        r.ingested, r.skipped_existing, r.skipped_filtered, r.failed, files
    );
}
```

- [ ] **Step 4: Run CLI tests**

Run: `cargo test --test cli`
Expected: PASS, all new tests green and existing ones unchanged. If `help_lists_all_subcommands` is the only failure, it is not — it asserts substrings that still exist; investigate any other failure with the failing test's output rather than adjusting assertions.

- [ ] **Step 5: README**

In `README.md` replace the `> **Status:** …` blockquote and the entire "## Build" section with:

```markdown
> **Status:** v0.17.0 — memory store, hybrid search (Tantivy + USearch),
> provider adapters, MCP server, TypeScript SDK, and bulk transcript
> ingestion. Constitution v0.2.0 ratified 2026-05-15.

## Quickstart

```bash
# Make every past Claude Code session searchable
singularmem ingest-transcript            # defaults to ~/.claude/projects
singularmem ingest-transcript --project "$PWD"   # only sessions from this repo

# Index a source tree (honours .gitignore; re-runs only pick up changes)
singularmem ingest-dir .

singularmem search "why did we pick tantivy"
singularmem retrieve --adapter claude "release process"
```

Both bulk verbs are idempotent: re-running ingests nothing already present.
```

- [ ] **Step 6: Full verification**

Run, and paste the tail of each into the PR description:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
./target/release/singularmem --store /tmp/sm-demo.db ingest-transcript ~/.claude/projects/-Users-jonasbroms-Sites-singularmem
./target/release/singularmem --store /tmp/sm-demo.db ingest-transcript ~/.claude/projects/-Users-jonasbroms-Sites-singularmem --quiet   # expect "ingested 0"
./target/release/singularmem --store /tmp/sm-demo.db ingest-dir .
./target/release/singularmem --store /tmp/sm-demo.db search "mempalace"
rm -rf /tmp/sm-demo.db*
```

- [ ] **Step 7: Commit and open the PR**

```bash
git add Cargo.toml Cargo.lock src/main.rs tests README.md
git commit -s -m "feat(cli): ingest-transcript and ingest-dir bulk verbs"
git push -u origin transcript-ingestion-11
gh pr create --title "feat: transcript + directory ingestion (sub-project 11)" --body-file - <<'PR'
Implements docs/superpowers/specs/2026-09-03-transcript-ingestion-11-design.md.

- store format v2: nullable unique `external_id`, 1→2 migration, `docs/formats/store-v2.md`
- new crate `singularmem-ingest`: `Source` trait, Claude Code JSONL parser, gitignore-aware directory walker, chunker, dedup driver
- CLI: `ingest-transcript`, `ingest-dir` (idempotent; changed files supersede)

Verification output attached below.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
PR
```

Then set the plan's frontmatter `status: merged` once the PR lands.
