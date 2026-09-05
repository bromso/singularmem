//! Graph operations on `Store`: add / invalidate / supersede and the read
//! surface (entity, predicate, timeline, stats, entities, history).
//! Spec: `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
//! § "Revisions and the two time axes" and § "Operations".

use jiff::Timestamp;
use singularmem_core::graph::time::parse_point;
use singularmem_core::graph::{Direction, GraphQuery, NewFact, NewObject};
use singularmem_core::{Clock, Error, NewItem, OsRng, ScopeFilter, Store};
use tempfile::TempDir;

struct FixedClock(Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

fn ts(s: &str) -> Timestamp {
    parse_point(s).unwrap()
}

fn store() -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    (d, s)
}

fn store_at(d: &TempDir, at: &str) -> Store {
    Store::open_with(
        d.path().join("s.db"),
        Box::new(FixedClock(ts(at))),
        Box::new(OsRng),
    )
    .unwrap()
}

fn entity(name: &str) -> NewObject {
    NewObject::Entity {
        name: name.into(),
        kind: None,
    }
}

#[test]
fn add_creates_entities_once_and_is_idempotent() {
    let (_d, s) = store();
    let a = s
        .add_fact(NewFact::triple("Singularmem", "uses", "Tantivy"))
        .unwrap();
    let b = s
        .add_fact(NewFact::triple("singularmem", "Uses", "tantivy"))
        .unwrap();
    assert_eq!(a.id, b.id, "identical open fact is a no-op");
    assert_eq!(
        a.subject.name, "Singularmem",
        "display name is the first spelling"
    );
    assert_eq!(s.entities(None, None).unwrap().len(), 2);
    assert_eq!(s.graph_stats(None).unwrap().open_facts, 1);
}

#[test]
fn value_objects_and_kinds() {
    let (_d, s) = store();
    let mut f = NewFact::triple("singularmem", "written_in", "rust");
    f.object = NewObject::Value("Rust 1.80".into());
    f.subject_kind = Some("project".into());
    let stored = s.add_fact(f).unwrap();
    assert!(
        matches!(stored.object, singularmem_core::graph::FactObject::Value(ref v) if v == "Rust 1.80")
    );
    assert_eq!(
        s.get_entity("singularmem", None)
            .unwrap()
            .unwrap()
            .kind
            .as_deref(),
        Some("project")
    );
    let mut g = NewFact::triple("singularmem", "has_author", "jonas");
    g.subject_kind = Some("person".into());
    assert!(
        matches!(s.add_fact(g), Err(Error::Validation { field: "kind", .. })),
        "kind is immutable"
    );
    let mut h = NewFact::triple("singularmem", "has_author", "jonas");
    h.subject_kind = None;
    s.add_fact(h).unwrap();
    let projects = s.entities(None, Some("project")).unwrap();
    assert_eq!(
        projects
            .iter()
            .map(|e| e.entity.name.as_str())
            .collect::<Vec<_>>(),
        vec!["singularmem"],
        "kind filter"
    );
}

#[test]
fn invalidate_appends_a_revision_and_never_mutates() {
    let d = TempDir::new().unwrap();
    let s = store_at(&d, "2026-06-01");
    let f = s
        .add_fact(NewFact::triple("singularmem", "uses", "tantivy"))
        .unwrap();
    let closed = s
        .invalidate_fact(
            "singularmem",
            "uses",
            &entity("tantivy"),
            None,
            Some(ts("2026-09-01")),
        )
        .unwrap();
    assert_ne!(closed.id, f.id);
    assert_eq!(closed.supersedes, Some(f.id));
    assert_eq!(closed.valid_to, Some(ts("2026-09-01")));
    let original = s.get_fact(f.id).unwrap();
    assert_eq!(original.valid_to, None, "the original row is untouched");
    let hist = s.fact_history(closed.id).unwrap();
    assert_eq!(
        hist.iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![f.id, closed.id]
    );
    assert!(matches!(
        s.invalidate_fact("singularmem", "uses", &entity("tantivy"), None, None),
        Err(Error::FactNotFound { .. })
    ));
}

