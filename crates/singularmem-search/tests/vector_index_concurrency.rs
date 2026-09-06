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

    fill(&a, &e, "a");
    a.commit(false).unwrap();
    fill(&b, &e, "b");
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

/// The case the journal-replay rule alone does *not* cover, and the one that
/// actually lost data: `a` compacts (truncating the journal), then long-lived
/// `b` compacts from an in-memory graph that predates `a`'s work. Replay finds
/// nothing left in the journal, so `b` would save its own 40 vectors over
/// `a`'s 80. A compaction must notice the on-disk keymap's generation moved
/// and reload `index.usearch` + `keymap.bin` before saving.
#[test]
fn a_compacts_then_b_compacts_and_a_fresh_open_sees_everything() {
    let dir = tempfile::tempdir().unwrap();
    let vdir = dir.path().join("v");
    let e = MockEmbedder::default();
    let a = VectorIndex::open(&vdir, &e).unwrap();
    let b = VectorIndex::open(&vdir, &e).unwrap();

    fill(&a, &e, "a");
    a.commit(false).unwrap();
    fill(&b, &e, "b");
    b.commit(false).unwrap();

    a.compact().unwrap();
    b.compact().unwrap();
    assert_eq!(
        b.len(),
        80,
        "b must adopt a's compacted state before saving"
    );
    drop((a, b));

    let idx = VectorIndex::open(&vdir, &e).unwrap();
    assert_eq!(
        idx.len(),
        80,
        "a stale handle's compaction must not clobber"
    );
}

/// Same hazard on the bulk path, where both handles compact on every commit
/// and the journal is never written at all. This is the shape the reviewer
/// reproduced: 40 of 80 vectors silently lost, no error.
#[test]
fn end_of_batch_commits_from_two_handles_keep_both_sets() {
    let dir = tempfile::tempdir().unwrap();
    let vdir = dir.path().join("v");
    let e = MockEmbedder::default();
    let a = VectorIndex::open(&vdir, &e).unwrap();
    let b = VectorIndex::open(&vdir, &e).unwrap();

    fill(&a, &e, "a");
    a.commit(true).unwrap();
    fill(&b, &e, "b");
    b.commit(true).unwrap();
    drop((a, b));

    let idx = VectorIndex::open(&vdir, &e).unwrap();
    assert_eq!(
        idx.len(),
        80,
        "an end-of-batch commit from a stale handle must not clobber the other's vectors"
    );
}

fn fill(idx: &VectorIndex, e: &MockEmbedder, tag: &str) {
    for i in 0..40 {
        let v = Embedder::embed(e, &format!("{tag} {i}")).unwrap();
        idx.add(fresh_id(), &v).unwrap();
    }
}

fn fresh_id() -> singularmem_core::ItemId {
    ulid::Ulid::new().to_string().parse().unwrap()
}
