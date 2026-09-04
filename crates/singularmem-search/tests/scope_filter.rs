//! Scope filtering across the lexical index, the semantic post-filter, and
//! the schema-version guard on pre-scope Tantivy sidecars.

use singularmem_core::{IndexHook, Item, ItemId, NewItem, ScopeFilter, Store};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{
    EmbedderIndex, Error, HybridSearchOptions, HybridSearcher, Index, Query, ScopeLookup,
    SearchOptions,
};
use std::collections::HashMap;
use std::str::FromStr;
use tempfile::TempDir;

fn item(id: &str, content: &str, scope: Option<&str>) -> Item {
    Item {
        id: ItemId::from_str(id).unwrap(),
        content: content.into(),
        created_at: jiff::Timestamp::now(),
        supersedes: None,
        tags: vec![],
        source: None,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        external_id: None,
        scope: scope.map(str::to_string),
    }
}

const A: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const B: &str = "01BX5ZZKBKACTAV9WEVGEMMVRZ";
const C: &str = "01CX5ZZKBKACTAV9WEVGEMMVRZ";
const D: &str = "01DX5ZZKBKACTAV9WEVGEMMVRZ";

fn seeded_index(dir: &TempDir) -> Index {
    let idx = Index::open(dir.path().join("idx")).unwrap();
    idx.on_ingest(&item(A, "zebra alpha", Some("a/b"))).unwrap();
    idx.on_ingest(&item(B, "zebra beta", Some("a/b/c")))
        .unwrap();
    idx.on_ingest(&item(C, "zebra gamma", Some("a/c"))).unwrap();
    idx.on_ingest(&item(D, "zebra delta", None)).unwrap();
    idx.commit().unwrap();
    idx
}

fn ids(hits: &[singularmem_search::Hit]) -> Vec<String> {
    let mut v: Vec<String> = hits.iter().map(|h| h.id.to_string()).collect();
    v.sort();
    v
}

#[test]
fn lexical_descendant_filter_excludes_sibling_and_unscoped() {
    let dir = TempDir::new().unwrap();
    let idx = seeded_index(&dir);
    let q = Query::parse("zebra").unwrap();
    let opts = SearchOptions {
        scope: Some(ScopeFilter::descendants("a/b").unwrap()),
        ..SearchOptions::default()
    };
    let res = idx.search(&q, opts).unwrap();
    assert_eq!(ids(&res.hits), vec![A, B]);
    assert_eq!(res.total_matched, 2);
}

#[test]
fn lexical_exact_filter_excludes_child() {
    let dir = TempDir::new().unwrap();
    let idx = seeded_index(&dir);
    let q = Query::parse("zebra").unwrap();
    let opts = SearchOptions {
        scope: Some(ScopeFilter::exact("a/b").unwrap()),
        ..SearchOptions::default()
    };
    assert_eq!(ids(&idx.search(&q, opts).unwrap().hits), vec![A]);
}

#[test]
fn no_filter_returns_everything() {
    let dir = TempDir::new().unwrap();
    let idx = seeded_index(&dir);
    let res = idx
        .search(&Query::parse("zebra").unwrap(), SearchOptions::default())
        .unwrap();
    assert_eq!(res.hits.len(), 4);
}

#[test]
fn old_schema_sidecar_is_schema_mismatch() {
    // Build an index with the *previous* schema by hand (no scope fields).
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("idx");
    {
        use tantivy::schema::{SchemaBuilder, FAST, INDEXED, STORED, STRING, TEXT};
        let mut b = SchemaBuilder::new();
        b.add_text_field("content", TEXT | STORED);
        b.add_text_field("tags", STRING | STORED);
        b.add_text_field("source", TEXT | STORED);
        b.add_text_field("id", STRING | STORED);
        b.add_date_field("created_at", INDEXED | STORED | FAST);
        b.add_text_field("supersedes", STRING | STORED);
        std::fs::create_dir_all(&p).unwrap();
        let mmap = tantivy::directory::MmapDirectory::open(&p).unwrap();
        tantivy::Index::open_or_create(mmap, b.build()).unwrap();
    }
    // `Index` is not `Debug`, so `Result::unwrap_err` is unavailable here.
    let Err(err) = Index::open(&p) else {
        panic!("expected IndexSchemaMismatch, but the stale sidecar opened cleanly");
    };
    assert!(matches!(err, Error::IndexSchemaMismatch { .. }), "{err:?}");
    assert!(err.to_string().contains("singularmem reindex"));
}

#[test]
fn hybrid_semantic_hits_are_post_filtered_through_the_store() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("s.db")).unwrap();
    let lex = Index::open(dir.path().join("idx")).unwrap();
    let sem =
        EmbedderIndex::open(dir.path().join("vec"), Box::new(MockEmbedder::default())).unwrap();
    let mut inside = NewItem::text("zebra inside");
    inside.scope = Some("a/b".into());
    let mut outside = NewItem::text("zebra outside");
    outside.scope = Some("x".into());
    for n in [inside, outside] {
        let it = store.ingest(n).unwrap();
        lex.on_ingest(&it).unwrap();
        sem.on_ingest(&it).unwrap();
    }
    lex.commit().unwrap();
    sem.commit().unwrap();

    let searcher = HybridSearcher::new(&lex, &sem).with_scope_lookup(&store);
    let opts = HybridSearchOptions {
        scope: Some(ScopeFilter::descendants("a").unwrap()),
        ..HybridSearchOptions::default()
    };
    let res = searcher.search("zebra", &opts).unwrap();
    assert_eq!(res.hits.len(), 1);
    assert_eq!(store.get(res.hits[0].id).unwrap().content, "zebra inside");

    // Semantic-only with a filter but no lookup is an error, not a silent
    // unscoped result.
    let bare = HybridSearcher::semantic_only(&sem);
    assert!(matches!(
        bare.search("zebra", &opts),
        Err(Error::ScopeLookupMissing)
    ));
}

