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
        s.get_entity("singularmem")
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
    let entities_before = s.entities(None, None).unwrap().len();
    let bad_new = entity("   ");
    assert!(s
        .supersede_fact("a", "p", &entity("old"), bad_new, None, None)
        .is_err());
    assert_eq!(
        s.graph_stats(None).unwrap().open_facts,
        1,
        "old fact still open after a failed supersede"
    );
    assert_eq!(
        s.entities(None, None).unwrap().len(),
        entities_before,
        "the rolled-back supersede created no entities either"
    );
    let (old, new) = s
        .supersede_fact("b", "p", &entity("nothing"), entity("new"), None, None)
        .unwrap();
    assert!(old.is_none());
    assert_eq!(new.subject.name, "b");
}

/// Seed a store with three facts spanning two scopes
/// (`claude-code/singularmem` and `claude-code/other`) for the
/// direction/scope/predicate/timeline/entities tests below.
fn seed_direction_and_scope_facts() -> (TempDir, Store) {
    let (dir, graph_store) = store();
    let mut owns_fact = NewFact::triple("jonas", "owns", "singularmem");
    owns_fact.scope = Some("claude-code/singularmem".into());
    graph_store.add_fact(owns_fact).unwrap();
    let mut uses_fact = NewFact::triple("singularmem", "uses", "tantivy");
    uses_fact.scope = Some("claude-code/singularmem".into());
    uses_fact.valid_from = Some(ts("2026-05-16"));
    graph_store.add_fact(uses_fact).unwrap();
    let mut other_uses_fact = NewFact::triple("other", "uses", "tantivy");
    other_uses_fact.scope = Some("claude-code/other".into());
    graph_store.add_fact(other_uses_fact).unwrap();
    (dir, graph_store)
}

#[test]
fn directions_scopes_predicates_timeline_entities() {
    let (_dir, graph_store) = seed_direction_and_scope_facts();
    let query_with_direction = |direction| GraphQuery {
        direction,
        ..Default::default()
    };
    assert_eq!(
        graph_store
            .query_entity("singularmem", &query_with_direction(Direction::Outgoing))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        graph_store
            .query_entity("singularmem", &query_with_direction(Direction::Incoming))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        graph_store
            .query_entity("singularmem", &query_with_direction(Direction::Both))
            .unwrap()
            .len(),
        2
    );
    let scoped = GraphQuery {
        scope: Some(ScopeFilter::descendants("claude-code/singularmem").unwrap()),
        ..Default::default()
    };
    assert_eq!(
        graph_store.query_predicate("uses", &scoped).unwrap().len(),
        1
    );
    assert_eq!(
        graph_store
            .query_predicate("uses", &GraphQuery::default())
            .unwrap()
            .len(),
        2
    );
    let tl = graph_store.timeline(Some("tantivy"), None).unwrap();
    assert_eq!(tl.len(), 2);
    assert!(tl.iter().all(|e| e.current));
    assert_eq!(
        tl[0].fact.valid_from,
        Some(ts("2026-05-16")),
        "dated first, NULL valid_from last"
    );
    let ents = graph_store.entities(None, None).unwrap();
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
    let st = graph_store.graph_stats(None).unwrap();
    assert_eq!(
        (st.entities, st.open_facts, st.closed_facts, st.predicates),
        (4, 3, 0, 2)
    );
}

#[test]
fn scoped_listing_stats_and_timeline_narrow_to_one_scope() {
    // A scope filter narrows the facts, and with it the entities taking
    // part in one.
    let (_dir, graph_store) = seed_direction_and_scope_facts();
    let other = ScopeFilter::descendants("claude-code/other").unwrap();
    let scoped_ents = graph_store.entities(Some(&other), None).unwrap();
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
    let scoped_stats = graph_store.graph_stats(Some(&other)).unwrap();
    assert_eq!(
        (
            scoped_stats.entities,
            scoped_stats.open_facts,
            scoped_stats.closed_facts,
            scoped_stats.predicates
        ),
        (2, 1, 0, 1)
    );
    assert_eq!(graph_store.timeline(None, Some(&other)).unwrap().len(), 1);
}

