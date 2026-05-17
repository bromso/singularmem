//! Integration test: ingest via `Store` with `EmbedderIndex` hook, then
//! re-open the index and run `semantic_search`.

use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, SemanticSearchOptions};
use tempfile::TempDir;

#[test]
fn ingest_then_semantic_search_finds_item() {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let vectors_path = dir.path().join("vectors");

    let embedder_idx =
        EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
    let store = Store::open_with_hook(&store_path, Box::new(embedder_idx)).unwrap();
    let item = store
        .ingest(NewItem::text("the cat sat on the mat"))
        .unwrap();
    drop(store);

    // Re-open EmbedderIndex for search; the hook's instance is now dropped.
    let embedder_idx =
        EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
    let results = embedder_idx
        .semantic_search("the cat sat on the mat", &SemanticSearchOptions::default())
        .unwrap();
    assert!(results.hits.iter().any(|h| h.id == item.id));
}
