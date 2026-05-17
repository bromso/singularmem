//! Tests for the `Embedder` trait + `MockEmbedder` fixture. `FastembedEmbedder`
//! is exercised in Task 4 and in `#[ignore]` integration tests.

use singularmem_search::testing::MockEmbedder;
use singularmem_search::Embedder;

#[test]
fn mock_embedder_has_consistent_dim() {
    let e = MockEmbedder::default();
    assert_eq!(
        e.dim(),
        384,
        "MockEmbedder uses the same default dim as all-MiniLM-L6-v2"
    );
}

#[test]
fn mock_embedder_is_deterministic() {
    let e = MockEmbedder::default();
    let v1 = e.embed("hello world").unwrap();
    let v2 = e.embed("hello world").unwrap();
    assert_eq!(v1, v2, "same input must produce byte-identical vector");
    assert_eq!(v1.len(), 384);
}

#[test]
fn mock_embedder_different_inputs_produce_different_vectors() {
    let e = MockEmbedder::default();
    let v1 = e.embed("hello").unwrap();
    let v2 = e.embed("world").unwrap();
    assert_ne!(v1, v2);
}

#[test]
fn mock_embedder_batch_matches_individual() {
    let e = MockEmbedder::default();
    let inputs = ["a", "b", "c"];
    let single: Vec<_> = inputs.iter().map(|s| e.embed(s).unwrap()).collect();
    let batched = e.embed_batch(&inputs).unwrap();
    assert_eq!(single, batched);
}

#[test]
fn mock_embedder_model_id_is_stable() {
    let e = MockEmbedder::default();
    assert_eq!(e.model_id(), "mock-embedder@v1");
}
