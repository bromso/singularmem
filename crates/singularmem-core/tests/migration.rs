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

    let item = store
        .get("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap())
        .unwrap();
    assert_eq!(item.content, "legacy item");
    assert_eq!(item.tags, vec!["old"]);
    assert_eq!(item.external_id, None);
    drop(store);

    // Third-party loader check: the column and unique index exist.
    let conn = Connection::open(&path).unwrap();
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('items')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
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
        .query_row(
            "SELECT value FROM singularmem_meta WHERE key='format_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, "1");
}

#[test]
fn newer_store_still_refused() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE singularmem_meta SET value='3' WHERE key='format_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(matches!(err, Error::UnsupportedFormatVersion { .. }));
}

/// Simulates the losing side of a migration race: another process already
/// completed the 1 -> 2 migration (full DDL + meta bump) by the time this
/// one opens the file. `Store::open` must still succeed and see version 2 —
/// whether it takes the "already current" fast path or re-enters
/// `migrate_1_to_2`'s in-transaction re-check, the outcome is the same.
#[test]
fn already_migrated_v1_fixture_opens_cleanly_as_v2() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "ALTER TABLE items ADD COLUMN external_id TEXT;
             CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
             UPDATE singularmem_meta SET value = '2' WHERE key = 'format_version';",
        )
        .unwrap();
    }

    let store = Store::open(&path).expect("open of already-migrated fixture succeeds");
    assert_eq!(store.format_version().unwrap(), "2");
}

/// Failure path: the migration's `CREATE UNIQUE INDEX idx_items_external_id`
/// collides with a pre-existing index of the same name (a hostile or
/// corrupted v1 fixture). The whole transaction — including the preceding
/// `ALTER TABLE ADD COLUMN` — must roll back: `format_version` stays `'1'`
/// and `items` must not gain an `external_id` column.
#[test]
fn conflicting_index_name_fails_migration_and_leaves_v1_intact() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE INDEX idx_items_external_id ON items(source)", [])
            .unwrap();
    }

    let err = Store::open(&path).unwrap_err();
    assert!(matches!(err, Error::Migration { .. }), "got {err:?}");

    let conn = Connection::open(&path).unwrap();
    let v: String = conn
        .query_row(
            "SELECT value FROM singularmem_meta WHERE key='format_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, "1", "format_version must remain 1 after rollback");

    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('items')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        !cols.contains(&"external_id".to_string()),
        "ALTER TABLE must have been rolled back too: {cols:?}"
    );
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
    assert_eq!(
        store.get(stored.id).unwrap().external_id.as_deref(),
        Some("test:1")
    );
}
