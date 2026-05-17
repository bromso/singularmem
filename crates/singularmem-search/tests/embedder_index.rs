//! Tests for `EmbedderIndex`: impl `IndexHook` backed by `Embedder` + `VectorIndex`.

use jiff::Timestamp;
use singularmem_core::{IndexHook, Item, ItemId};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::EmbedderIndex;
use std::str::FromStr;
use tempfile::TempDir;

#[test]
fn on_ingest_then_commit_increments_doc_count() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("v");
    let idx = EmbedderIndex::open(&path, Box::new(MockEmbedder::default())).unwrap();
    let item = Item {
        id: ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
        content: "hello".into(),
        created_at: Timestamp::now(),
        supersedes: None,
        tags: vec![],
        source: None,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
    };
    idx.on_ingest(&item).unwrap();
    idx.commit().unwrap();
    assert_eq!(idx.vector_index().doc_count().unwrap(), 1);
}
