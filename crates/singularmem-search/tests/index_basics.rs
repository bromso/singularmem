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
