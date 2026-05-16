//! Criterion benches feeding the perf-budgets CI gate.
//!
//! Two benches:
//! - `ingest_throughput`: items per second when ingesting in a tight loop
//!   against a fresh store.
//! - `get_p95`: point-read latency p95 over a pre-seeded store of 10 000 items.
//!
//! `.github/scripts/perf-check.sh` parses the output of these benches and
//! enforces the budgets from Constitution Principle X.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use singularmem_core::{NewItem, Store};
use tempfile::TempDir;

fn bench_ingest_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_throughput");
    group.throughput(Throughput::Elements(1));
    group.bench_function("ingest_one", |b| {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();
        b.iter(|| {
            store
                .ingest(NewItem::text("benchmark item content"))
                .expect("ingest");
        });
    });
    group.finish();
}

fn bench_get_p95(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();

    // Seed 10 000 items.
    let mut ids = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        let item = store
            .ingest(NewItem::text(format!("seed-{i}")))
            .expect("seed ingest");
        ids.push(item.id);
    }

    let mut group = c.benchmark_group("get_p95");
    group.bench_function("point_read", |b| {
        let mut idx = 0_usize;
        b.iter(|| {
            // Round-robin through the seeded IDs to avoid SQLite's row cache
            // skewing the measurement to a hot row.
            let id = ids[idx % ids.len()];
            idx += 1;
            let _ = store.get(id).expect("get");
        });
    });
    group.finish();
}

criterion_group!(benches, bench_ingest_throughput, bench_get_p95);
criterion_main!(benches);
