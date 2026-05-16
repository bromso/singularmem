//! Revision-walk tests: chains, `latest_revision`, `AmbiguousLatest` forks.

use singularmem_core::{Error, NewItem, Store};
use tempfile::TempDir;

fn fresh_store() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("store.db")).unwrap();
    (dir, store)
}

#[test]
fn revision_history_single_item_returns_self() {
    let (_dir, store) = fresh_store();
    let item = store.ingest(NewItem::text("only")).unwrap();
    let history = store.revision_history(item.id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, item.id);
}

#[test]
fn revision_history_walks_chain_newest_first() {
    let (_dir, store) = fresh_store();
    let v1 = store.ingest(NewItem::text("v1")).unwrap();
    let mut new_v2 = NewItem::text("v2");
    new_v2.supersedes = Some(v1.id);
    let v2 = store.ingest(new_v2).unwrap();
    let mut new_v3 = NewItem::text("v3");
    new_v3.supersedes = Some(v2.id);
    let v3 = store.ingest(new_v3).unwrap();

    let history = store.revision_history(v3.id).expect("history");
    let ids: Vec<_> = history.iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![v3.id, v2.id, v1.id]);
}

#[test]
fn revision_history_unknown_id_errors() {
    use std::str::FromStr;
    let (_dir, store) = fresh_store();
    let bogus = singularmem_core::ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let err = store.revision_history(bogus).unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }));
}

#[test]
fn latest_revision_finds_newest_in_linear_chain() {
    let (_dir, store) = fresh_store();
    let v1 = store.ingest(NewItem::text("v1")).unwrap();
    let mut new_v2 = NewItem::text("v2");
    new_v2.supersedes = Some(v1.id);
    let v2 = store.ingest(new_v2).unwrap();

    let latest = store.latest_revision(v1.id).expect("latest");
    assert_eq!(latest.id, v2.id);
}

#[test]
fn latest_revision_starting_from_head_returns_self() {
    let (_dir, store) = fresh_store();
    let v1 = store.ingest(NewItem::text("v1")).unwrap();
    let latest = store.latest_revision(v1.id).expect("latest");
    assert_eq!(latest.id, v1.id);
}

#[test]
fn latest_revision_ambiguous_fork_errors() {
    let (_dir, store) = fresh_store();
    let original = store.ingest(NewItem::text("original")).unwrap();
    let mut fork_a = NewItem::text("fork-a");
    fork_a.supersedes = Some(original.id);
    let fa = store.ingest(fork_a).unwrap();
    let mut fork_b = NewItem::text("fork-b");
    fork_b.supersedes = Some(original.id);
    let fb = store.ingest(fork_b).unwrap();

    let err = store.latest_revision(original.id).unwrap_err();
    match err {
        Error::AmbiguousLatest { candidates } => {
            let mut sorted = candidates;
            sorted.sort_by_key(ToString::to_string);
            let mut expected = vec![fa.id, fb.id];
            expected.sort_by_key(ToString::to_string);
            assert_eq!(sorted, expected);
        }
        other => panic!("expected AmbiguousLatest, got {other:?}"),
    }
}
