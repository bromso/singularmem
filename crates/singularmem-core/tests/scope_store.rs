use singularmem_core::{Error, NewItem, ScopeFilter, Store};
use tempfile::TempDir;

fn seeded() -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    for (content, scope, tag) in [
        ("root-a", Some("a"), "t"),
        ("child-ab", Some("a/b"), "t"),
        ("grandchild-abc", Some("a/b/c"), "u"),
        ("sibling-ac", Some("a/c"), "t"),
        ("other-x", Some("x"), "t"),
        ("unscoped", None, "t"),
    ] {
        let mut n = NewItem::text(content);
        n.scope = scope.map(str::to_string);
        n.tags = vec![tag.to_string()];
        s.ingest(n).unwrap();
    }
    (d, s)
}

fn contents(iter: singularmem_core::ItemIter<'_>) -> Vec<String> {
    iter.map(|r| r.unwrap().content).collect()
}

#[test]
fn descendants_include_self_children_and_grandchildren_only() {
    let (_d, s) = seeded();
    let f = ScopeFilter::descendants("a/b").unwrap();
    assert_eq!(
        contents(s.list_scoped(Some(&f)).unwrap()),
        vec!["child-ab", "grandchild-abc"]
    );
    let f = ScopeFilter::descendants("a").unwrap();
    assert_eq!(
        contents(s.list_scoped(Some(&f)).unwrap()),
        vec!["root-a", "child-ab", "grandchild-abc", "sibling-ac"]
    );
}

#[test]
fn exact_matches_only_that_scope() {
    let (_d, s) = seeded();
    let f = ScopeFilter::exact("a/b").unwrap();
    assert_eq!(contents(s.list_scoped(Some(&f)).unwrap()), vec!["child-ab"]);
}

#[test]
fn none_filter_lists_everything_including_unscoped() {
    let (_d, s) = seeded();
    assert_eq!(s.list_scoped(None).unwrap().count(), 6);
}

#[test]
fn prefix_that_is_not_a_segment_boundary_does_not_match() {
    let (_d, s) = seeded();
    let mut n = NewItem::text("abz");
    n.scope = Some("a/bz".into());
    s.ingest(n).unwrap();
    let f = ScopeFilter::descendants("a/b").unwrap();
    assert!(!contents(s.list_scoped(Some(&f)).unwrap()).contains(&"abz".to_string()));
}

#[test]
fn underscore_in_scope_is_literal_not_wildcard() {
    let (_d, s) = seeded();
    for (c, sc) in [("under", "p_q/one"), ("slash", "p/q/one")] {
        let mut n = NewItem::text(c);
        n.scope = Some(sc.into());
        s.ingest(n).unwrap();
    }
    let f = ScopeFilter::descendants("p_q").unwrap();
    assert_eq!(contents(s.list_scoped(Some(&f)).unwrap()), vec!["under"]);
}

#[test]
fn tags_and_scope_compose() {
    let (_d, s) = seeded();
    let f = ScopeFilter::descendants("a").unwrap();
    assert_eq!(
        contents(s.list_by_tags_scoped(&["u"], Some(&f)).unwrap()),
        vec!["grandchild-abc"]
    );
    assert_eq!(
        contents(s.list_by_tags_scoped(&["t"], Some(&f)).unwrap()),
        vec!["root-a", "child-ab", "sibling-ac"]
    );
}

#[test]
fn scopes_lists_distinct_paths_with_counts_sorted() {
    let (_d, s) = seeded();
    assert_eq!(
        s.scopes().unwrap(),
        vec![
            ("a".to_string(), 1),
            ("a/b".to_string(), 1),
            ("a/b/c".to_string(), 1),
            ("a/c".to_string(), 1),
            ("x".to_string(), 1)
        ]
    );
}

#[test]
fn scope_of_and_set_scope() {
    let (_d, s) = seeded();
    let item = s.list().unwrap().next().unwrap().unwrap();
    assert_eq!(s.scope_of(item.id).unwrap().as_deref(), Some("a"));
    let moved = s.set_scope(item.id, Some("Moved/Here")).unwrap();
    assert_eq!(moved.scope.as_deref(), Some("moved/here"));
    assert_eq!(s.scope_of(item.id).unwrap().as_deref(), Some("moved/here"));
    let cleared = s.set_scope(item.id, None).unwrap();
    assert_eq!(cleared.scope, None);
    assert!(matches!(
        s.set_scope(item.id, Some("bad//x")),
        Err(Error::Validation { field: "scope", .. })
    ));
    let bogus = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    assert!(matches!(
        s.set_scope(bogus, Some("a")),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn set_scope_refused_read_only() {
    let d = TempDir::new().unwrap();
    let p = d.path().join("s.db");
    let id = {
        let s = Store::open(&p).unwrap();
        s.ingest(NewItem::text("x")).unwrap().id
    };
    let ro =
        Store::open_with_options(&p, singularmem_core::StoreOptions { read_only: true }).unwrap();
    assert!(matches!(
        ro.set_scope(id, Some("a")),
        Err(Error::ReadOnly { .. })
    ));
}
