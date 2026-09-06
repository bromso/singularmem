use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use singularmem_core::{IndexHook, Item, NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{Embedder, EmbedderIndex, EMBED_CHUNK};

/// Wraps `MockEmbedder` and counts how many `embed_batch` calls it receives
/// and the largest batch it saw.
struct Counting {
    inner: MockEmbedder,
    calls: Arc<AtomicUsize>,
    max_batch: Arc<AtomicUsize>,
}

impl Embedder for Counting {
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn embed(&self, c: &str) -> singularmem_search::Result<Vec<f32>> {
        self.inner.embed(c)
    }
    fn embed_batch(&self, items: &[&str]) -> singularmem_search::Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.max_batch.fetch_max(items.len(), Ordering::SeqCst);
        self.inner.embed_batch(items)
    }
}

fn seeded(
    n: usize,
) -> (
    tempfile::TempDir,
    Vec<Item>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let max_batch = Arc::new(AtomicUsize::new(0));
    let emb = Counting {
        inner: MockEmbedder::default(),
        calls: calls.clone(),
        max_batch: max_batch.clone(),
    };
    let idx = EmbedderIndex::open(dir.path().join("v"), Box::new(emb)).unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(idx)).unwrap();
    let items = store
        .ingest_many((0..n).map(|i| NewItem::text(format!("text number {i}"))))
        .unwrap();
    drop(store);
    (dir, items, calls, max_batch)
}

#[test]
fn batch_ingest_embeds_in_chunks_of_embed_chunk() {
    let (_d, _items, calls, max_batch) = seeded(150);
    // 150 items -> 64 + 64 + 22 = 3 embed_batch calls
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(max_batch.load(Ordering::SeqCst), EMBED_CHUNK);
}

#[test]
fn batch_vectors_equal_per_item_vectors() {
    let (dir, items, _c, _m) = seeded(10);
    let idx = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let mock = MockEmbedder::default();
    for item in &items {
        let expected = mock.embed(&item.content).unwrap();
        let hits = idx.vector_index().search(&expected, 1).unwrap();
        assert_eq!(
            hits[0].id, item.id,
            "nearest neighbour of an item's own vector is itself"
        );
    }
}

#[test]
fn every_item_is_present_after_batch_ingest() {
    let (dir, items, _c, _m) = seeded(150);
    let idx = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    for item in &items {
        assert!(idx.vector_index().contains(item.id));
    }
}

/// Wraps `MockEmbedder` but drops the last vector of the result on one
/// specific `embed_batch` call (`drop_on_call`, 0-indexed), simulating an
/// `Embedder::embed_batch` implementation that returns a short result.
/// Every other call behaves normally.
struct DropsLastVector {
    inner: MockEmbedder,
    calls: AtomicUsize,
    drop_on_call: usize,
}

impl Embedder for DropsLastVector {
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn embed(&self, c: &str) -> singularmem_search::Result<Vec<f32>> {
        self.inner.embed(c)
    }
    fn embed_batch(&self, items: &[&str]) -> singularmem_search::Result<Vec<Vec<f32>>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut vectors = self.inner.embed_batch(items)?;
        if call == self.drop_on_call {
            vectors.pop();
        }
        Ok(vectors)
    }
}

#[test]
fn on_ingest_batch_errors_on_short_embed_batch_result_but_keeps_earlier_chunks() {
    // Wire the fake into a real store and ingest two full chunks' worth of
    // items. `ingest_many` must still return `Ok`: a failing hook is logged
    // and swallowed by the store, not propagated to the caller.
    let store_dir = tempfile::tempdir().unwrap();
    let store_idx = EmbedderIndex::open(
        store_dir.path().join("v"),
        Box::new(DropsLastVector {
            inner: MockEmbedder::default(),
            calls: AtomicUsize::new(0),
            drop_on_call: 1,
        }),
    )
    .unwrap();
    let store = Store::open_with_hook(store_dir.path().join("s.db"), Box::new(store_idx)).unwrap();
    let items: Vec<Item> = store
        .ingest_many(
            (0..(EMBED_CHUNK * 2)).map(|i| NewItem::text(format!("short-result item {i}"))),
        )
        .expect("ingest_many succeeds even when the IndexHook fails internally");
    drop(store);

    // Now call the hook directly on a fresh `EmbedderIndex` built over an
    // identically-misbehaving fake, so the hook error is directly
    // observable (not just logged-and-swallowed as above).
    let dir = tempfile::tempdir().unwrap();
    let idx = EmbedderIndex::open(
        dir.path().join("v"),
        Box::new(DropsLastVector {
            inner: MockEmbedder::default(),
            calls: AtomicUsize::new(0),
            drop_on_call: 1,
        }),
    )
    .unwrap();

    let err = IndexHook::on_ingest_batch(&idx, &items).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("embed_batch returned a short result"),
        "error should surface the short-result guard, got: {msg}"
    );

    // The first chunk fully succeeded before the second (short) chunk
    // failed; those items must still be indexed.
    for item in &items[..EMBED_CHUNK] {
        assert!(
            idx.vector_index().contains(item.id),
            "item from the first (successful) chunk should still be indexed"
        );
    }
}
