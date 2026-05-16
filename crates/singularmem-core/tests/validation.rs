//! Integration tests covering each `Error::Validation` branch through
//! `Store::ingest`. Mirror counterparts to the unit tests in `src/item.rs`
//! but exercise the full ingest pipeline (locks, transactions, etc.).

use singularmem_core::{Error, NewItem, Store};
use tempfile::TempDir;

fn fresh_store() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    (dir, store)
}

#[test]
fn empty_content_rejected() {
    let (_dir, store) = fresh_store();
    let err = store.ingest(NewItem::text("")).unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "content",
            ..
        }
    ));
}

#[test]
fn oversized_content_rejected() {
    let (_dir, store) = fresh_store();
    let big = "x".repeat(1_048_577);
    let err = store.ingest(NewItem::text(big)).unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "content",
            ..
        }
    ));
}

#[test]
fn metadata_array_rejected() {
    let (_dir, store) = fresh_store();
    let mut item = NewItem::text("ok");
    item.metadata = serde_json::json!([1, 2, 3]);
    let err = store.ingest(item).unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "metadata",
            ..
        }
    ));
}

#[test]
fn metadata_scalar_rejected() {
    let (_dir, store) = fresh_store();
    let mut item = NewItem::text("ok");
    item.metadata = serde_json::Value::String("string-not-object".into());
    let err = store.ingest(item).unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "metadata",
            ..
        }
    ));
}

#[test]
fn empty_tag_rejected() {
    let (_dir, store) = fresh_store();
    let mut item = NewItem::text("ok");
    item.tags = vec!["valid".into(), String::new()];
    let err = store.ingest(item).unwrap_err();
    assert!(matches!(err, Error::Validation { field: "tags", .. }));
}

#[test]
fn oversized_tag_rejected() {
    let (_dir, store) = fresh_store();
    let mut item = NewItem::text("ok");
    item.tags = vec!["t".repeat(65)];
    let err = store.ingest(item).unwrap_err();
    assert!(matches!(err, Error::Validation { field: "tags", .. }));
}

#[test]
fn long_source_rejected() {
    let (_dir, store) = fresh_store();
    let mut item = NewItem::text("ok");
    item.source = Some("s".repeat(257));
    let err = store.ingest(item).unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "source",
            ..
        }
    ));
}

#[test]
fn validation_failure_leaves_store_empty() {
    let (dir, store) = fresh_store();
    let _ = store.ingest(NewItem::text(String::new())).unwrap_err();
    drop(store);
    let conn = rusqlite::Connection::open(dir.path().join("store.db")).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "validation failure must not write any row");
}
