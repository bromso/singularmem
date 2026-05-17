//! Test fixtures. Available in any `#[test]` build and behind the `testing`
//! feature flag for cross-crate test reuse.
//!
//! The `MockEmbedder` type is gated by `#[cfg(any(test, feature = "testing"))]`
//! so it is compiled when:
//! - Running unit tests inside this crate (`cfg(test)` is true for the lib), or
//! - Any crate enables the `testing` feature of `singularmem-search`.
//!
//! Integration tests under `tests/` are compiled as separate binaries that
//! import the library without `cfg(test)`. Those tests must pass
//! `--features singularmem-search/testing` or depend on the crate with the
//! `testing` feature — OR the test binary itself compiles `testing` because
//! `cfg(test)` is true for the test binary's compilation of this type.
//!
//! In practice Cargo integration tests compile the lib without `cfg(test)`,
//! so the test binary needs `--features testing` or the module must be
//! unconditionally available. We unconditionally export `MockEmbedder` to
//! keep the plain `cargo test` command working without extra flags.

use crate::embedder::Embedder;
use crate::error::Result;

/// Deterministic-pseudo-hash `Embedder` implementation for tests.
///
/// - `dim()` returns 384 (matches all-MiniLM-L6-v2 so `VectorIndex` schema
///   tests don't need to vary on dim).
/// - `embed(s)` returns a 384-dim vector derived from `s` via a fast hash;
///   same `s` → byte-identical vector.
/// - No ONNX runtime, no model download, no network.
pub struct MockEmbedder {
    dim: usize,
    model_id: String,
}

impl MockEmbedder {
    /// Construct a `MockEmbedder` with a custom dimension.
    #[must_use]
    pub fn with_dim(dim: usize) -> Self {
        Self {
            dim,
            model_id: "mock-embedder@v1".to_string(),
        }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::with_dim(384)
    }
}

impl Embedder for MockEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn embed(&self, content: &str) -> Result<Vec<f32>> {
        // Deterministic pseudo-hash: seed a small PRNG with a hash of the
        // input, draw `dim` floats in [-1, 1], normalize to unit length.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let mut seed = hasher.finish();

        let mut v = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            // xorshift64 step
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // Map to [-1, 1]. The wrapping reinterpretation from u64 to i64 is
            // intentional: it gives us signed integers for the full range.
            let signed = i64::from_ne_bytes(seed.to_ne_bytes());
            #[allow(clippy::cast_precision_loss)]
            let f = signed as f32 / i64::MAX as f32;
            v.push(f);
        }
        // L2-normalize.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}
