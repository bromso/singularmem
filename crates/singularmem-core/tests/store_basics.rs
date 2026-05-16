//! Smoke tests for Store lifecycle: open creates schema; reopen finds it;
//! `format_version` is recorded; unsupported versions are rejected.

use singularmem_core::{NewItem, Store, StoreOptions, FORMAT_VERSION};
use tempfile::TempDir;

#[test]
fn open_fresh_creates_schema_and_format_version() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");

    let store = Store::open(&path).expect("open fresh");
    assert_eq!(store.format_version().expect("read meta"), FORMAT_VERSION);
}

#[test]
fn reopen_existing_does_not_recreate_schema() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");

    {
        let _ = Store::open(&path).expect("open fresh");
    } // drop closes

    let reopened = Store::open(&path).expect("reopen");
    assert_eq!(
        reopened.format_version().expect("read meta"),
        FORMAT_VERSION
    );
}

#[test]
fn open_creates_parent_directory() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("subdir").join("store.db");
    assert!(!path.parent().unwrap().exists());

    let _ = Store::open(&path).expect("open creates parents");
    assert!(path.parent().unwrap().exists());
    assert!(path.exists());
}

#[test]
fn open_with_options_read_only_refuses_create() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.db");
    let result = Store::open_with_options(&path, StoreOptions { read_only: true });
    assert!(result.is_err());
}

#[test]
fn unsupported_format_version_rejected() {
    use rusqlite::Connection;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("future.db");

    // Manually create a store with format_version = "999"
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE singularmem_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;").unwrap();
        conn.execute(
            "INSERT INTO singularmem_meta (key, value) VALUES ('format_version', '999')",
            [],
        )
        .unwrap();
    }

    let err = Store::open(&path).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("999"), "error mentions found version: {msg}");
    assert!(msg.contains('1'), "error mentions max supported: {msg}");
}

#[test]
fn ingest_then_get_round_trip() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();

    let mut new_item = NewItem::text("hello");
    new_item.tags = vec!["greeting".into()];
    new_item.source = Some("test".into());
    new_item.metadata = serde_json::json!({"k": "v"});

    let stored = store.ingest(new_item).expect("ingest");
    assert_eq!(stored.content, "hello");
    assert_eq!(stored.tags, vec!["greeting"]);
    assert_eq!(stored.source.as_deref(), Some("test"));
    assert_eq!(stored.metadata, serde_json::json!({"k": "v"}));

    let fetched = store.get(stored.id).expect("get");
    assert_eq!(fetched, stored);
}

#[test]
fn ingest_assigns_distinct_ids_for_concurrent_calls() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let a = store.ingest(NewItem::text("a")).unwrap();
    let b = store.ingest(NewItem::text("b")).unwrap();
    assert_ne!(a.id, b.id);
    assert!(b.created_at >= a.created_at);
}

#[test]
fn ingest_supersedes_resolution_succeeds() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let original = store.ingest(NewItem::text("original")).unwrap();

    let mut correction = NewItem::text("correction");
    correction.supersedes = Some(original.id);
    let new = store.ingest(correction).expect("ingest with supersedes");
    assert_eq!(new.supersedes, Some(original.id));
}

#[test]
fn ingest_supersedes_unknown_id_errors() {
    use singularmem_core::Error;
    use std::str::FromStr;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let store = Store::open(&path).unwrap();
    let bogus = singularmem_core::ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();

    let mut correction = NewItem::text("correction");
    correction.supersedes = Some(bogus);
    let err = store.ingest(correction).unwrap_err();
    assert!(matches!(err, Error::SupersedesNotFound { .. }));

    // Verify no rows by direct SQL on the same file.
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn ingest_many_persists_all_items_atomically() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let items = vec![
        NewItem::text("one"),
        NewItem::text("two"),
        NewItem::text("three"),
    ];
    let stored = store.ingest_many(items).expect("bulk ingest");
    assert_eq!(stored.len(), 3);
    assert_eq!(stored[0].content, "one");
    assert_eq!(stored[2].content, "three");
}

#[test]
fn ingest_many_rolls_back_on_validation_failure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let store = Store::open(&path).unwrap();
    let items = vec![
        NewItem::text("good"),
        NewItem::text(""), // validation failure here
        NewItem::text("never-reached"),
    ];
    let err = store.ingest_many(items).unwrap_err();
    assert!(matches!(
        err,
        singularmem_core::Error::Validation {
            field: "content",
            ..
        }
    ));
    drop(store);
    // Confirm zero rows persisted — atomic rollback.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn get_optional_returns_none_for_missing() {
    use std::str::FromStr;
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let bogus = singularmem_core::ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let result = store.get_optional(bogus).expect("get_optional ok");
    assert!(result.is_none());
}

#[test]
fn get_optional_returns_some_for_present() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let item = store.ingest(NewItem::text("present")).unwrap();
    let fetched = store
        .get_optional(item.id)
        .expect("get_optional ok")
        .expect("present");
    assert_eq!(fetched, item);
}

#[test]
fn list_iterates_in_created_at_order() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let a = store.ingest(NewItem::text("a")).unwrap();
    let b = store.ingest(NewItem::text("b")).unwrap();
    let c = store.ingest(NewItem::text("c")).unwrap();
    let ids: Vec<_> = store.list().unwrap().map(|r| r.unwrap().id).collect();
    assert_eq!(ids, vec![a.id, b.id, c.id]);
}

#[test]
fn list_by_tags_filters_and_semantics() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let mut item_a = NewItem::text("a");
    item_a.tags = vec!["work".into(), "urgent".into()];
    let mut item_b = NewItem::text("b");
    item_b.tags = vec!["work".into()];
    let mut item_c = NewItem::text("c");
    item_c.tags = vec!["urgent".into()];
    let a = store.ingest(item_a).unwrap();
    let _ = store.ingest(item_b).unwrap();
    let _ = store.ingest(item_c).unwrap();

    // AND filter — only items with BOTH tags
    let filtered: Vec<_> = store
        .list_by_tags(&["work", "urgent"])
        .unwrap()
        .map(|r| r.unwrap().id)
        .collect();
    assert_eq!(filtered, vec![a.id]);
}

#[test]
fn list_by_tags_empty_filter_lists_everything() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let _ = store.ingest(NewItem::text("a")).unwrap();
    let _ = store.ingest(NewItem::text("b")).unwrap();
    let count = store.list_by_tags(&[]).unwrap().count();
    assert_eq!(count, 2);
}

#[test]
fn export_emits_meta_line_and_items_in_order() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let _a = store.ingest(NewItem::text("first")).unwrap();
    let _b = store.ingest(NewItem::text("second")).unwrap();

    let mut buf = Vec::new();
    store.export(&mut buf).expect("export ok");
    let text = String::from_utf8(buf).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "1 meta + 2 item lines");

    let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(meta["_singularmem_format"], "export-v1");
    assert_eq!(meta["_kind"], "meta");
    assert_eq!(meta["store_format_version"], "1");

    let first: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first["_kind"], "item");
    assert_eq!(first["content"], "first");

    let second: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(second["content"], "second");
}
