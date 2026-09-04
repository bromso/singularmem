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

#[test]
fn count_scoped_any_unions_without_double_counting_overlap() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    for (c, sc) in [("one", "a"), ("two", "a/b")] {
        let mut n = NewItem::text(c);
        n.scope = Some(sc.into());
        s.ingest(n).unwrap();
    }
    let a = ScopeFilter::descendants("a").unwrap();
    let a_b = ScopeFilter::descendants("a/b").unwrap();
    // `a` (descendants) already matches both items ("one" scoped to `a`
    // itself, "two" scoped to `a/b` beneath it); `a/b` additionally matches
    // "two". Summing `count_scoped` per filter would double count "two" as
    // 2 + 1 = 3; the union must count it once: 2.
    assert_eq!(s.count_scoped_any(&[a.clone(), a_b.clone()]).unwrap(), 2);
    assert_eq!(s.count_scoped(Some(&a)).unwrap(), 2);
    assert_eq!(s.count_scoped(Some(&a_b)).unwrap(), 1);
    assert_eq!(s.count_scoped_any(&[]).unwrap(), 0);
}
