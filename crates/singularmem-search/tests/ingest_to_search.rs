//! End-to-end tests: ingest items via Store with the Index hook attached,
//! then verify `Index::search` finds them.

use singularmem_core::{NewItem, Store};
use singularmem_search::{Index, Query, SearchOptions};
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

/// Set up a store backed by a search index. Returns the `TempDir` (to keep
/// paths alive), the store, and the index directory path.
/// The index is wired as a hook so every `ingest` call updates it.
fn store_with_index() -> (TempDir, Store, PathBuf) {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let index_path = dir.path().join("store.db.tantivy");
    let index = Index::open(&index_path).expect("open index");
    let store = Store::open_with_hook(&store_path, Box::new(index)).expect("open store with hook");
    (dir, store, index_path)
}

/// Tantivy's reload policy is async; give the reader a moment to see new commits.
fn wait_for_index_visibility() {
    std::thread::sleep(Duration::from_millis(150));
}

#[test]
fn ingest_then_search_returns_the_item() {
    let (dir, store, index_path) = store_with_index();
    let item = store
        .ingest(NewItem::text("Decision: use SQLite for v0"))
        .unwrap();

    // Drop the store (and hook) so the writer lock is released before
    // opening a second Index for reading.
    drop(store);
    wait_for_index_visibility();

    let index = Index::open(&index_path).unwrap();
    let query = Query::parse("decision").unwrap();
    let results = index.search(&query, SearchOptions::default()).unwrap();

    assert_eq!(results.total_matched, 1);
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].id, item.id);
    assert!(results.hits[0].score > 0.0);

    drop(dir);
}

#[test]
fn search_with_no_matches_returns_empty_results() {
    let (dir, store, index_path) = store_with_index();
    let _ = store.ingest(NewItem::text("nothing to find here")).unwrap();
    drop(store);
    wait_for_index_visibility();

    let index = Index::open(&index_path).unwrap();
    let query = Query::parse("missing").unwrap();
    let results = index.search(&query, SearchOptions::default()).unwrap();

    assert_eq!(results.total_matched, 0);
    assert!(results.hits.is_empty());
    drop(dir);
}

#[test]
fn search_respects_limit() {
    let (dir, store, index_path) = store_with_index();
    for i in 0..10 {
        store
            .ingest(NewItem::text(format!("note {i} about decisions")))
            .unwrap();
    }
    drop(store);
    wait_for_index_visibility();

    let index = Index::open(&index_path).unwrap();
    let query = Query::parse("decisions").unwrap();
    let opts = SearchOptions {
        limit: 3,
        offset: 0,
        include_snippets: false,
    };
    let results = index.search(&query, opts).unwrap();

    assert_eq!(results.total_matched, 10);
    assert_eq!(results.hits.len(), 3);
    drop(dir);
}

#[test]
fn search_with_snippets_returns_marked_text() {
    let (dir, store, index_path) = store_with_index();
    let _ = store
        .ingest(NewItem::text("This is a long sentence containing the word decision and more text"))
        .unwrap();
    drop(store);
    wait_for_index_visibility();

    let index = Index::open(&index_path).unwrap();
    let query = Query::parse("decision").unwrap();
    let results = index.search(&query, SearchOptions::default()).unwrap();

    assert_eq!(results.hits.len(), 1);
    let snippet = results.hits[0].snippet.as_deref().expect("snippet present by default");
    assert!(snippet.contains("<mark>") || snippet.contains("<b>"),
            "snippet should contain highlight markers: {snippet}");
    drop(dir);
}
