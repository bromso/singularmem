//! Property tests for embedding invariants:
//! - Determinism: identical input always yields byte-identical output.
//! - Self-similarity: an ingested item is its own nearest neighbour with
//!   cosine similarity >= 0.95.

use proptest::prelude::*;
use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, SemanticSearchOptions};
use tempfile::TempDir;

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

    /// Embedding is deterministic given identical input.
    #[test]
    fn embed_is_deterministic(content in "[a-zA-Z ]{1,200}") {
        use singularmem_search::Embedder;
        let e = MockEmbedder::default();
        let v1 = e.embed(&content).unwrap();
        let v2 = e.embed(&content).unwrap();
        prop_assert_eq!(v1, v2);
    }

    /// An ingested item's embedding is its own nearest neighbour with high score.
    #[test]
    fn ingest_then_semantic_search_finds_self(content in "[a-zA-Z ]{20,200}") {
        let dir = TempDir::new().unwrap();
        let vectors_path = dir.path().join("v");
        let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(dir.path().join("store.db"), Box::new(embedder_idx)).unwrap();
        let item = store.ingest(NewItem::text(content.clone())).unwrap();
        drop(store);

        let embedder_idx = EmbedderIndex::open(&vectors_path, Box::new(MockEmbedder::default())).unwrap();
        let results = embedder_idx.semantic_search(&content, &SemanticSearchOptions::default()).unwrap();
        prop_assert!(
            results.hits.iter().any(|h| h.id == item.id && h.score > 0.95),
            "self-similarity should be ~1.0; hits: {:?}",
            results.hits
        );
    }
}