#[test]
fn as_of_and_recorded_at_answer_both_axes() {
    let d = TempDir::new().unwrap();
    let s1 = store_at(&d, "2026-06-01T00:00:00Z");
    s1.add_fact(NewFact::triple("singularmem", "uses", "tantivy"))
        .unwrap();
    drop(s1);
    let s2 = store_at(&d, "2026-09-01T00:00:00Z");
    let (old, new) = s2
        .supersede_fact(
            "singularmem",
            "uses",
            &entity("tantivy"),
            entity("meilisearch"),
            None,
            Some(ts("2026-09-01")),
        )
        .unwrap();
    assert!(old.is_some());
    assert_eq!(new.valid_from, Some(ts("2026-09-01")));
    let names = |facts: Vec<singularmem_core::graph::Fact>| {
        facts
            .into_iter()
            .map(|f| match f.object {
                singularmem_core::graph::FactObject::Entity(e) => e.name,
                singularmem_core::graph::FactObject::Value(v) => v,
            })
            .collect::<Vec<_>>()
    };
    let q = |as_of: Option<&str>, rec: Option<&str>| GraphQuery {
        as_of: as_of.map(ts),
        recorded_at: rec.map(ts),
        direction: Direction::Outgoing,
        scope: None,
    };
    assert_eq!(
        names(s2.query_entity("singularmem", &q(None, None)).unwrap()),
        vec!["meilisearch"],
        "open facts only by default"
    );
    assert_eq!(
        names(
            s2.query_entity("singularmem", &q(Some("2026-08-01"), None))
                .unwrap()
        ),
        vec!["tantivy"]
    );
    assert_eq!(
        names(
            s2.query_entity("singularmem", &q(Some("2026-09-01"), None))
                .unwrap()
        ),
        vec!["meilisearch"],
        "half-open: valid_to excluded, valid_from included"
    );
    assert_eq!(
        names(
            s2.query_entity("singularmem", &q(None, Some("2026-07-01")))
                .unwrap()
        ),
        vec!["tantivy"],
        "what we believed before the supersede"
    );
    // The closing revision inherited the original's NULL `valid_from`, which
    // the spec defines as "since unknown" — so it is still valid at any point
    // before its `valid_to`. (The task brief expected `is_empty()` here; that
    // contradicts the spec's as-of rule, § "Revisions and the two time axes",
    // which is the contract. Flagged in the task report.)
    assert_eq!(
        names(
            s2.query_entity("singularmem", &q(Some("2025-01-01"), None))
                .unwrap()
        ),
        vec!["tantivy"],
        "NULL valid_from means 'since unknown', not 'since recorded'"
    );
}

#[test]
fn supersede_is_atomic_and_tolerates_missing_old() {
    let (_d, s) = store();
    s.add_fact(NewFact::triple("a", "p", "old")).unwrap();
    let bad_new = entity("   ");
    assert!(s
        .supersede_fact("a", "p", &entity("old"), bad_new, None, None)
        .is_err());
    assert_eq!(
        s.graph_stats(None).unwrap().open_facts,
        1,
        "old fact still open after a failed supersede"
    );
    let (old, new) = s
        .supersede_fact("b", "p", &entity("nothing"), entity("new"), None, None)
        .unwrap();
    assert!(old.is_none());
    assert_eq!(new.subject.name, "b");
}

