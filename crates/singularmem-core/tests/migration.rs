//! Store format migrations (1 → 2 → 3). Builds v1 and v2 stores with raw
//! SQL (the exact DDL from `docs/formats/store-v1.md` and the 1 → 2
//! migration statements), opens them with the current binary, and asserts
//! each is upgraded in place with all data intact.

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

fn make_v2(dir: &TempDir) -> std::path::PathBuf {
    let path = make_v1(dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE items ADD COLUMN external_id TEXT;
         CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
         UPDATE singularmem_meta SET value = '2' WHERE key = 'format_version';",
    )
    .unwrap();
    path
}

#[test]
fn v1_store_migrates_to_v3_on_open() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);

    let store = Store::open(&path).expect("open migrates");
    assert_eq!(store.format_version().unwrap(), "3");

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
        "UPDATE singularmem_meta SET value='4' WHERE key='format_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(matches!(err, Error::UnsupportedFormatVersion { .. }));
}

/// Simulates the losing side of a migration race: another process already
/// completed the 1 -> 2 -> 3 migration chain (full DDL + meta bump) by the
/// time this one opens the file. `Store::open` must still succeed and see
/// version 3 — whether it takes the "already current" fast path or
/// re-enters a migration function's in-transaction re-check, the outcome is
/// the same.
#[test]
fn already_migrated_v2_fixture_opens_cleanly_as_v3() {
    let dir = TempDir::new().unwrap();
    let path = make_v2(&dir);

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "ALTER TABLE items ADD COLUMN scope TEXT;
             CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;
             UPDATE singularmem_meta SET value = '3' WHERE key = 'format_version';",
        )
        .unwrap();
    }

    let store = Store::open(&path).expect("open of already-migrated fixture succeeds");
    assert_eq!(store.format_version().unwrap(), "3");
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
fn fresh_store_is_v3_with_external_id_column() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fresh.db");
    let store = Store::open(&path).unwrap();
    assert_eq!(store.format_version().unwrap(), "3");
    let mut item = singularmem_core::NewItem::text("x");
    item.external_id = Some("test:1".into());
    let stored = store.ingest(item).unwrap();
    assert_eq!(stored.external_id.as_deref(), Some("test:1"));
    assert_eq!(
        store.get(stored.id).unwrap().external_id.as_deref(),
        Some("test:1")
    );
}