#[test]
fn provenance_and_read_only() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("s.db");
    let item = {
        let ingest_store = Store::open(&path).unwrap();
        ingest_store
            .ingest(NewItem::text("we picked tantivy"))
            .unwrap()
    };
    let graph_store = Store::open(&path).unwrap();
    let mut uses_fact = NewFact::triple("singularmem", "uses", "tantivy");
    uses_fact.source_item_id = Some(item.id);
    assert_eq!(
        graph_store.add_fact(uses_fact).unwrap().source_item_id,
        Some(item.id)
    );
    let mut bad_source_fact = NewFact::triple("x", "y", "z");
    bad_source_fact.source_item_id = Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap());
    assert!(matches!(
        graph_store.add_fact(bad_source_fact),
        Err(Error::Validation {
            field: "source_item_id",
            ..
        })
    ));
    let mut bad = NewFact::triple("x", "y", "z");
    bad.valid_from = Some(ts("2026-09-01"));
    bad.valid_to = Some(ts("2026-08-01"));
    assert!(matches!(
        graph_store.add_fact(bad),
        Err(Error::Validation {
            field: "valid_window",
            ..
        })
    ));
    let ro = Store::open_with_options(&path, singularmem_core::StoreOptions { read_only: true })
        .unwrap();
    assert!(matches!(
        ro.add_fact(NewFact::triple("q", "r", "s")),
        Err(Error::ReadOnly { .. })
    ));
}

#[test]
fn literal_values_and_kinds_are_trimmed() {
    let (_d, s) = store();
    let mut f = NewFact::triple("proj", "written_in", "unused");
    f.object = NewObject::Value("  Rust  ".into());
    f.subject_kind = Some("  project  ".into());
    let stored = s.add_fact(f).unwrap();
    assert!(
        matches!(stored.object, singularmem_core::graph::FactObject::Value(ref v) if v == "Rust"),
        "the literal is stored trimmed: {:?}",
        stored.object
    );
    assert_eq!(
        s.get_entity("proj").unwrap().unwrap().kind.as_deref(),
        Some("project"),
        "kind is stored trimmed"
    );

    // A trimmed kind on a later add matches the stored one rather than
    // tripping the immutability check.
    let mut g = NewFact::triple("proj", "has_author", "jonas");
    g.subject_kind = Some(" project ".into());
    s.add_fact(g).unwrap();

    // …and the untrimmed literal addresses the same triple on the way out.
    let closed = s
        .invalidate_fact(
            "proj",
            "written_in",
            &NewObject::Value("Rust".into()),
            None,
            Some(ts("2026-09-01")),
        )
        .unwrap();
    assert_eq!(closed.supersedes, Some(stored.id));
}

#[test]
fn value_objects_are_idempotent() {
    let (_d, s) = store();
    let value = || {
        let mut f = NewFact::triple("proj", "written_in", "unused");
        f.object = NewObject::Value("Rust".into());
        f
    };
    let a = s.add_fact(value()).unwrap();
    let b = s.add_fact(value()).unwrap();
    assert_eq!(a.id, b.id, "an identical open literal fact is a no-op");
    assert_eq!(s.graph_stats(None).unwrap().open_facts, 1);
}

#[test]
fn confidence_round_trips_and_is_bounded() {
    let (_d, s) = store();
    let mut f = NewFact::triple("a", "p", "b");
    f.confidence = 0.25;
    let stored = s.add_fact(f).unwrap();
    assert!(
        (stored.confidence - 0.25).abs() < f32::EPSILON,
        "0.25 is exact in binary floating point: {}",
        stored.confidence
    );
    assert!(
        (s.get_fact(stored.id).unwrap().confidence - 0.25).abs() < f32::EPSILON,
        "and survives the REAL round trip"
    );
    for bad in [1.5_f32, -0.1_f32] {
        let mut f = NewFact::triple("a", "q", "b");
        f.confidence = bad;
        assert!(
            matches!(
                s.add_fact(f),
                Err(Error::Validation {
                    field: "confidence",
                    ..
                })
            ),
            "{bad} is out of range"
        );
    }
}

#[test]
fn invalidating_before_valid_from_is_a_window_error() {
    let (_d, s) = store();
    let mut f = NewFact::triple("a", "p", "b");
    f.valid_from = Some(ts("2026-06-01"));
    s.add_fact(f).unwrap();
    assert!(matches!(
        s.invalidate_fact("a", "p", &entity("b"), None, Some(ts("2026-05-01"))),
        Err(Error::Validation {
            field: "valid_window",
            ..
        })
    ));
    assert_eq!(
        s.graph_stats(None).unwrap().open_facts,
        1,
        "the refused invalidate wrote nothing"
    );
}

