//! Tests for the `IndexHook` integration with `Store`. The Principle VII
//! asymmetric-failure tests live in this file too (added in Task 8); this
//! initial version covers just the "trait + `Store::set_hook` + `Store::open_with_hook`
//! compile and run" surface.

use singularmem_core::{Error, IndexHook, Item, NewItem, Result, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

/// Counting hook: records the number of `on_ingest` / `on_reindex` / commit calls.
struct CountingHook {
    on_ingest_calls: Arc<AtomicUsize>,
    commit_calls: Arc<AtomicUsize>,
}

impl IndexHook for CountingHook {
    fn on_ingest(&self, _item: &Item) -> Result<()> {
        self.on_ingest_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn on_reindex(&self, _item: &Item) -> Result<()> {
        Ok(())
    }
    fn commit(&self) -> Result<()> {
        self.commit_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn open_with_hook_calls_on_ingest_per_item() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let on_ingest = Arc::new(AtomicUsize::new(0));
    let commit = Arc::new(AtomicUsize::new(0));
    let hook = Box::new(CountingHook {
        on_ingest_calls: Arc::clone(&on_ingest),
        commit_calls: Arc::clone(&commit),
    });
    let store = Store::open_with_hook(&path, hook).expect("open with hook");

    let _ = store.ingest(NewItem::text("one")).unwrap();
    let _ = store.ingest(NewItem::text("two")).unwrap();

    assert_eq!(on_ingest.load(Ordering::SeqCst), 2);
    // Single-item ingest calls commit after each on_ingest.
    assert_eq!(commit.load(Ordering::SeqCst), 2);
}

#[test]
fn ingest_many_calls_commit_once() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let on_ingest = Arc::new(AtomicUsize::new(0));
    let commit = Arc::new(AtomicUsize::new(0));
    let hook = Box::new(CountingHook {
        on_ingest_calls: Arc::clone(&on_ingest),
        commit_calls: Arc::clone(&commit),
    });
    let store = Store::open_with_hook(&path, hook).expect("open with hook");

    let items: Vec<NewItem> = (0..10)
        .map(|i| NewItem::text(format!("item-{i}")))
        .collect();
    let _ = store.ingest_many(items).unwrap();

    assert_eq!(on_ingest.load(Ordering::SeqCst), 10);
    // Bulk ingest: one commit at the end of the batch, NOT one per item.
    assert_eq!(commit.load(Ordering::SeqCst), 1);
}

#[test]
fn store_open_without_hook_works_unchanged() {
    // Verifies the v0.1.0 path is preserved (no IndexHook overhead when not opted in).
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    let item = store.ingest(NewItem::text("no hook")).unwrap();
    let fetched = store.get(item.id).unwrap();
    assert_eq!(fetched.content, "no hook");
}

/// Always-failing hook. Used to assert Principle VII: `SQLite` write succeeds
/// even when the hook errors.
struct FailingHook;

impl IndexHook for FailingHook {
    fn on_ingest(&self, _item: &Item) -> Result<()> {
        Err(Error::Io(std::io::Error::other("simulated hook failure")))
    }
    fn on_reindex(&self, _item: &Item) -> Result<()> {
        Err(Error::Io(std::io::Error::other("simulated hook failure")))
    }
    fn commit(&self) -> Result<()> {
        Err(Error::Io(std::io::Error::other("simulated commit failure")))
    }
}

#[test]
fn failing_hook_does_not_fail_ingest() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let store = Store::open_with_hook(&path, Box::new(FailingHook)).unwrap();

    // Ingest succeeds despite the hook failing.
    let item = store
        .ingest(NewItem::text("durable despite hook failure"))
        .expect("ingest must succeed when hook fails (Principle VII)");
    assert_eq!(item.content, "durable despite hook failure");

    // Item is still in the SQLite store — verify with a fresh Store::get.
    let fetched = store.get(item.id).expect("item should still be in SQLite");
    assert_eq!(fetched, item);
}

#[test]
fn failing_hook_does_not_fail_ingest_many() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let store = Store::open_with_hook(&path, Box::new(FailingHook)).unwrap();

    let items: Vec<NewItem> = (0..5).map(|i| NewItem::text(format!("bulk-{i}"))).collect();

    let stored = store
        .ingest_many(items)
        .expect("ingest_many must succeed when hook fails (Principle VII)");

    assert_eq!(stored.len(), 5);

    // All five items should still be in the SQLite store.
    for item in &stored {
        let fetched = store
            .get(item.id)
            .expect("each item should still be in SQLite");
        assert_eq!(fetched.id, item.id);
    }
}

#[test]
fn failing_hook_after_store_drop_does_not_panic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");

    {
        let store = Store::open_with_hook(&path, Box::new(FailingHook)).unwrap();
        let _ = store.ingest(NewItem::text("hello")).unwrap();
    } // Store drops; hook drops; no panic.

    // Reopen without hook; the item is still there.
    let store2 = Store::open(&path).unwrap();
    let count = store2.list().unwrap().count();
    assert_eq!(count, 1);
}
