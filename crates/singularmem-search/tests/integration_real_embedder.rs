//! Real `FastembedEmbedder` integration tests. These hit the network on
//! first run to download model weights (~80 MB). Skipped by default;
//! run with `cargo test --test integration_real_embedder -- --ignored`.

use singularmem_search::{Embedder, FastembedEmbedder};

#[test]
#[ignore = "downloads ~80 MB model on first run; run manually with --ignored"]
fn fastembed_default_model_works() {
    let e = FastembedEmbedder::new().expect("construct (may download model)");
    assert_eq!(e.dim(), 384);
    let v = e.embed("hello world").unwrap();
    assert_eq!(v.len(), 384);
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.01, "vector should be unit-length, got norm={norm}");
}

#[test]
#[ignore = "downloads ~80 MB model on first run; run manually with --ignored"]
fn fastembed_is_deterministic() {
    let e = FastembedEmbedder::new().expect("construct");
    let v1 = e.embed("the quick brown fox").unwrap();
    let v2 = e.embed("the quick brown fox").unwrap();
    assert_eq!(v1, v2);
}
