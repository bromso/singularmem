use std::path::PathBuf;
use std::sync::Arc;

use singularmem_bench::dataset::load;
use singularmem_bench::metrics::{summarise, SearchMode};
use singularmem_bench::runner::{run_question, run_question_in, RunConfig, SharedEmbedder};
use singularmem_search::testing::MockEmbedder;
use singularmem_search::{Embedder, Error as SearchError};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/longmemeval-mini.json")
}

fn mock() -> SharedEmbedder {
    SharedEmbedder::new(Arc::new(MockEmbedder::default()))
}

/// An [`Embedder`] whose `embed` (and therefore the default `embed_batch`)
/// always fails, to exercise the runner's post-ingest doc-count guard: the
/// vector index hook's `on_ingest` fails for every item, `Store::ingest_many`
/// only logs that and swallows it, so without the guard the question would
/// come back with empty hits and `error: None`.
#[derive(Default)]
struct FailingEmbedder;

impl Embedder for FailingEmbedder {
    fn dim(&self) -> usize {
        384
    }
    fn model_id(&self) -> &'static str {
        "failing-embedder@v1"
    }
    fn embed(&self, _content: &str) -> singularmem_search::Result<Vec<f32>> {
        Err(SearchError::Embedding {
            context: "test",
            reason: "always fails".to_string(),
        })
    }
}

#[test]
fn lexical_hits_on_the_fixture_are_exact() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Lexical],
        ks: vec![1, 5],
    };
    let results: Vec<_> = qs.iter().map(|q| run_question(q, &cfg, None)).collect();
    for r in &results {
        assert!(r.error.is_none(), "{}: {:?}", r.id, r.error);
        assert!(
            r.ingest_ms < 60_000,
            "{}: ingest took {} ms",
            r.id,
            r.ingest_ms
        );
        assert!(
            r.items_ingested > 0,
            "{}: items_ingested should count actually-ingested items",
            r.id
        );
    }
    // q1..q4 hit at rank 1; q5 is a lexical miss; q6_abs is an abstention.
    assert_eq!(results[0].hits[&SearchMode::Lexical][0], "s1a");
    assert_eq!(results[1].hits[&SearchMode::Lexical][0], "s2b");
    assert_eq!(results[2].hits[&SearchMode::Lexical][0], "s3c");
    let q4 = &results[3].hits[&SearchMode::Lexical][0];
    assert!(q4 == "s4a" || q4 == "s4c", "{q4}");
    assert!(
        !results[4].hits[&SearchMode::Lexical].contains(&"s5a".to_string()),
        "q5 is written to be a lexical miss: {:?}",
        results[4].hits
    );
    assert!(results[5].abstention);

    let s = summarise(&results, &cfg.ks);
    let m = &s.overall[&SearchMode::Lexical];
    assert_eq!(m.n, 5);
    assert!((m.recall[&1] - 0.8).abs() < 1e-9, "{:?}", m.recall);
    assert!((m.recall[&5] - 0.8).abs() < 1e-9, "{:?}", m.recall);
    assert!((m.mrr - 0.8).abs() < 1e-9);
    assert_eq!(s.abstentions, 1);
    assert_eq!(s.errors, 0);
}

#[test]
fn hit_lists_are_distinct_sessions_bounded_by_max_k() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![
            SearchMode::Lexical,
            SearchMode::Semantic,
            SearchMode::Hybrid,
        ],
        ks: vec![1, 2],
    };
    let emb = mock();
    let r = run_question(&qs[0], &cfg, Some(&emb));
    assert!(r.error.is_none(), "{:?}", r.error);
    for mode in SearchMode::ALL {
        let hits = &r.hits[&mode];
        assert!(hits.len() <= 2, "{mode}: {hits:?}");
        let mut dedup = hits.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), hits.len(), "{mode}: duplicates in {hits:?}");
        for h in hits {
            assert!(
                qs[0].haystack.iter().any(|s| &s.id == h),
                "{mode}: unknown session {h}"
            );
        }
        assert!(r.query_us.contains_key(&mode));
    }
}

#[test]
fn semantic_mode_without_an_embedder_is_a_per_question_error() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Semantic],
        ks: vec![1],
    };
    let r = run_question(&qs[0], &cfg, None);
    let err = r.error.expect("error recorded");
    assert!(err.contains("embedder"), "{err}");
    assert!(r.hits.is_empty());
}

#[test]
fn a_failing_question_does_not_abort_the_batch() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Lexical],
        ks: vec![1],
    };
    // Force a failure by making the temp root unwritable: pass a regular
    // file as the scratch root.
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("file");
    std::fs::write(&not_a_dir, b"x").unwrap();
    let results: Vec<_> = qs
        .iter()
        .map(|q| run_question_in(q, &cfg, None, &not_a_dir))
        .collect();
    assert_eq!(results.len(), 6);
    assert!(results.iter().all(|r| r.error.is_some()));
    let s = summarise(&results, &cfg.ks);
    assert_eq!(s.errors, 6);
}

#[test]
fn a_broken_vector_index_hook_is_a_recorded_error_not_empty_hits() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Semantic],
        ks: vec![1],
    };
    let failing = SharedEmbedder::new(Arc::new(FailingEmbedder));
    let r = run_question(&qs[0], &cfg, Some(&failing));
    let err = r
        .error
        .expect("index hook failures must surface as an error");
    assert!(err.contains("dropped"), "{err}");
    assert!(r.hits.is_empty());
}
