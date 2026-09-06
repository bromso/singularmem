//! Concurrency test for the vectors-v2 commit lock: two independent
//! `EmbedderIndex` handles writing the same directory must not lose each
//! other's vectors, whether they only append or one of them compacts.

use std::thread;

use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{Embedder, EmbedderIndex, VectorIndex};

#[test]
fn concurrent_single_ingests_from_two_handles_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join("s.db");
    let vdir = dir.path().join("v");
    let mut handles = Vec::new();
    for t in 0..2 {
        let (sp, vd) = (store_path.clone(), vdir.clone());
        handles.push(thread::spawn(move || {
            let idx = EmbedderIndex::open(&vd, Box::new(MockEmbedder::default())).unwrap();
            let store = Store::open_with_hook(&sp, Box::new(idx)).unwrap();
            for i in 0..40 {
                store
                    .ingest(NewItem::text(format!("t{t} item {i}")))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let idx = VectorIndex::open(&vdir, &MockEmbedder::default()).unwrap();
    assert_eq!(
        idx.len(),
        80,
        "every vector from both writers is present after replay"
    );
}

/// Deterministic form of the same hazard: two long-lived handles each hold
/// their own in-memory `USearch`, and one of them compacts. A handle that
/// compacted straight from its stale view would drop the other writer's
/// journal-only vectors, so `compact` must first replay the journal (skipping
/// ids already in its keymap) under the lock.
#[test]
fn compaction_from_a_stale_handle_keeps_the_other_handles_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let vdir = dir.path().join("v");
    let e = MockEmbedder::default();
    let a = VectorIndex::open(&vdir, &e).unwrap();
    let b = VectorIndex::open(&vdir, &e).unwrap();

    for i in 0..40 {
        let v = Embedder::embed(&e, &format!("a {i}")).unwrap();
        a.add(fresh_id(), &v).unwrap();
    }
    a.commit(false).unwrap();
    for i in 0..40 {
        let v = Embedder::embed(&e, &format!("b {i}")).unwrap();
        b.add(fresh_id(), &v).unwrap();
    }
    b.commit(false).unwrap();
    assert_eq!(a.journal_len().unwrap(), 80, "both writers appended");

    // `a` has only its own 40 vectors in memory; compacting must not lose b's.
    a.compact().unwrap();
    assert_eq!(a.journal_len().unwrap(), 0);
    drop((a, b));

    let idx = VectorIndex::open(&vdir, &e).unwrap();
    assert_eq!(
        idx.len(),
        80,
        "compaction must replay the journal under the lock before saving"
    );
}

fn fresh_id() -> singularmem_core::ItemId {
    ulid::Ulid::new().to_string().parse().unwrap()
}
