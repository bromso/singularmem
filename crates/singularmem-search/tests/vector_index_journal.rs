//! Tests for the vectors-v2 journal wiring in `VectorIndex`: single ingests
//! append to `journal.bin` without rewriting `index.usearch`, compaction fires
//! at the threshold and at batch end, replay is idempotent, and a v1 directory
//! is upgraded to v2 on its first commit.

use singularmem_core::{NewItem, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{Embedder, EmbedderIndex, VectorIndex, COMPACT_THRESHOLD};

fn open(dir: &std::path::Path) -> EmbedderIndex {
    EmbedderIndex::open(dir.join("v"), Box::new(MockEmbedder::default())).unwrap()
}

fn embed(text: &str) -> Vec<f32> {
    Embedder::embed(&MockEmbedder::default(), text).unwrap()
}

#[test]
fn single_ingest_appends_to_journal_and_does_not_rewrite_index() {
    let dir = tempfile::tempdir().unwrap();
    // Seed 200 items in one batch -> compaction at batch end, journal empty.
    {
        let store =
            Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
        store
            .ingest_many((0..200).map(|i| NewItem::text(format!("seed {i}"))))
            .unwrap();
    }
    let usearch = dir.path().join("v/index.usearch");
    let journal = dir.path().join("v/journal.bin");
    let before = std::fs::metadata(&usearch).unwrap().modified().unwrap();
    let jlen_before = std::fs::metadata(&journal).unwrap().len();
    std::thread::sleep(std::time::Duration::from_millis(20));
    {
        let store =
            Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
        store.ingest(NewItem::text("one more")).unwrap();
    }
    assert_eq!(
        std::fs::metadata(&usearch).unwrap().modified().unwrap(),
        before,
        "index.usearch must not be rewritten by a single ingest"
    );
    assert!(
        std::fs::metadata(&journal).unwrap().len() > jlen_before,
        "journal grew"
    );
    // And the item is searchable after reopen (journal replay).
    let idx = open(dir.path());
    let v = embed("one more");
    let hits = idx.vector_index().search(&v, 1).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn compaction_fires_at_threshold_and_at_batch_end() {
    let dir = tempfile::tempdir().unwrap();
    let idx = open(dir.path());
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(idx)).unwrap();
    for i in 0..COMPACT_THRESHOLD {
        store.ingest(NewItem::text(format!("s {i}"))).unwrap();
    }
    let idx_probe = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(
        idx_probe.journal_len().unwrap(),
        COMPACT_THRESHOLD,
        "at the threshold, not yet compacted"
    );
    drop(idx_probe);
    store.ingest(NewItem::text("over")).unwrap();
    let idx_probe = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(
        idx_probe.journal_len().unwrap(),
        0,
        "one past the threshold compacts"
    );
    drop(idx_probe);
    store.ingest(NewItem::text("after")).unwrap();
    store
        .ingest_many(vec![NewItem::text("b1"), NewItem::text("b2")])
        .unwrap();
    let idx_probe = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx_probe.journal_len().unwrap(), 0, "batch end compacts");
}

#[test]
fn replay_skips_ids_already_in_keymap_and_search_is_identical_before_and_after_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
    let items = store
        .ingest_many((0..50).map(|i| NewItem::text(format!("corpus {i}"))))
        .unwrap();
    for i in 0..30 {
        store.ingest(NewItem::text(format!("extra {i}"))).unwrap();
    }
    drop(store);
    let q = embed("corpus 7");
    let before = open(dir.path()).vector_index().search(&q, 5).unwrap();
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    idx.compact().unwrap();
    assert_eq!(idx.journal_len().unwrap(), 0);
    drop(idx);
    let after = open(dir.path()).vector_index().search(&q, 5).unwrap();
    assert_eq!(
        before.iter().map(|h| h.id).collect::<Vec<_>>(),
        after.iter().map(|h| h.id).collect::<Vec<_>>()
    );
    assert!(after.iter().any(|h| h.id == items[7].id));
    // Reopen again: replay of an empty journal, no duplicates.
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(idx.len(), 80);
}

#[test]
fn v1_directory_opens_and_becomes_v2_on_first_commit() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store =
            Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
        store.ingest_many(vec![NewItem::text("a")]).unwrap();
    }
    // Simulate v0.20.0 layout: no journal, format_version "1".
    std::fs::remove_file(dir.path().join("v/journal.bin")).unwrap();
    let meta_path = dir.path().join("v/.meta.json");
    let meta = std::fs::read_to_string(&meta_path)
        .unwrap()
        .replace("\"format_version\": \"2\"", "\"format_version\": \"1\"");
    assert!(
        meta.contains("\"format_version\": \"1\""),
        "fixture must actually downgrade the meta"
    );
    std::fs::write(&meta_path, meta).unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
    store.ingest(NewItem::text("b")).unwrap();
    assert!(std::fs::read_to_string(&meta_path)
        .unwrap()
        .contains("\"format_version\": \"2\""));
    assert!(dir.path().join("v/journal.bin").exists());
}

#[test]
fn crash_between_rename_and_truncate_is_idempotent_on_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(open(dir.path()))).unwrap();
    for i in 0..10 {
        store.ingest(NewItem::text(format!("x {i}"))).unwrap();
    }
    drop(store);
    // Compact, then put the journal records back as if truncate never happened.
    let journal_bytes = std::fs::read(dir.path().join("v/journal.bin")).unwrap();
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    idx.compact().unwrap();
    drop(idx);
    std::fs::write(dir.path().join("v/journal.bin"), journal_bytes).unwrap();
    let idx = VectorIndex::open(dir.path().join("v"), &MockEmbedder::default()).unwrap();
    assert_eq!(
        idx.len(),
        10,
        "replayed ids already in the keymap are skipped"
    );
}

/// A vector removed after it was journalled must stay removed: compaction
/// replays the journal, so without a tombstone the removed record is added
/// straight back in.
#[test]
fn remove_after_journalling_is_not_resurrected_by_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let vdir = dir.path().join("v");
    let e = MockEmbedder::default();
    let idx = VectorIndex::open(&vdir, &e).unwrap();

    let keep: singularmem_core::ItemId = ulid::Ulid::new().to_string().parse().unwrap();
    let drop_me: singularmem_core::ItemId = ulid::Ulid::new().to_string().parse().unwrap();
    idx.add(keep, &Embedder::embed(&e, "keep").unwrap())
        .unwrap();
    idx.add(drop_me, &Embedder::embed(&e, "drop").unwrap())
        .unwrap();
    idx.commit(false).unwrap();
    assert_eq!(idx.journal_len().unwrap(), 2);

    idx.remove(drop_me).unwrap();
    idx.compact().unwrap();
    assert_eq!(idx.len(), 1);
    assert!(!idx.contains(drop_me));
    drop(idx);

    let reopened = VectorIndex::open(&vdir, &e).unwrap();
    assert_eq!(reopened.len(), 1, "the removal survives a reopen");
    assert!(reopened.contains(keep));
    assert!(!reopened.contains(drop_me));
}