/// Three write events under three different clocks: open at A, supersede at
/// B, supersede again at C. Each supersede closes the standing head (a
/// two-revision chain) and opens a fresh one, so `fact_history` returns the
/// closed pair for the chains behind it, and a `recorded_at` between the
/// second and third events sees exactly the second belief.
#[test]
fn revision_chains_and_recorded_at_between_writes() {
    let d = TempDir::new().unwrap();

    let s1 = store_at(&d, "2026-01-01T00:00:00Z");
    let mut first = NewFact::triple("a", "p", "x");
    first.valid_from = Some(ts("2026-01-01"));
    let r1 = s1.add_fact(first).unwrap();
    drop(s1);

    let s2 = store_at(&d, "2026-02-01T00:00:00Z");
    let (closed1, r3) = s2
        .supersede_fact(
            "a",
            "p",
            &entity("x"),
            entity("y"),
            None,
            Some(ts("2026-02-01")),
        )
        .unwrap();
    let r2 = closed1.expect("the open head was there to close");
    drop(s2);

    let s3 = store_at(&d, "2026-03-01T00:00:00Z");
    let (closed2, r5) = s3
        .supersede_fact(
            "a",
            "p",
            &entity("y"),
            entity("z"),
            None,
            Some(ts("2026-03-01")),
        )
        .unwrap();
    let r4 = closed2.expect("the second head was there to close");

    assert_eq!(
        s3.fact_history(r1.id)
            .unwrap()
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![r1.id, r2.id],
        "oldest first, from either end of the chain"
    );
    assert_eq!(
        s3.fact_history(r2.id)
            .unwrap()
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![r1.id, r2.id]
    );
    assert_eq!(
        s3.fact_history(r4.id)
            .unwrap()
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![r3.id, r4.id]
    );
    assert_eq!(s3.fact_history(r5.id).unwrap().len(), 1, "the open head");

    let objects = |q: &GraphQuery| {
        s3.query_entity("a", q)
            .unwrap()
            .into_iter()
            .map(|f| match f.object {
                singularmem_core::graph::FactObject::Entity(e) => e.name,
                singularmem_core::graph::FactObject::Value(v) => v,
            })
            .collect::<Vec<_>>()
    };
    let believed_at = |r: &str| GraphQuery {
        recorded_at: Some(ts(r)),
        direction: Direction::Outgoing,
        ..Default::default()
    };
    assert_eq!(
        objects(&believed_at("2026-02-15T00:00:00Z")),
        vec!["y"],
        "between the second and third writes we believed exactly y"
    );
    assert_eq!(objects(&believed_at("2026-01-15T00:00:00Z")), vec!["x"]);
    assert_eq!(objects(&GraphQuery::default()), vec!["z"]);
}

/// A closing revision must leave the row it closes byte-identical. Compared
/// column by column through a second `rusqlite` connection, not through the
/// typed API, so a silent `UPDATE` cannot hide behind parsing.
#[test]
fn invalidate_leaves_the_original_row_byte_identical() {
    type RawRow = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        f64,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    );

    fn raw_row(path: &std::path::Path, id: &str) -> RawRow {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.query_row(
            "SELECT id, subject_id, predicate, object_id, object_value, valid_from, valid_to, \
             confidence, source_item_id, scope, supersedes, recorded_at \
             FROM facts WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            },
        )
        .unwrap()
    }

    let d = TempDir::new().unwrap();
    let path = d.path().join("s.db");
    let s = store_at(&d, "2026-06-01T00:00:00Z");
    let mut f = NewFact::triple("a", "p", "b");
    f.valid_from = Some(ts("2026-06-01"));
    f.confidence = 0.5;
    f.scope = Some("proj/sub".into());
    let original = s.add_fact(f).unwrap();
    let before = raw_row(&path, &original.id.to_string());

    s.invalidate_fact(
        "a",
        "p",
        &entity("b"),
        Some("proj/sub"),
        Some(ts("2026-09-01")),
    )
    .unwrap();

    assert_eq!(
        raw_row(&path, &original.id.to_string()),
        before,
        "every column of the superseded row is unchanged"
    );
}

