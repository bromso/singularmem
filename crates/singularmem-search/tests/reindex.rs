//! Reindex driver: empty index → reindex from store → search works.

use singularmem_core::{NewItem, Store};
use singularmem_search::{Index, Query, SearchOptions};
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;

#[test]
fn reindex_from_empty_store_succeeds_with_zero_count() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let index = Index::open(dir.path().join("idx")).unwrap();

    let count = index
        .reindex_from(store.list().unwrap().filter_map(Result::ok), |_| {})
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(index.doc_count().unwrap(), 0);
}

#[test]
fn reindex_from_populated_store_rebuilds_index() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let store = Store::open(&store_path).unwrap();

    for i in 0..5 {
        store.ingest(NewItem::text(format!("item {i}"))).unwrap();
    }

    let index_path = dir.path().join("idx");
    let index = Index::open(&index_path).unwrap();
    let progress_calls = AtomicU64::new(0);
    let count = index
        .reindex_from(store.list().unwrap().filter_map(Result::ok), |_n| {
            progress_calls.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    assert_eq!(count, 5);

    std::thread::sleep(std::time::Duration::from_millis(150));
    let query = Query::parse("item").unwrap();
    let results = index.search(&query, SearchOptions::default()).unwrap();
    assert_eq!(results.total_matched, 5);
}
