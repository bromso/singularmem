use std::fs;

use serde_json::json;
use singularmem_core::{NewItem, Store};
use singularmem_ingest::{ingest_source, DirectoryWalker, Report, Source};
use tempfile::TempDir;

struct Fixed(Vec<NewItem>);
impl Source for Fixed {
    fn name(&self) -> String {
        "fixed".into()
    }
    fn items(&self) -> Box<dyn Iterator<Item = singularmem_ingest::Result<NewItem>> + '_> {
        Box::new(self.0.iter().cloned().map(Ok))
    }
    fn filtered_count(&self) -> usize {
        3
    }
}

fn keyed(c: &str, k: &str) -> NewItem {
    let mut n = NewItem::text(c);
    n.external_id = Some(k.into());
    n
}

#[test]
fn second_run_ingests_nothing() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let src = Fixed(vec![keyed("a", "k:1"), keyed("b", "k:2")]);
    let r1 = ingest_source(&s, &src, false).unwrap();
    assert_eq!(
        r1,
        Report {
            ingested: 2,
            skipped_existing: 0,
            skipped_filtered: 3,
            failed: 0
        }
    );
    let r2 = ingest_source(&s, &src, false).unwrap();
    assert_eq!(
        r2,
        Report {
            ingested: 0,
            skipped_existing: 2,
            skipped_filtered: 3,
            failed: 0
        }
    );
    assert_eq!(s.list().unwrap().count(), 2);
}

#[test]
fn dry_run_writes_nothing_but_reports() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let src = Fixed(vec![keyed("a", "k:1")]);
    let r = ingest_source(&s, &src, true).unwrap();
    assert_eq!(r.ingested, 1);
    assert_eq!(s.list().unwrap().count(), 0);
}

#[test]
fn per_item_errors_are_counted_not_fatal() {
    struct Flaky;
    impl Source for Flaky {
        fn name(&self) -> String {
            "flaky".into()
        }
        fn items(&self) -> Box<dyn Iterator<Item = singularmem_ingest::Result<NewItem>> + '_> {
            Box::new(
                vec![
                    Ok(keyed("a", "k:1")),
                    Err(singularmem_ingest::Error::NotFound { path: "/x".into() }),
                    Ok(keyed("b", "k:2")),
                ]
                .into_iter(),
            )
        }
    }
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let r = ingest_source(&s, &Flaky, false).unwrap();
    assert_eq!(r.ingested, 2);
    assert_eq!(r.failed, 1);
}

#[test]
fn changed_file_supersedes_previous_item() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".gitignore"), "s.db*\n").unwrap();
    fs::write(d.path().join("a.txt"), "version one").unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let w = DirectoryWalker::new(d.path()).unwrap();
    let r1 = ingest_source(&s, &w, false).unwrap();
    assert_eq!(r1.ingested, 1);
    let old = s.list().unwrap().next().unwrap().unwrap();

    // Unchanged → skipped.
    let r2 = ingest_source(&s, &w, false).unwrap();
    assert_eq!((r2.ingested, r2.skipped_existing), (0, 1));

    fs::write(d.path().join("a.txt"), "version two").unwrap();
    let r3 = ingest_source(&s, &w, false).unwrap();
    assert_eq!(r3.ingested, 1);
    let key = old.external_id.clone().unwrap();
    let newest = s.get_by_external_id(&key).unwrap().unwrap();
    assert_eq!(newest.content, "version two");
    assert_eq!(newest.supersedes, Some(old.id));
    assert_eq!(s.get(old.id).unwrap().external_id, None);
}

#[test]
fn large_batches_are_split() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let items: Vec<NewItem> = (0..1203)
        .map(|i| keyed(&format!("c{i}"), &format!("k:{i}")))
        .collect();
    let r = ingest_source(&s, &Fixed(items), false).unwrap();
    assert_eq!(r.ingested, 1203);
    assert_eq!(s.list().unwrap().count(), 1203);
}

#[test]
fn duplicate_external_id_within_one_run_is_skipped_deterministically() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let src = Fixed(vec![
        keyed("a", "k:1"),
        keyed("b", "k:2"),
        keyed("c", "k:1"),
    ]);
    let r = ingest_source(&s, &src, false).unwrap();
    assert_eq!(r.ingested, 2);
    assert_eq!(r.skipped_existing, 1);
    assert_eq!(s.list().unwrap().count(), 2);
    let kept = s.get_by_external_id("k:1").unwrap().unwrap();
    assert_eq!(kept.content, "a");
}

#[test]
fn duplicate_external_id_across_batch_boundary_is_skipped_deterministically() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let items: Vec<NewItem> = (0..501)
        .map(|i| {
            let key = if i == 500 {
                "k:0".to_string()
            } else {
                format!("k:{i}")
            };
            keyed(&format!("c{i}"), &key)
        })
        .collect();
    let r = ingest_source(&s, &Fixed(items), false).unwrap();
    assert_eq!(r.ingested, 500);
    assert_eq!(r.skipped_existing, 1);
    assert_eq!(s.list().unwrap().count(), 500);
}

#[test]
fn existing_item_without_hash_is_not_replaced() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    s.ingest(keyed("v1", "k")).unwrap();

    let mut new_version = keyed("v2", "k");
    new_version.metadata = json!({"sha256": "abc"});
    let src = Fixed(vec![new_version]);
    let r = ingest_source(&s, &src, false).unwrap();
    assert_eq!(r.ingested, 0);
    assert_eq!(r.skipped_existing, 1);

    assert_eq!(s.list().unwrap().count(), 1);
    let kept = s.get_by_external_id("k").unwrap().unwrap();
    assert_eq!(kept.content, "v1");
}

#[test]
fn in_run_duplicate_of_existing_key_is_replaced_at_most_once() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let mut v0 = keyed("v0", "k");
    v0.metadata = json!({"sha256": "h0"});
    s.ingest(v0).unwrap();

    let mut item_a = keyed("vA", "k");
    item_a.metadata = json!({"sha256": "hA"});
    let mut item_b = keyed("vB", "k");
    item_b.metadata = json!({"sha256": "hB"});
    let src = Fixed(vec![item_a, item_b]);
    let r = ingest_source(&s, &src, false).unwrap();
    assert_eq!(r.ingested, 1);
    assert_eq!(r.skipped_existing, 1);

    let newest = s.get_by_external_id("k").unwrap().unwrap();
    assert_eq!(newest.content, "vA");
    assert_eq!(s.list().unwrap().count(), 2);
    assert_eq!(s.revision_history(newest.id).unwrap().len(), 2);
}

#[test]
fn item_rejected_by_validation_is_counted_not_fatal() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let src = Fixed(vec![
        keyed("a", "k:1"),
        keyed("bad", &"x".repeat(600)),
        keyed("b", "k:2"),
    ]);
    let r = ingest_source(&s, &src, false).unwrap();
    assert_eq!(r.ingested, 2);
    assert_eq!(r.failed, 1);
    assert_eq!(s.list().unwrap().count(), 2);
}
