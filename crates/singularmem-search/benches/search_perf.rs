//! Criterion benches feeding the perf-budgets CI gate.
//!
//! Benches:
//! - `search_latency_p95`: BM25 query latency over a 10 K-doc store.
//! - `reindex_throughput`: full-rebuild time for 100 and 1000-item stores.
//! - `ingest_throughput/ingest_with_indexes`: bulk `ingest_many` throughput
//!   with a lexical + semantic (mock-embedded) hook attached.
//! - `ingest_throughput/ingest_single_with_vector_index`: single-item
//!   `ingest` cost with **only** the `EmbedderIndex` hook attached, at
//!   20,000 pre-seeded vectors — proves the journal makes the vector
//!   index's own per-item cost independent of index size (sub-project 17,
//!   `docs/superpowers/specs/2026-09-06-ingest-throughput-17-design.md`
//!   § "Part 3"). This is the one the CI gate enforces: the sibling bench
//!   below adds a Tantivy `Index` hook whose per-item `commit` cost is
//!   pre-existing and out of this sub-project's scope, so it would make the
//!   gate fail on a cost this work didn't introduce and can't fix.
//! - `ingest_throughput/ingest_single_with_both_hooks`: the same single-item
//!   `ingest`, but through a `MultiHook` of a Tantivy `Index` **and** an
//!   `EmbedderIndex` — i.e. what a real ingest path looks like. Ungated;
//!   reported informationally by `.github/scripts/perf-check.sh` so the
//!   Tantivy-dominated combined cost stays visible without gating on it.
//! - `open_with_journal`: `VectorIndex::open` on a 20,000-vector compacted
//!   directory with `COMPACT_THRESHOLD - 1` journal records waiting to be replayed — the
//!   read-side cost the journal buys the write side with. Ungated,
//!   informational; see `docs/benchmarks/ingest.md` § "Read-side cost".
//!
//! `.github/scripts/perf-check.sh` reads the per-bench
//! `target/criterion/<bench>/new/estimates.json` files produced here.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::COMPACT_THRESHOLD;
use singularmem_search::{
    Embedder, EmbedderIndex, HybridSearcher, Index, Query, SearchOptions, SemanticSearchOptions,
    VectorIndex,
};
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

fn bench_hybrid_search_latency(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store_path = dir.path().join("store.db");
    let lex_path = dir.path().join("lex");
    let sem_path = dir.path().join("sem");

    let lex_hook = Index::open(&lex_path).unwrap();
    let sem_hook = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
    let multi =
        singularmem_core::hook::MultiHook::new(vec![Box::new(lex_hook), Box::new(sem_hook)]);
    let store = Store::open_with_hook(&store_path, Box::new(multi)).unwrap();
    for i in 0..1_000 {
        store
            .ingest(NewItem::text(format!("benchmark hybrid item number {i}")))
            .unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(store);

    let lex = Index::open(&lex_path).unwrap();
    let sem = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
    let searcher = HybridSearcher::new(&lex, &sem);
    let opts = singularmem_search::HybridSearchOptions::default();

    c.bench_function("hybrid_search_latency", |b| {
        b.iter(|| {
            let _ = searcher.search("benchmark hybrid item", &opts).unwrap();
        });
    });
}

/// A ~1,500-char realistic conversational turn, so the bench's per-item cost
/// approximates a real transcript ingest rather than a short synthetic string.
fn realistic(i: usize) -> String {
    format!("assistant: {i} ")
        + &"We discussed the migration plan for the store format and agreed to keep append-only revisions; the reviewer asked for a doc-count guard after ingest. ".repeat(9)
}

/// Gates the constitution's Principle X ingest-throughput floor
/// (`.github/scripts/perf-check.sh`) with both indexes attached: a Tantivy
/// `Index` hook and an `EmbedderIndex` over `MockEmbedder`. Each iteration
/// bulk-ingests 100 realistic-length items through `ingest_many` into a fresh
/// store, so it measures one `on_ingest_batch` + one `commit` (one journal
/// append + one compaction, per the design's batch-end rule).
fn bench_ingest_with_indexes(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_throughput");
    group.throughput(criterion::Throughput::Elements(100));
    group.bench_function("ingest_with_indexes", |b| {
        b.iter_batched(
            || {
                let dir = TempDir::new().unwrap();
                let lex = Index::open(dir.path().join("lex")).unwrap();
                let sem =
                    EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default()))
                        .unwrap();
                let multi =
                    singularmem_core::hook::MultiHook::new(vec![Box::new(lex), Box::new(sem)]);
                let store =
                    Store::open_with_hook(dir.path().join("s.db"), Box::new(multi)).unwrap();
                (dir, store)
            },
            |(_dir, store)| {
                store
                    .ingest_many((0..100).map(|i| NewItem::text(realistic(i))))
                    .unwrap();
            },
            criterion::BatchSize::PerIteration,
        );
    });
    group.finish();
}

/// Pre-seed a fresh directory with 20,000 mock-embedded vectors via **only**
/// the `EmbedderIndex` hook, then drop that store (releasing the vector
/// index's file lock) so the caller can reopen the same directory with
/// whichever hook(s) it wants to measure. Shared by both single-item ingest
/// benches below so the corpus is identical between them.
fn seed_vector_dir(n: usize) -> TempDir {
    let dir = TempDir::new().unwrap();
    let sem = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(sem)).unwrap();
    for chunk in (0..n).collect::<Vec<_>>().chunks(500) {
        store
            .ingest_many(chunk.iter().map(|i| NewItem::text(format!("seed {i}"))))
            .unwrap();
    }
    dir
}

