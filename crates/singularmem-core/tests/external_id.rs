//! `external_id` uniqueness conflicts: single-item and bulk ingest.

use std::collections::HashSet;

use singularmem_core::{Error, NewItem, Store};
use tempfile::TempDir;

fn store() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let s = Store::open(dir.path().join("s.db")).unwrap();
    (dir, s)
}

fn keyed(content: &str, ext: &str) -> NewItem {
    let mut n = NewItem::text(content);
    n.external_id = Some(ext.to_string());
    n
}

#[test]
fn duplicate_external_id_is_conflict_and_nothing_persists() {
    let (_d, s) = store();
    s.ingest(keyed("a", "x:1")).unwrap();
    let err = s.ingest(keyed("b", "x:1")).unwrap_err();
    assert!(matches!(err, Error::ExternalIdConflict { ref external_id } if external_id == "x:1"));
    assert_eq!(s.list().unwrap().count(), 1);
}

#[test]
fn bulk_conflict_rolls_back_whole_batch() {
    let (_d, s) = store();
    s.ingest(keyed("a", "x:1")).unwrap();
    let err = s
        .ingest_many(vec![keyed("b", "x:2"), keyed("c", "x:1")])
        .unwrap_err();
    assert!(matches!(err, Error::ExternalIdConflict { .. }));
    assert_eq!(s.list().unwrap().count(), 1, "x:2 must not persist");
}

#[test]
fn get_by_external_id_round_trips() {
    let (_d, s) = store();
    let a = s.ingest(keyed("a", "x:1")).unwrap();
    assert_eq!(s.get_by_external_id("x:1").unwrap().unwrap().id, a.id);
    assert!(s.get_by_external_id("nope").unwrap().is_none());
}

#[test]
fn existing_external_ids_returns_only_present() {
    let (_d, s) = store();
    s.ingest(keyed("a", "x:1")).unwrap();
    s.ingest(keyed("b", "x:2")).unwrap();
    let got = s.existing_external_ids(&["x:1", "x:3", "x:2"]).unwrap();
    let want: HashSet<String> = ["x:1", "x:2"].iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);
    assert!(s.existing_external_ids(&[]).unwrap().is_empty());
}

#[test]
fn ingest_replacing_moves_external_id_and_supersedes() {
    let (_d, s) = store();
    let old = s.ingest(keyed("v1", "file:/a.rs")).unwrap();
    let new = s
        .ingest_replacing(keyed("v2", "file:/a.rs"), old.id)
        .unwrap();
    assert_eq!(new.supersedes, Some(old.id));
    assert_eq!(new.external_id.as_deref(), Some("file:/a.rs"));
    assert_eq!(
        s.get(old.id).unwrap().external_id,
        None,
        "old item's id is freed"
    );
    assert_eq!(
        s.get_by_external_id("file:/a.rs").unwrap().unwrap().id,
        new.id
    );
    let hist = s.revision_history(new.id).unwrap();
    assert_eq!(
        hist.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![new.id, old.id]
    );
}

#[test]
fn ingest_replacing_unknown_target_is_supersedes_not_found() {
    let (_d, s) = store();
    let bogus = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let err = s
        .ingest_replacing(keyed("v2", "file:/a.rs"), bogus)
        .unwrap_err();
    assert!(matches!(err, Error::SupersedesNotFound { .. }));
    assert_eq!(s.list().unwrap().count(), 0);
}

#[test]
fn ingest_replacing_refused_read_only() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("s.db");
    let old_id = {
        let s = Store::open(&p).unwrap();
        s.ingest(keyed("v1", "k")).unwrap().id
    };
    let ro =
        Store::open_with_options(&p, singularmem_core::StoreOptions { read_only: true }).unwrap();
    assert!(matches!(
        ro.ingest_replacing(keyed("v2", "k"), old_id),
        Err(Error::ReadOnly { .. })
    ));
}
