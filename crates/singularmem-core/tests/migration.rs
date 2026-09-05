//! Store format migrations (1 → 2 → 3 → 4). Builds v1, v2, and v3 stores
//! with raw SQL (the exact DDL from `docs/formats/store-v1.md` and the
//! 1 → 2 / 2 → 3 migration statements), opens them with the current binary,
//! and asserts each is upgraded in place with all data intact.

use rusqlite::Connection;
use singularmem_core::graph::NewFact;
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

fn make_v3(dir: &TempDir) -> std::path::PathBuf {
    let path = make_v2(dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE items ADD COLUMN scope TEXT;
         CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;
         UPDATE singularmem_meta SET value = '3' WHERE key = 'format_version';",
    )
    .unwrap();
    path
}

/// Read back `(type, name, sql)` for every non-`sqlite_%` object in
/// `sqlite_master`, with the `sql` column's whitespace stripped so two
/// schemas written with different formatting still compare equal. Used by
/// [`fresh_and_migrated_v4_schemas_are_identical`].
fn schema(path: &std::path::Path) -> Vec<(String, String, String)> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT type, name, sql FROM sqlite_master \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            let sql: Option<String> = r.get(2)?;
            let stripped: String = sql
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            Ok((r.get(0)?, r.get(1)?, stripped))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    drop(stmt);
    rows
}

#[test]
fn v1_store_migrates_to_v3_on_open() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);

    let store = Store::open(&path).expect("open migrates");
    assert_eq!(store.format_version().unwrap(), "4");

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
        "UPDATE singularmem_meta SET value='5' WHERE key='format_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(matches!(err, Error::UnsupportedFormatVersion { .. }));
}

/// Simulates the losing side of a migration race: another process already
/// completed the 3 -> 4 migration (full DDL + meta bump) by the time this
/// one opens the file. `Store::open` must still succeed and see version 4 —
/// whether it takes the "already current" fast path or re-enters a
/// migration function's in-transaction re-check, the outcome is the same.
#[test]
fn already_migrated_v3_fixture_opens_cleanly_as_v4() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE entities (
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
             UPDATE singularmem_meta SET value = '4' WHERE key = 'format_version';",
        )
        .unwrap();
    }

    let store = Store::open(&path).expect("open of already-migrated fixture succeeds");
    assert_eq!(store.format_version().unwrap(), "4");
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
    assert_eq!(store.format_version().unwrap(), "4");
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
    assert_eq!(meta["store_format_version"], "4");

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
    assert_eq!(store.format_version().unwrap(), "4");
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
    assert_eq!(format_version, "4");

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
    assert_eq!(store.format_version().unwrap(), "4");
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
    assert_eq!(store.format_version().unwrap(), "4");
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
/// around the DDL, the losers hit "table `singularmem_meta` already exists"
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
                assert_eq!(s.format_version().unwrap(), "4", "opener {i}");
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

#[test]
fn v3_store_migrates_to_v4_with_graph_tables() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.format_version().unwrap(), "4");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    for t in ["entities", "facts"] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [t],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "table {t}");
    }
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_entities_identity'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn v1_store_migrates_through_the_chain_to_v4() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);
    assert_eq!(Store::open(&path).unwrap().format_version().unwrap(), "4");
}

#[test]
fn failing_3_to_4_leaves_store_at_v3_and_readable() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE INDEX idx_facts_subject ON items(source);")
        .unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(
        matches!(err, Error::Migration { ref from, to: "4", .. } if from == "3"),
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
    assert_eq!(v, "3");
}

#[test]
fn read_only_v3_refuses_to_migrate() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);
    assert!(matches!(
        Store::open_with_options(&path, StoreOptions { read_only: true }),
        Err(Error::Migration { .. })
    ));
}

/// A store migrated 3 → 4 ends up with exactly the schema a fresh v4 store
/// is created with: same tables, same indexes, same definitions. Compared
/// with whitespace stripped, because `make_v3` writes the v1–v3 DDL more
/// compactly than `schema.rs` does and `sqlite_master.sql` preserves the
/// original text verbatim: the layout differs while the schema — every
/// table, index, column, and constraint — must not.
#[test]
fn fresh_and_migrated_v4_schemas_are_identical() {
    let fresh_dir = TempDir::new().unwrap();
    let fresh = fresh_dir.path().join("fresh.db");
    drop(Store::open(&fresh).unwrap());

    let migrated_dir = TempDir::new().unwrap();
    let migrated = make_v3(&migrated_dir);
    let store = Store::open(&migrated).unwrap();
    assert_eq!(store.format_version().unwrap(), "4");
    drop(store);

    let fresh_schema = schema(&fresh);
    let migrated_schema = schema(&migrated);
    assert!(
        fresh_schema.iter().any(|(_, name, _)| name == "facts"),
        "the fresh store really did create the v4 tables"
    );
    assert_eq!(fresh_schema, migrated_schema);
}

/// Acceptance criterion 6 for the graph: a raw-`rusqlite` loader — no
/// Singularmem code — reads a fact out of a store this binary migrated
/// 3 → 4, using exactly the head and as-of SQL documented in
/// `docs/formats/store-v4.md` § "Revisions and the two time axes".
#[test]
fn third_party_loader_reads_graph_from_migrated_store() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);

    let store = Store::open(&path).expect("open migrates 3 -> 4");
    assert_eq!(store.format_version().unwrap(), "4");
    store
        .add_fact(NewFact::triple("singularmem", "uses", "tantivy"))
        .unwrap();
    drop(store);

    let conn = Connection::open(&path).unwrap();

    // Head SQL from store-v4.md: the open, non-superseded revision of the
    // (subject, predicate) pair.
    let rows: Vec<(String, String)> = conn
        .prepare(
            "SELECT s.name, f.predicate FROM facts f \
             JOIN entities s ON s.id = f.subject_id \
             WHERE f.valid_to IS NULL \
               AND NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id)",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![("singularmem".to_string(), "uses".to_string())],
        "exactly one open head fact, unaffected by the migration"
    );

    // As-of SQL from store-v4.md, with T inside the (unbounded, unbounded)
    // window every freshly-added fact starts in: still matches.
    let as_of_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM facts f \
             JOIN entities s ON s.id = f.subject_id \
             WHERE s.name = 'singularmem' AND f.predicate = 'uses' \
               AND NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id) \
               AND (f.valid_from IS NULL OR f.valid_from <= '2026-09-05T00:00:00Z') \
               AND (f.valid_to IS NULL OR '2026-09-05T00:00:00Z' < f.valid_to)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(as_of_rows, 1, "T inside the open window still matches");
}