/// Gates the size-independence acceptance criterion: single-item
/// `Store::ingest` through **only** the `EmbedderIndex` hook, into a store
/// pre-seeded with 20,000 vectors. Before the journal, every commit
/// rewrote the whole `index.usearch` file, so this cost scaled with index
/// size; after it, a single-item commit only appends to `journal.bin`
/// (compacting once every `COMPACT_THRESHOLD` records — the note below is
/// why the median, not the max, is the right statistic here).
///
/// This is the bench `.github/scripts/perf-check.sh` gates at ≤ 20 ms: it
/// isolates the vector index's own per-item cost from the Tantivy `Index`
/// hook's pre-existing, out-of-scope per-item `commit` cost (see the
/// sibling `ingest_single_with_both_hooks` bench below, which is ungated
/// for that reason).
///
/// Note the bench crosses the compaction threshold every `COMPACT_THRESHOLD` iterations;
/// that is intended (the gate is a median, and the amortised cost is what
/// users see).
fn bench_ingest_single_with_vector_index(c: &mut Criterion) {
    let dir = seed_vector_dir(20_000);
    let sem = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(sem)).unwrap();
    let mut group = c.benchmark_group("ingest_throughput");
    let mut n = 0usize;
    group.bench_function("ingest_single_with_vector_index", |b| {
        b.iter(|| {
            n += 1;
            store.ingest(NewItem::text(realistic(n))).unwrap();
        });
    });
    group.finish();
}

/// Same single-item ingest, but through a `MultiHook` of a Tantivy `Index`
/// **and** an `EmbedderIndex` — the shape a real ingest path uses. Ungated:
/// Tantivy commits a whole segment on every single-item `commit()` call (a
/// pre-existing cost, unrelated to this sub-project and already present
/// before any of Tasks 1–4), and it dominates this combined figure
/// (~88 ms of the ~99 ms total on the measurement machine, see
/// `docs/benchmarks/ingest.md`). `.github/scripts/perf-check.sh` prints
/// this bench's median informationally, without gating on it.
fn bench_ingest_single_with_both_hooks(c: &mut Criterion) {
    let dir = seed_vector_dir(20_000);
    let lex = Index::open(dir.path().join("lex")).unwrap();
    let sem = EmbedderIndex::open(dir.path().join("v"), Box::new(MockEmbedder::default())).unwrap();
    let multi = singularmem_core::hook::MultiHook::new(vec![Box::new(lex), Box::new(sem)]);
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(multi)).unwrap();
    let mut group = c.benchmark_group("ingest_throughput");
    let mut n = 0usize;
    group.bench_function("ingest_single_with_both_hooks", |b| {
        b.iter(|| {
            n += 1;
            store.ingest(NewItem::text(realistic(n))).unwrap();
        });
    });
    group.finish();
}

/// Seed a vector directory with `compacted` compacted vectors and then
/// `journalled` more sitting in `journal.bin` (a `commit(false)` below the
/// `COMPACT_THRESHOLD`-record compaction threshold), using `VectorIndex` directly rather
/// than a `Store` so the setup costs only the embedding and the graph
/// inserts.
fn seed_journalled_vector_dir(compacted: usize, journalled: usize) -> TempDir {
    let dir = TempDir::new().unwrap();
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(dir.path().join("v"), &e).unwrap();
    for chunk in (0..compacted).collect::<Vec<_>>().chunks(1_000) {
        let vectors: Vec<Vec<f32>> = chunk
            .iter()
            .map(|i| Embedder::embed(&e, &format!("seed {i}")).unwrap())
            .collect();
        let entries: Vec<(singularmem_core::ItemId, &[f32])> = vectors
            .iter()
            .map(|v| (ulid::Ulid::new().to_string().parse().unwrap(), v.as_slice()))
            .collect();
        idx.add_batch(&entries).unwrap();
    }
    idx.compact().unwrap();
    for i in 0..journalled {
        let v = Embedder::embed(&e, &format!("journalled {i}")).unwrap();
        idx.add(ulid::Ulid::new().to_string().parse().unwrap(), &v)
            .unwrap();
    }
    idx.commit(false).unwrap();
    assert_eq!(idx.journal_len().unwrap(), journalled);
    drop(idx);
    dir
}

/// The read side of the journal trade: `VectorIndex::open` has to replay
/// every journal record into the loaded graph, so open latency grows with
/// journal length while commit latency stops growing with index size.
///
/// Informational only — **not gated**. `COMPACT_THRESHOLD` is what bounds
/// this cost (a journal never holds more than that many records before a
/// commit compacts it), and any bulk `ingest_many` ends with a compacting
/// commit that resets it to zero. `COMPACT_THRESHOLD - 1` records is
/// therefore the worst case a reader can encounter.
fn bench_open_with_journal(c: &mut Criterion) {
    let dir = seed_journalled_vector_dir(20_000, COMPACT_THRESHOLD - 1);
    let vdir = dir.path().join("v");
    let e = MockEmbedder::default();
    let mut group = c.benchmark_group("open_with_journal");
    // Each open replays COMPACT_THRESHOLD - 1 vectors into a 20,000-node HNSW graph; the
    // default 100 samples would run for minutes.
    group.sample_size(10);
    group.bench_function("open_with_journal_at_threshold", |b| {
        b.iter(|| {
            let idx = VectorIndex::open(&vdir, &e).unwrap();
            assert_eq!(idx.len(), 20_000 + COMPACT_THRESHOLD - 1);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_search_latency,
    bench_reindex_throughput,
    bench_embed_throughput,
    bench_semantic_search_latency,
    bench_hybrid_search_latency,
    bench_ingest_with_indexes,
    bench_ingest_single_with_vector_index,
    bench_ingest_single_with_both_hooks,
    bench_open_with_journal,
);
criterion_main!(benches);