/// Two rows superseding the same revision is a fork the store's own writes
/// cannot produce; `fact_history` must say so rather than pick a branch.
#[test]
fn forked_chain_is_reported_not_guessed() {
    let d = TempDir::new().unwrap();
    let path = d.path().join("s.db");
    let s = store_at(&d, "2026-06-01T00:00:00Z");
    let head = s.add_fact(NewFact::triple("a", "p", "b")).unwrap();
    drop(s);

    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for fork_id in ["01ARZ3NDEKTSV4RRFFQ69G5FAV", "01ARZ3NDEKTSV4RRFFQ69G5FAW"] {
            conn.execute(
                "INSERT INTO facts \
                 (id, subject_id, predicate, object_id, object_value, valid_from, valid_to, \
                  confidence, source_item_id, scope, supersedes, recorded_at) \
                 SELECT ?1, subject_id, predicate, object_id, object_value, valid_from, \
                        '2026-09-01T00:00:00Z', confidence, source_item_id, scope, id, \
                        recorded_at \
                 FROM facts WHERE id = ?2",
                rusqlite::params![fork_id, head.id.to_string()],
            )
            .unwrap();
        }
    }

    let s = store_at(&d, "2026-10-01T00:00:00Z");
    let err = s.fact_history(head.id).unwrap_err();
    assert!(
        matches!(err, Error::AmbiguousFactRevision { ref candidates } if candidates.len() == 2),
        "{err:?}"
    );
    assert!(err.to_string().contains("forks"), "{err}");
}

/// Graph timestamps are stored at a fixed nine-digit precision so `SQLite`'s
/// string comparison is chronological. With `Timestamp`'s trimming `Display`
/// the clock-minted `…T03:12:00.788Z` sorted *before* the user-supplied
/// `…T03:12:00Z` (`'.'` < `'Z'`), which inverted `--recorded-at` answers
/// inside a one-second window.
#[test]
fn recorded_at_ordering_survives_sub_second_writes() {
    let d = TempDir::new().unwrap();

    let s1 = store_at(&d, "2026-09-05T03:11:59.745Z");
    s1.add_fact(NewFact::triple("a", "p", "x")).unwrap();
    drop(s1);

    let s2 = store_at(&d, "2026-09-05T03:12:00.788Z");
    s2.supersede_fact("a", "p", &entity("x"), entity("y"), None, None)
        .unwrap();

    let believed_at = |r: &str| {
        s2.query_entity(
            "a",
            &GraphQuery {
                recorded_at: Some(ts(r)),
                direction: Direction::Outgoing,
                ..Default::default()
            },
        )
        .unwrap()
        .into_iter()
        .map(|f| match f.object {
            singularmem_core::graph::FactObject::Entity(e) => e.name,
            singularmem_core::graph::FactObject::Value(v) => v,
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(
        believed_at("2026-09-05T03:12:00Z"),
        vec!["x"],
        "the supersede was recorded at 03:12:00.788, after this instant"
    );
    assert_eq!(
        believed_at("2026-09-05T03:12:01Z"),
        vec!["y"],
        "and is visible one second later"
    );
}

/// Every graph timestamp column holds the 30-character fixed-precision form.
#[test]
fn stored_graph_timestamps_are_fixed_width() {
    let d = TempDir::new().unwrap();
    let path = d.path().join("s.db");
    let s = store_at(&d, "2026-09-05T03:11:59.745Z");
    let mut f = NewFact::triple("a", "p", "b");
    f.valid_from = Some(ts("2026-05-16"));
    s.add_fact(f).unwrap();
    s.invalidate_fact("a", "p", &entity("b"), None, Some(ts("2026-09-01")))
        .unwrap();
    drop(s);

    let conn = rusqlite::Connection::open(&path).unwrap();
    for (sql, column) in [
        ("SELECT created_at FROM entities", "entities.created_at"),
        (
            "SELECT valid_from FROM facts WHERE valid_from IS NOT NULL",
            "facts.valid_from",
        ),
        (
            "SELECT valid_to FROM facts WHERE valid_to IS NOT NULL",
            "facts.valid_to",
        ),
        ("SELECT recorded_at FROM facts", "facts.recorded_at"),
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let values: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!values.is_empty(), "{column} produced no rows to check");
        for v in values {
            assert_eq!(v.len(), 30, "{column} = {v:?}");
            assert!(v.ends_with('Z'), "{column} = {v:?}");
        }
    }
}
