//! `external_id` uniqueness conflicts: single-item and bulk ingest.

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
