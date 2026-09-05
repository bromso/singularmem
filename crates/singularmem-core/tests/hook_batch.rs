use std::sync::{Arc, Mutex};

use singularmem_core::hook::MultiHook;
use singularmem_core::{IndexHook, Item, NewItem, Store};

/// Records every call so tests can assert the exact sequence.
#[derive(Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<String>>>,
    fail_on: Option<String>,
}

impl IndexHook for Recorder {
    fn on_ingest(&self, item: &Item) -> singularmem_core::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("ingest:{}", item.content));
        if self.fail_on.as_deref() == Some(item.content.as_str()) {
            return Err(singularmem_core::Error::Validation {
                field: "hook",
                reason: "boom".into(),
            });
        }
        Ok(())
    }
    fn on_reindex(&self, item: &Item) -> singularmem_core::Result<()> {
        self.on_ingest(item)
    }
    fn commit(&self) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push("commit".into());
        Ok(())
    }
}

/// Overrides the batch method so the test can see it was used.
struct BatchAware {
    calls: Arc<Mutex<Vec<String>>>,
}

impl IndexHook for BatchAware {
    fn on_ingest(&self, item: &Item) -> singularmem_core::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("single:{}", item.content));
        Ok(())
    }
    fn on_reindex(&self, item: &Item) -> singularmem_core::Result<()> {
        self.on_ingest(item)
    }
    fn on_ingest_batch(&self, items: &[Item]) -> singularmem_core::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("batch:{}", items.len()));
        Ok(())
    }
    fn commit(&self) -> singularmem_core::Result<()> {
        self.calls.lock().unwrap().push("commit".into());
        Ok(())
    }
}

fn items(n: usize) -> Vec<NewItem> {
    (0..n).map(|i| NewItem::text(format!("item {i}"))).collect()
}

#[test]
fn default_batch_is_per_item_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let hook = Recorder {
        calls: calls.clone(),
        fail_on: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(hook)).unwrap();
    store.ingest_many(items(3)).unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["ingest:item 0", "ingest:item 1", "ingest:item 2", "commit"]
    );
}

#[test]
fn ingest_many_uses_the_batch_method_once_per_batch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(
        dir.path().join("s.db"),
        Box::new(BatchAware {
            calls: calls.clone(),
        }),
    )
    .unwrap();
    store.ingest_many(items(5)).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["batch:5", "commit"]);
    // Single-item ingest still uses the per-item path.
    store.ingest(NewItem::text("solo")).unwrap();
    assert_eq!(calls.lock().unwrap().last().unwrap(), "commit");
    assert!(calls.lock().unwrap().contains(&"single:solo".to_string()));
}

#[test]
fn multi_hook_forwards_the_batch_to_every_member() {
    let a = Arc::new(Mutex::new(Vec::new()));
    let b = Arc::new(Mutex::new(Vec::new()));
    let multi = MultiHook::new(vec![
        Box::new(BatchAware { calls: a.clone() }),
        Box::new(Recorder {
            calls: b.clone(),
            fail_on: None,
        }),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(multi)).unwrap();
    store.ingest_many(items(2)).unwrap();
    assert_eq!(*a.lock().unwrap(), vec!["batch:2", "commit"]);
    assert_eq!(
        *b.lock().unwrap(),
        vec!["ingest:item 0", "ingest:item 1", "commit"]
    );
}

#[test]
fn a_failing_batch_does_not_fail_ingest_many() {
    // Items are durably stored; the hook failure is logged, not returned.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let hook = Recorder {
        calls,
        fail_on: Some("item 1".into()),
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_hook(dir.path().join("s.db"), Box::new(hook)).unwrap();
    let out = store.ingest_many(items(3)).unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(store.list().unwrap().count(), 3);
}
