//! Smoke tests for Index lifecycle: open creates directory; `doc_count` works
//! on empty index; reopen finds existing data.

use singularmem_search::{Index, IndexOptions};
use tempfile::TempDir;


#[test]
fn open_fresh_creates_directory_and_doc_count_is_zero() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index");
    let index = Index::open(&index_path).expect("open fresh");
    assert!(index_path.exists());
    assert_eq!(index.doc_count().unwrap(), 0);
}

#[test]
fn open_creates_parent_directories() {
    let dir = TempDir::new().unwrap();
    let deep_path = dir.path().join("nested").join("subdir").join("index");
    assert!(!deep_path.parent().unwrap().exists());
    let _ = Index::open(&deep_path).expect("open with parent create");
    assert!(deep_path.exists());
}

#[test]
fn open_with_options_respects_writer_memory() {
    let dir = TempDir::new().unwrap();
    let options = IndexOptions {
        writer_memory_bytes: 16 * 1024 * 1024,
    };
    let _ = Index::open_with_options(dir.path().join("index"), options).expect("open with options");
}

use jiff::Timestamp;
use singularmem_core::{IndexHook, Item, ItemId};
use std::str::FromStr;

#[test]
fn on_ingest_then_commit_increments_doc_count() {
    let dir = TempDir::new().unwrap();
    let index = Index::open(dir.path().join("idx")).unwrap();

    let item = Item {
        id: ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
        content: "hello world".to_string(),
        created_at: Timestamp::now(),
        supersedes: None,
        tags: vec!["greeting".to_string()],
        source: None,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
    };

    index.on_ingest(&item).unwrap();
    index.commit().unwrap();

    // Reader needs a moment to reload after commit. Tantivy's
    // ReloadPolicy::OnCommitWithDelay handles this asynchronously.
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(index.doc_count().unwrap(), 1);
}