#[test]
fn scope_clause_does_not_change_lexical_scores() {
    let dir = TempDir::new().unwrap();
    let idx = Index::open(dir.path().join("idx")).unwrap();
    idx.on_ingest(&item(A, "zebra", Some("a"))).unwrap();
    idx.on_ingest(&item(B, "zebra", Some("a/b/c"))).unwrap();
    idx.commit().unwrap();

    let unscoped = idx
        .search(&Query::parse("zebra").unwrap(), SearchOptions::default())
        .unwrap();
    let scoped = idx
        .search(
            &Query::parse("zebra").unwrap(),
            SearchOptions {
                scope: Some(ScopeFilter::descendants("a").unwrap()),
                ..SearchOptions::default()
            },
        )
        .unwrap();

    assert_eq!(unscoped.hits.len(), 2);
    assert_eq!(scoped.hits.len(), 2);

    let mut unscoped_scores: Vec<(String, f32)> = unscoped
        .hits
        .iter()
        .map(|h| (h.id.to_string(), h.score))
        .collect();
    let mut scoped_scores: Vec<(String, f32)> = scoped
        .hits
        .iter()
        .map(|h| (h.id.to_string(), h.score))
        .collect();
    unscoped_scores.sort_by(|a, b| a.0.cmp(&b.0));
    scoped_scores.sort_by(|a, b| a.0.cmp(&b.0));

    for ((id_a, score_a), (id_b, score_b)) in unscoped_scores.iter().zip(scoped_scores.iter()) {
        assert_eq!(id_a, id_b);
        assert!(
            (score_a - score_b).abs() < f32::EPSILON,
            "scope clause changed score for {id_a}: unscoped={score_a} scoped={score_b}"
        );
    }
}

#[test]
fn lexical_only_with_filter_needs_no_lookup() {
    let dir = TempDir::new().unwrap();
    let idx = seeded_index(&dir);
    let searcher = HybridSearcher::lexical_only(&idx);
    let opts = HybridSearchOptions {
        scope: Some(ScopeFilter::descendants("a/b").unwrap()),
        ..HybridSearchOptions::default()
    };
    let res = searcher.search("zebra", &opts).unwrap();
    let mut ids: Vec<String> = res.hits.iter().map(|h| h.id.to_string()).collect();
    ids.sort();
    assert_eq!(ids, vec![A, B]);
}

/// `ScopeLookup` backed by a plain map rather than a `Store`, to isolate
/// `scope_filter_hits`' behaviour from `Store::scope_of`.
struct MapLookup(HashMap<ItemId, Option<String>>);

impl ScopeLookup for MapLookup {
    fn scope_of(&self, id: ItemId) -> Option<String> {
        self.0.get(&id).cloned().flatten()
    }
}

#[test]
fn semantic_hit_with_unknown_scope_is_dropped() {
    let dir = TempDir::new().unwrap();
    let sem =
        EmbedderIndex::open(dir.path().join("vec"), Box::new(MockEmbedder::default())).unwrap();
    // Identical content (== the query text below) so both docs get the same
    // maximal cosine similarity from `MockEmbedder` — deterministic and
    // guaranteed positive, so the default `min_score: 0.0` cutoff inside
    // `search_semantic_only` never filters either one out before the scope
    // filter runs.
    let known = item(A, "zebra", Some("a/b"));
    let unknown = item(B, "zebra", None);
    sem.on_ingest(&known).unwrap();
    sem.on_ingest(&unknown).unwrap();
    sem.commit().unwrap();

    let mut map = HashMap::new();
    map.insert(known.id, Some("a/b".to_string()));
    map.insert(unknown.id, None);
    let lookup = MapLookup(map);

    let searcher = HybridSearcher::semantic_only(&sem).with_scope_lookup(&lookup);
    let opts = HybridSearchOptions {
        scope: Some(ScopeFilter::descendants("a").unwrap()),
        ..HybridSearchOptions::default()
    };
    let res = searcher.search("zebra", &opts).unwrap();
    assert_eq!(res.hits.len(), 1);
    assert_eq!(res.hits[0].id, known.id);
}

#[test]
fn hybrid_with_filter_but_no_lookup_errors() {
    let dir = TempDir::new().unwrap();
    let lex = Index::open(dir.path().join("idx")).unwrap();
    let sem =
        EmbedderIndex::open(dir.path().join("vec"), Box::new(MockEmbedder::default())).unwrap();
    let searcher = HybridSearcher::new(&lex, &sem);
    let opts = HybridSearchOptions {
        scope: Some(ScopeFilter::descendants("a").unwrap()),
        ..HybridSearchOptions::default()
    };
    assert!(matches!(
        searcher.search("zebra", &opts),
        Err(Error::ScopeLookupMissing)
    ));
}

#[test]
fn query_scoped_builder_filters() {
    let dir = TempDir::new().unwrap();
    let idx = seeded_index(&dir);
    let filter = ScopeFilter::descendants("a/b").unwrap();

    let via_options = idx
        .search(
            &Query::parse("zebra").unwrap(),
            SearchOptions {
                scope: Some(filter.clone()),
                ..SearchOptions::default()
            },
        )
        .unwrap();

    let via_builder = idx
        .search(
            &Query::parse("zebra").unwrap().scoped(&filter),
            SearchOptions::default(),
        )
        .unwrap();

    assert_eq!(ids(&via_options.hits), vec![A, B]);
    assert_eq!(ids(&via_options.hits), ids(&via_builder.hits));
}
