use std::collections::BTreeMap;

use singularmem_bench::dataset::QuestionType;
use singularmem_bench::metrics::{summarise, QuestionResult, SearchMode};

fn result(
    id: &str,
    kind: QuestionType,
    abstention: bool,
    evidence: &[&str],
    hits: &[&str],
    error: Option<&str>,
) -> QuestionResult {
    let mut h = BTreeMap::new();
    h.insert(
        SearchMode::Lexical,
        hits.iter().map(ToString::to_string).collect::<Vec<_>>(),
    );
    let mut q = BTreeMap::new();
    q.insert(SearchMode::Lexical, 100_u64);
    QuestionResult {
        id: id.into(),
        kind,
        abstention,
        evidence: evidence.iter().map(ToString::to_string).collect(),
        hits: h,
        ingest_ms: 10,
        query_ms: q,
        error: error.map(ToString::to_string),
    }
}

#[test]
fn recall_and_mrr_by_hand() {
    // q1: evidence at rank 1 -> R@1=1, R@5=1, RR=1
    // q2: evidence at rank 3 -> R@1=0, R@5=1, RR=1/3
    // q3: miss              -> 0, 0, 0
    let rs = vec![
        result(
            "q1",
            QuestionType::MultiSession,
            false,
            &["a"],
            &["a", "x", "y"],
            None,
        ),
        result(
            "q2",
            QuestionType::MultiSession,
            false,
            &["b"],
            &["x", "y", "b"],
            None,
        ),
        result(
            "q3",
            QuestionType::KnowledgeUpdate,
            false,
            &["c"],
            &["x", "y", "z"],
            None,
        ),
    ];
    let s = summarise(&rs, &[1, 5]);
    let m = &s.overall[&SearchMode::Lexical];
    assert_eq!(m.n, 3);
    assert!((m.recall[&1] - 1.0 / 3.0).abs() < 1e-9);
    assert!((m.recall[&5] - 2.0 / 3.0).abs() < 1e-9);
    assert!((m.mrr - (1.0 + 1.0 / 3.0) / 3.0).abs() < 1e-9);
    // 3 queries in 300 ms -> 10 q/s
    assert!((m.queries_per_s - 10.0).abs() < 1e-9);

    let multi = &s.by_type[&QuestionType::MultiSession][&SearchMode::Lexical];
    assert_eq!(multi.n, 2);
    assert!((multi.recall[&5] - 1.0).abs() < 1e-9);
    assert_eq!(s.abstentions, 0);
    assert_eq!(s.errors, 0);
}

#[test]
fn multi_evidence_any_hit_counts() {
    let rs = vec![result(
        "q",
        QuestionType::MultiSession,
        false,
        &["a", "b"],
        &["b", "z"],
        None,
    )];
    let s = summarise(&rs, &[1]);
    assert!((s.overall[&SearchMode::Lexical].recall[&1] - 1.0).abs() < 1e-9);
}

#[test]
fn abstentions_and_errors_are_excluded_and_counted() {
    let rs = vec![
        result(
            "q1",
            QuestionType::SingleSessionUser,
            false,
            &["a"],
            &["a"],
            None,
        ),
        result(
            "q2_abs",
            QuestionType::SingleSessionUser,
            true,
            &[],
            &["a"],
            None,
        ),
        result(
            "q3",
            QuestionType::SingleSessionUser,
            false,
            &["a"],
            &[],
            Some("boom"),
        ),
    ];
    let s = summarise(&rs, &[1]);
    let m = &s.overall[&SearchMode::Lexical];
    assert_eq!(m.n, 1, "only q1 is scored");
    assert!((m.recall[&1] - 1.0).abs() < 1e-9);
    assert_eq!(s.abstentions, 1);
    assert_eq!(s.errors, 1);
}

#[test]
fn empty_input_yields_zero_metrics_not_nan() {
    let s = summarise(&[], &[1, 5]);
    assert!(s.overall.is_empty());
    assert_eq!(s.abstentions, 0);
}

#[test]
fn search_mode_names_round_trip() {
    for (name, mode) in [
        ("lexical", SearchMode::Lexical),
        ("semantic", SearchMode::Semantic),
        ("hybrid", SearchMode::Hybrid),
    ] {
        assert_eq!(mode.as_str(), name);
        assert_eq!(name.parse::<SearchMode>().unwrap(), mode);
    }
    assert!("fuzzy".parse::<SearchMode>().is_err());
}
