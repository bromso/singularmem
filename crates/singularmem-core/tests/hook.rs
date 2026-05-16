//! Tests for the `IndexHook` integration with `Store`. The Principle VII
//! asymmetric-failure tests live in this file too (added in Task 8); this
//! initial version covers just the "trait + `Store::set_hook` + `Store::open_with_hook`
//! compile and run" surface.

use singularmem_core::{IndexHook, Item, NewItem, Result, Store};
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

    let items: Vec<NewItem> = (0..10).map(|i| NewItem::text(format!("item-{i}"))).collect();
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
