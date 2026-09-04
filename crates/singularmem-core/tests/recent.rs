use singularmem_core::{NewItem, ScopeFilter, Store};
use tempfile::TempDir;

fn seeded() -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    for (c, sc) in [("one", "a"), ("two", "a/b"), ("three", "x"), ("four", "a")] {
        let mut n = NewItem::text(c);
        n.scope = Some(sc.into());
        s.ingest(n).unwrap();
    }
    s.ingest(NewItem::text("unscoped")).unwrap();
    (d, s)
}

#[test]
fn recent_is_newest_first_and_limited() {
    let (_d, s) = seeded();
    let v: Vec<String> = s
        .recent(None, 3)
        .unwrap()
        .into_iter()
        .map(|i| i.content)
        .collect();
    assert_eq!(v, vec!["unscoped", "four", "three"]);
}

#[test]
fn recent_respects_scope_filter() {
    let (_d, s) = seeded();
    let f = ScopeFilter::descendants("a").unwrap();
    let v: Vec<String> = s
        .recent(Some(&f), 10)
        .unwrap()
        .into_iter()
        .map(|i| i.content)
        .collect();
    assert_eq!(v, vec!["four", "two", "one"]);
    assert_eq!(s.count_scoped(Some(&f)).unwrap(), 3);
    assert_eq!(s.count_scoped(None).unwrap(), 5);
}

#[test]
fn recent_zero_limit_is_empty() {
    let (_d, s) = seeded();
    assert!(s.recent(None, 0).unwrap().is_empty());
}
