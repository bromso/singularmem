//! Tests for the `MultiHook` composite + `Store::open_with_hooks` constructor.
//! The Principle VII isolation test (failing hook doesn't block others)
//! lands here in Task 17; this file initially covers the construction +
//! per-hook fan-out.

use singularmem_core::{IndexHook, Item, NewItem, Result, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

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
fn multi_hook_fans_out_on_ingest_to_all_members() {
    let a_ingest = Arc::new(AtomicUsize::new(0));
    let a_commit = Arc::new(AtomicUsize::new(0));
    let b_ingest = Arc::new(AtomicUsize::new(0));
    let b_commit = Arc::new(AtomicUsize::new(0));

    let dir = TempDir::new().unwrap();
    let store = Store::open_with_hooks(
        dir.path().join("store.db"),
        vec![
            Box::new(CountingHook {
                on_ingest_calls: Arc::clone(&a_ingest),
                commit_calls: Arc::clone(&a_commit),
            }),
            Box::new(CountingHook {
                on_ingest_calls: Arc::clone(&b_ingest),
                commit_calls: Arc::clone(&b_commit),
            }),
        ],
    )
    .expect("open with two hooks");

    let _ = store.ingest(NewItem::text("hello")).unwrap();

    assert_eq!(a_ingest.load(Ordering::SeqCst), 1);
    assert_eq!(b_ingest.load(Ordering::SeqCst), 1);
    assert_eq!(a_commit.load(Ordering::SeqCst), 1);
    assert_eq!(b_commit.load(Ordering::SeqCst), 1);
}

struct FailingHook;

impl IndexHook for FailingHook {
    fn on_ingest(&self, _: &Item) -> Result<()> {
        Err(singularmem_core::Error::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn on_reindex(&self, _: &Item) -> Result<()> {
        Ok(())
    }
    fn commit(&self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn failing_hook_in_multi_hook_does_not_block_others() {
    let working_count = Arc::new(AtomicUsize::new(0));
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");

    let store = Store::open_with_hooks(
        &path,
        vec![
            Box::new(FailingHook),
            Box::new(CountingHook {
                on_ingest_calls: Arc::clone(&working_count),
                commit_calls: Arc::new(AtomicUsize::new(0)),
            }),
        ],
    )
    .unwrap();

    // ingest returns Ok despite the failing hook (Principle VII).
    let item = store.ingest(NewItem::text("durable")).unwrap();
    assert_eq!(
        working_count.load(Ordering::SeqCst),
        1,
        "working hook still fires"
    );

    // Item is durable in SQLite.
    let fetched = store.get(item.id).unwrap();
    assert_eq!(fetched.content, "durable");
}
