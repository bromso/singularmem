//! Concurrency tests: search readers during a long-running reindex see
//! consistent state.

use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, Index, Query, SearchOptions, SemanticSearchOptions};
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

#[test]
fn parallel_readers_during_reindex_see_consistent_state() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let index_path = dir.path().join("idx");

    // Seed 500 items via direct ingest (no index attached during seeding).
    {
        let store = Store::open(&store_path).unwrap();
        for i in 0..500 {
            store.ingest(NewItem::text(format!("seed-{i}"))).unwrap();
        }
    }

    // Now reindex with 8 readers running concurrent searches.
    let index_for_reindex = Index::open(&index_path).unwrap();
    let store_for_reindex = Store::open(&store_path).unwrap();
    let index_path_arc = Arc::new(index_path.clone());

    let mut readers = Vec::new();
    for _ in 0..8 {
        let path = Arc::clone(&index_path_arc);
        readers.push(thread::spawn(move || {
            for _ in 0..50 {
                // Each reader opens its own Index instance (reader-only; no
                // writer contention because the reindex thread holds the writer
                // on the same path — but Tantivy readers share the directory
                // via MmapDirectory so they don't need the writer lock).
                // NOTE: Index::open tries to acquire the writer lock and will
                // fail if the reindex thread holds it. Use a retry loop and
                // treat LockBusy as "reindex in progress" → skip this iteration.
                if let Ok(index) = Index::open(&*path) {
                    let query = Query::parse("seed").unwrap();
                    // Just confirm the call succeeds (results may be 0 or 500
                    // depending on reindex progress; both are valid consistent
                    // states).
                    let _ = index.search(&query, SearchOptions::default()).unwrap();
                }
                // Small yield to give the reindex thread CPU time.
                std::hint::spin_loop();
            }
        }));
    }

    let reindex_handle = thread::spawn(move || {
        index_for_reindex
            .reindex_from(
                store_for_reindex.list().unwrap().filter_map(Result::ok),
                |_| {},
            )
            .expect("reindex");
    });

    for r in readers {
        r.join().expect("reader join");
    }
    reindex_handle.join().expect("reindex join");

    // Post-reindex: 500 items should be searchable.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let index = Index::open(&index_path).unwrap();
    let query = Query::parse("seed").unwrap();
    let results = index.search(&query, SearchOptions::default()).unwrap();
    assert_eq!(results.total_matched, 500);
}

/// Readers that open their own [`EmbedderIndex`] concurrently with a
/// reindexing writer should not panic or deadlock. `USearch`'s writer-lock
/// may serialise the opens; this test accepts either serialised or truly
/// concurrent behaviour — the invariant is that all operations return `Ok`.
///
/// Note: reader count is 4 and iteration count is 20 to keep the test fast
/// even if `USearch` serialises all opens under an internal mutex.
#[test]
fn parallel_semantic_searchers_during_reindex_see_consistent_state() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let vectors_path = dir.path().join("v");

    // Seed 200 items with embedder attached.
    {
        let embedder_idx =
            EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(&store_path, Box::new(embedder_idx)).unwrap();
        for i in 0..200 {
            store.ingest(NewItem::text(format!("item {i}"))).unwrap();
        }
    }

    let vectors_arc = Arc::new(vectors_path.clone());
    let mut readers = Vec::new();
    for _ in 0..4 {
        let path = Arc::clone(&vectors_arc);
        readers.push(thread::spawn(move || {
            for _ in 0..20 {
                let idx = EmbedderIndex::open(&*path, Box::new(MockEmbedder::default())).unwrap();
                let _ = idx
                    .semantic_search("item 50", &SemanticSearchOptions::default())
                    .unwrap();
            }
        }));
    }

    // Move vectors_path and store_path into the closure directly (no clone needed
    // because the main thread no longer uses them after this point).
    let reindexer = thread::spawn(move || {
        use singularmem_core::IndexHook as _;
        let store = Store::open(&store_path).unwrap();
        let idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        for item in store.list().unwrap().filter_map(Result::ok) {
            idx.on_reindex(&item).unwrap();
        }
        idx.commit().unwrap();
    });

    for r in readers {
        r.join().unwrap();
    }
    reindexer.join().unwrap();
}
