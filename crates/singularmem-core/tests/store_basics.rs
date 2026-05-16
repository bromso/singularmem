//! Smoke tests for Store lifecycle: open creates schema; reopen finds it;
//! `format_version` is recorded; unsupported versions are rejected.

use singularmem_core::{FORMAT_VERSION, NewItem, Store, StoreOptions};
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
    assert_eq!(reopened.format_version().expect("read meta"), FORMAT_VERSION);
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
