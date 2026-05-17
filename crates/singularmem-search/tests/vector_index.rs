//! Tests for `VectorIndex`: open, model-mismatch detection, meta persistence,
//! `add`/`remove`/`save`/`doc_count`, and `KNN` search.

use singularmem_search::testing::MockEmbedder;
use singularmem_search::{Embedder, VectorIndex, VectorIndexOptions};
use tempfile::TempDir;

// ── Task 5: open + meta + model-mismatch ──────────────────────────────────

#[test]
fn open_fresh_creates_meta_and_index_files() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let _ = VectorIndex::open(dir.path().join("vectors"), &e).expect("open fresh");

    let meta_path = dir.path().join("vectors").join(".meta.json");
    assert!(meta_path.exists(), "meta.json should be created");
    // index.usearch may not exist until first save — that's fine.
}

#[test]
fn reopen_with_same_model_succeeds() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let path = dir.path().join("vectors");
    let _ = VectorIndex::open(&path, &e).expect("open fresh");
    let _ = VectorIndex::open(&path, &e).expect("reopen with same model");
}

/// A stub `Embedder` with a different `model_id` than `MockEmbedder`, used to
/// trigger `Error::ModelMismatch` detection in the test below.
struct OtherMock;

impl Embedder for OtherMock {
    fn dim(&self) -> usize {
        384
    }

    fn model_id(&self) -> &'static str {
        "different-model@v1"
    }

    fn embed(&self, _: &str) -> singularmem_search::Result<Vec<f32>> {
        unimplemented!()
    }
}

#[test]
fn reopen_with_different_model_returns_model_mismatch() {
    let dir = TempDir::new().unwrap();
    let e1 = MockEmbedder::default();
    let path = dir.path().join("vectors");
    let _ = VectorIndex::open(&path, &e1).expect("open with e1");
    drop(e1);

    let result = VectorIndex::open(&path, &OtherMock);
    match result {
        Err(singularmem_search::Error::ModelMismatch { found_model, expected_model, .. }) => {
            assert_eq!(found_model, "mock-embedder@v1");
            assert_eq!(expected_model, "different-model@v1");
        }
        other => panic!("expected ModelMismatch, got {other:?}"),
    }
}

#[test]
fn open_with_options_respects_hnsw_params() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let opts = VectorIndexOptions { hnsw_m: 32, hnsw_ef_construction: 256, expansion_search: 128 };
    let idx = VectorIndex::open_with_options(dir.path().join("v"), &e, opts).unwrap();
    assert_eq!(idx.meta().hnsw_m, 32);
}

// ── Task 6: add / remove / save / doc_count ───────────────────────────────

use singularmem_core::ItemId;
use std::str::FromStr;

#[test]
fn add_then_save_then_reopen_preserves_vectors() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let path = dir.path().join("v");
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    {
        let idx = VectorIndex::open(&path, &e).unwrap();
        let v = e.embed("hello").unwrap();
        idx.add(id, &v).expect("add");
        idx.save().expect("save");
        assert_eq!(idx.doc_count().unwrap(), 1);
    }
    let idx2 = VectorIndex::open(&path, &e).unwrap();
    assert_eq!(idx2.doc_count().unwrap(), 1);
}

#[test]
fn add_with_wrong_dim_returns_dim_mismatch() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default(); // 384
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let too_small = vec![0.0_f32; 128];
    let err = idx.add(id, &too_small).unwrap_err();
    assert!(matches!(err, singularmem_search::Error::DimMismatch { expected: 384, got: 128 }));
}

#[test]
fn remove_absent_id_is_noop() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    idx.remove(id).expect("remove of absent ID is no-op, not error");
}

#[test]
fn add_increments_doc_count_and_remove_decrements_it() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
    let id = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(idx.doc_count().unwrap(), 0, "fresh index has zero docs");
    assert!(!idx.contains(id), "id not yet added");

    let v = e.embed("hello").unwrap();
    idx.add(id, &v).unwrap();
    assert_eq!(idx.doc_count().unwrap(), 1);
    assert!(idx.contains(id), "id should be present after add");

    idx.remove(id).unwrap();
    assert_eq!(idx.doc_count().unwrap(), 0, "remove should decrement doc_count");
    assert!(!idx.contains(id), "id should be absent after remove");
}

// ── Task 7: search (KNN) ──────────────────────────────────────────────────

#[test]
fn search_returns_nearest_neighbours_by_cosine() {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();

    let id1 = ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let id2 = ItemId::from_str("01BX5ZZKBKACTAV9WEVGEMMVRZ").unwrap();
    let v1 = e.embed("hello world").unwrap();
    let v2 = e.embed("totally different text").unwrap();
    idx.add(id1, &v1).unwrap();
    idx.add(id2, &v2).unwrap();
    idx.save().unwrap();

    let query = e.embed("hello world").unwrap();
    let hits = idx.search(&query, 2).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, id1, "highest similarity = same vector = id1");
    assert!(hits[0].score > 0.99, "self-similarity should be ~1.0, got {}", hits[0].score);
}