/// Plan acceptance criterion 5: a v1 store written by an older binary opens
/// under the current one and exports cleanly at `store_format_version = 3`.
#[test]
fn migrated_v1_store_exports_cleanly() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);

    let store = Store::open(&path).expect("open migrates");
    let mut out: Vec<u8> = Vec::new();
    store.export(&mut out).expect("export");
    let text = String::from_utf8(out).unwrap();
    let mut lines = text.lines();

    let meta: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(meta["_kind"], "meta");
    assert_eq!(meta["store_format_version"], "3");

    let item: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(item["_kind"], "item");
    assert_eq!(item["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    assert_eq!(item["content"], "legacy item");
    assert_eq!(item["tags"], serde_json::json!(["old"]));
    assert!(
        item.get("external_id").is_none(),
        "a migrated legacy row carries no external_id: {item}"
    );
    assert!(lines.next().is_none(), "exactly one item line");
}

#[test]
fn v2_store_migrates_to_v3_on_open() {
    let dir = TempDir::new().unwrap();
    let path = make_v2(&dir);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.format_version().unwrap(), "3");
    let item = store
        .get("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap())
        .unwrap();
    assert_eq!(item.scope, None);

    // Spec acceptance criterion 6: a raw-rusqlite loader (no Singularmem
    // code) can read `scope` from a store this binary migrated, per the
    // walkthrough in docs/formats/store-v3.md.
    let mut scoped = singularmem_core::NewItem::text("loader-visible item");
    scoped.scope = Some("loader/check".into());
    let scoped = store.ingest(scoped).unwrap();
    assert_eq!(scoped.scope.as_deref(), Some("loader/check"));

    // A true descendant, plus two near-misses a correct descendant-inclusive
    // query must exclude: a hyphenated sibling ("loader-check") and an
    // underscore near-miss ("loader_check") that would only slip in if `_`
    // — a legal scope byte and a SQL LIKE wildcard — were left unescaped in
    // the bound pattern.
    let mut child = singularmem_core::NewItem::text("descendant item");
    child.scope = Some("loader/check/sub".into());
    let child = store.ingest(child).unwrap();

    let mut hyphen_sibling = singularmem_core::NewItem::text("hyphen sibling");
    hyphen_sibling.scope = Some("loader-check".into());
    store.ingest(hyphen_sibling).unwrap();

    let mut underscore_sibling = singularmem_core::NewItem::text("underscore sibling");
    underscore_sibling.scope = Some("loader_check".into());
    store.ingest(underscore_sibling).unwrap();

    drop(store);

    let conn = Connection::open(&path).unwrap();

    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('items')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(cols.contains(&"scope".to_string()));
    let idx: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_items_scope'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 1);

    let scope: String = conn
        .query_row(
            "SELECT scope FROM items WHERE id = ?1",
            [scoped.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scope, "loader/check");

    let format_version: String = conn
        .query_row(
            "SELECT value FROM singularmem_meta WHERE key='format_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(format_version, "3");

    // Descendant-inclusive query from docs/formats/store-v3.md, using the
    // escaped-LIKE form the reference implementation actually binds: `?2`
    // is the path with `\`, `%`, and `_` backslash-escaped, then `/%`
    // appended. None of those bytes appear in "loader/check", so escaping
    // is a no-op here and `?2` is simply "loader/check/%".
    let ids: Vec<String> = conn
        .prepare(
            "SELECT id FROM items WHERE scope = ?1 OR scope LIKE ?2 ESCAPE '\\' \
             ORDER BY created_at",
        )
        .unwrap()
        .query_map(rusqlite::params!["loader/check", "loader/check/%"], |r| {
            r.get(0)
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        ids,
        vec![scoped.id.to_string(), child.id.to_string()],
        "must match the exact scope and its descendant, in created_at \
         order, and exclude both near-miss siblings"
    );
}

#[test]
fn v1_store_migrates_through_the_chain_to_v3() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.format_version().unwrap(), "3");
    let conn = Connection::open(&path).unwrap();
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('items')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(cols.contains(&"external_id".to_string()) && cols.contains(&"scope".to_string()));
}

#[test]
fn failing_2_to_3_leaves_store_at_v2_and_readable() {
    let dir = TempDir::new().unwrap();
    let path = make_v2(&dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE INDEX idx_items_scope ON items(source);")
        .unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(
        matches!(err, Error::Migration { ref from, to: "3", .. } if from == "2"),
        "{err:?}"
    );
    let conn = Connection::open(&path).unwrap();
    let v: String = conn
        .query_row(
            "SELECT value FROM singularmem_meta WHERE key='format_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, "2");
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('items')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!cols.contains(&"scope".to_string()));
}

#[test]
fn read_only_v2_refuses_to_migrate() {
    let dir = TempDir::new().unwrap();
    let path = make_v2(&dir);
    let err = Store::open_with_options(&path, StoreOptions { read_only: true }).unwrap_err();
    assert!(matches!(err, Error::Migration { .. }));
}

#[test]
fn fresh_store_is_v3_and_round_trips_scope() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("f.db")).unwrap();
    assert_eq!(store.format_version().unwrap(), "3");
    let mut n = singularmem_core::NewItem::text("x");
    n.scope = Some("Proj/Sub".into());
    let stored = store.ingest(n).unwrap();
    assert_eq!(
        stored.scope.as_deref(),
        Some("proj/sub"),
        "normalised at ingest"
    );
    assert_eq!(
        store.get(stored.id).unwrap().scope.as_deref(),
        Some("proj/sub")
    );
    let mut bad = singularmem_core::NewItem::text("y");
    bad.scope = Some("a//b".into());
    assert!(matches!(
        store.ingest(bad),
        Err(Error::Validation { field: "scope", .. })
    ));
}

/// Four processes (here: threads) opening the same *non-existent* store at
/// once all take the fresh-store bootstrap branch. Without a transaction
/// around the DDL, the losers hit "table singularmem_meta already exists"
/// and `Store::open` fails — exactly the shape a burst of editor hooks
/// produces on a brand-new machine.
#[test]
fn concurrent_first_open_of_a_fresh_store_all_succeed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fresh.db");
    assert!(!path.exists());

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let path = path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(&path).map(|s| (i, s))
            })
        })
        .collect();

    let mut stores = Vec::new();
    for h in handles {
        match h.join().expect("thread panicked") {
            Ok((i, s)) => {
                assert_eq!(s.format_version().unwrap(), "3", "opener {i}");
                stores.push(s);
            }
            Err(e) => panic!("concurrent Store::open failed: {e}"),
        }
    }
    assert_eq!(stores.len(), 4);

    // Exactly one bootstrap ran: one format_version row, one created_at row.
    let conn = Connection::open(&path).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM singularmem_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 2, "one format_version and one created_at row");
}
