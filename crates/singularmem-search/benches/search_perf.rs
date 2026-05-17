//! Criterion benches feeding the perf-budgets CI gate.
//!
//! Two benches:
//! - `search_latency_p95`: BM25 query latency over a 10 K-doc store.
//! - `reindex_throughput`: full-rebuild time for 100 and 1000-item stores.
//!
//! `.github/scripts/perf-check.sh` reads the per-bench
//! `target/criterion/<bench>/new/estimates.json` files produced here.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{EmbedderIndex, Index, Query, SearchOptions, SemanticSearchOptions};
use tempfile::TempDir;

/// Seed `n` items into a fresh store+index (hook-wired) and return the
/// dir guard + a search-ready `Index` opened on the same path.
fn seed_store_and_index(n: usize) -> (TempDir, Index) {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let index_path = dir.path().join("idx");
    let hook_index = Index::open(&index_path).unwrap();
    let store = Store::open_with_hook(&store_path, Box::new(hook_index)).unwrap();
    let items: Vec<NewItem> = (0..n)
        .map(|i| NewItem::text(format!("benchmark item number {i} with content")))
        .collect();
    store.ingest_many(items).unwrap();
    // Let Tantivy's async reader reload settle before we start measuring.
    std::thread::sleep(std::time::Duration::from_millis(200));
    // Drop the store to release the hook writer lock, then open a fresh Index
    // for the search measurement (Tantivy allows only one writer per directory).
    drop(store);
    let search_index = Index::open(&index_path).unwrap();
    (dir, search_index)
}

fn bench_search_latency(c: &mut Criterion) {
    let (_dir, index) = seed_store_and_index(10_000);
    let query = Query::parse("benchmark").unwrap();
    c.bench_function("search_latency_p95", |b| {
        b.iter(|| {
            let _ = index.search(&query, SearchOptions::default()).unwrap();
        });
    });
}

fn bench_reindex_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("reindex_throughput");
    for n in [100_usize, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let dir = TempDir::new().unwrap();
            let store_path = dir.path().join("store.db");
            let store = Store::open(&store_path).unwrap();
            let items: Vec<NewItem> = (0..n).map(|i| NewItem::text(format!("item {i}"))).collect();
            store.ingest_many(items).unwrap();

            b.iter(|| {
                let dir2 = TempDir::new().unwrap();
                let index = Index::open(dir2.path().join("idx")).unwrap();
                let count = index
                    .reindex_from(store.list().unwrap().filter_map(Result::ok), |_| {})
                    .unwrap();
                assert_eq!(count, n as u64);
            });
        });
    }
    group.finish();
}

fn bench_embed_throughput(c: &mut Criterion) {
    use singularmem_search::Embedder;
    let e = MockEmbedder::default();
    c.bench_function("embed_throughput", |b| {
        b.iter(|| {
            e.embed("benchmark item with moderate content length")
                .unwrap()
        });
    });
}

fn bench_semantic_search_latency(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let embedder_idx =
        EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let store = Store::open_with_hook(dir.path().join("store.db"), Box::new(embedder_idx)).unwrap();
    for i in 0..10_000 {
        store
            .ingest(NewItem::text(format!("seed item number {i}")))
            .unwrap();
    }
    drop(store);
    let embedder_idx =
        EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    c.bench_function("semantic_search_latency", |b| {
        b.iter(|| {
            embedder_idx
                .semantic_search("seed item number 5000", &SemanticSearchOptions::default())
                .unwrap()
        });
    });
}

criterion_group!(
    benches,
    bench_search_latency,
    bench_reindex_throughput,
    bench_embed_throughput,
    bench_semantic_search_latency,
);
criterion_main!(benches);