#[test]
fn directions_scopes_predicates_timeline_entities() {
    let (_d, s) = store();
    let mut f = NewFact::triple("jonas", "owns", "singularmem");
    f.scope = Some("claude-code/singularmem".into());
    s.add_fact(f).unwrap();
    let mut g = NewFact::triple("singularmem", "uses", "tantivy");
    g.scope = Some("claude-code/singularmem".into());
    g.valid_from = Some(ts("2026-05-16"));
    s.add_fact(g).unwrap();
    let mut h = NewFact::triple("other", "uses", "tantivy");
    h.scope = Some("claude-code/other".into());
    s.add_fact(h).unwrap();
    let q = |dir| GraphQuery {
        direction: dir,
        ..Default::default()
    };
    assert_eq!(
        s.query_entity("singularmem", &q(Direction::Outgoing))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.query_entity("singularmem", &q(Direction::Incoming))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.query_entity("singularmem", &q(Direction::Both))
            .unwrap()
            .len(),
        2
    );
    let scoped = GraphQuery {
        scope: Some(ScopeFilter::descendants("claude-code/singularmem").unwrap()),
        ..Default::default()
    };
    assert_eq!(s.query_predicate("uses", &scoped).unwrap().len(), 1);
    assert_eq!(
        s.query_predicate("uses", &GraphQuery::default())
            .unwrap()
            .len(),
        2
    );
    let tl = s.timeline(Some("tantivy"), None).unwrap();
    assert_eq!(tl.len(), 2);
    assert!(tl.iter().all(|e| e.current));
    assert_eq!(
        tl[0].fact.valid_from,
        Some(ts("2026-05-16")),
        "dated first, NULL valid_from last"
    );
    let ents = s.entities(None, None).unwrap();
    assert_eq!(
        ents.iter()
            .map(|e| e.entity.name.as_str())
            .collect::<Vec<_>>(),
        vec!["jonas", "other", "singularmem", "tantivy"]
    );
    assert_eq!(
        ents.iter()
            .find(|e| e.entity.name == "tantivy")
            .unwrap()
            .fact_count,
        2
    );
    let st = s.graph_stats(None).unwrap();
    assert_eq!(
        (st.entities, st.open_facts, st.closed_facts, st.predicates),
        (4, 3, 0, 2)
    );

    // Scoped listing/stats/timeline: a scope filter narrows the facts, and
    // with it the entities taking part in one.
    let other = ScopeFilter::descendants("claude-code/other").unwrap();
    let scoped_ents = s.entities(Some(&other), None).unwrap();
    assert_eq!(
        scoped_ents
            .iter()
            .map(|e| e.entity.name.as_str())
            .collect::<Vec<_>>(),
        vec!["other", "tantivy"]
    );
    assert_eq!(
        scoped_ents
            .iter()
            .find(|e| e.entity.name == "tantivy")
            .unwrap()
            .fact_count,
        1,
        "only the in-scope fact counts"
    );
    let scoped_stats = s.graph_stats(Some(&other)).unwrap();
    assert_eq!(
        (
            scoped_stats.entities,
            scoped_stats.open_facts,
            scoped_stats.closed_facts,
            scoped_stats.predicates
        ),
        (2, 1, 0, 1)
    );
    assert_eq!(s.timeline(None, Some(&other)).unwrap().len(), 1);
}

#[test]
fn provenance_and_read_only() {
    let d = TempDir::new().unwrap();
    let p = d.path().join("s.db");
    let item = {
        let s = Store::open(&p).unwrap();
        s.ingest(NewItem::text("we picked tantivy")).unwrap()
    };
    let s = Store::open(&p).unwrap();
    let mut f = NewFact::triple("singularmem", "uses", "tantivy");
    f.source_item_id = Some(item.id);
    assert_eq!(s.add_fact(f).unwrap().source_item_id, Some(item.id));
    let mut g = NewFact::triple("x", "y", "z");
    g.source_item_id = Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap());
    assert!(matches!(
        s.add_fact(g),
        Err(Error::Validation {
            field: "source_item_id",
            ..
        })
    ));
    let mut bad = NewFact::triple("x", "y", "z");
    bad.valid_from = Some(ts("2026-09-01"));
    bad.valid_to = Some(ts("2026-08-01"));
    assert!(matches!(
        s.add_fact(bad),
        Err(Error::Validation {
            field: "valid_window",
            ..
        })
    ));
    let ro =
        Store::open_with_options(&p, singularmem_core::StoreOptions { read_only: true }).unwrap();
    assert!(matches!(
        ro.add_fact(NewFact::triple("q", "r", "s")),
        Err(Error::ReadOnly { .. })
    ));
}
