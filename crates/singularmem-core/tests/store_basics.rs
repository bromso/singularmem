//! Smoke tests for Store lifecycle: open creates schema; reopen finds it;
//! `format_version` is recorded; unsupported versions are rejected.

use singularmem_core::{FORMAT_VERSION, Store, StoreOptions};
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
