use std::collections::HashSet;
use std::path::PathBuf;

use singularmem_bench::dataset::{load, Error, QuestionType};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loads_the_mini_fixture() {
    let qs = load(&fixture("longmemeval-mini.json")).unwrap();
    assert_eq!(qs.len(), 6);

    let q1 = &qs[0];
    assert_eq!(q1.id, "q1");
    assert_eq!(q1.kind, QuestionType::SingleSessionUser);
    assert!(!q1.abstention);
    assert_eq!(q1.text, "What breed is my dog Biscuit?");
    assert_eq!(q1.date.as_deref(), Some("2024/03/01 (Fri) 10:00"));
    assert_eq!(q1.haystack.len(), 3);
    assert_eq!(q1.haystack[0].id, "s1a");
    assert_eq!(
        q1.haystack[0].date.as_deref(),
        Some("2024/02/01 (Thu) 09:00")
    );
    assert_eq!(q1.haystack[0].turns.len(), 2);
    assert_eq!(q1.haystack[0].turns[0].role, "user");
    assert_eq!(q1.evidence, HashSet::from(["s1a".to_string()]));

    let q4 = &qs[3];
    assert_eq!(q4.kind, QuestionType::MultiSession);
    assert_eq!(
        q4.evidence,
        HashSet::from(["s4a".to_string(), "s4c".to_string()])
    );

    let q6 = &qs[5];
    assert_eq!(q6.kind, QuestionType::KnowledgeUpdate);
    assert!(q6.abstention, "id ending in _abs is an abstention");
    assert!(q6.evidence.is_empty());

    let kinds: Vec<_> = qs.iter().map(|q| q.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            QuestionType::SingleSessionUser,
            QuestionType::SingleSessionAssistant,
            QuestionType::SingleSessionPreference,
            QuestionType::MultiSession,
            QuestionType::TemporalReasoning,
            QuestionType::KnowledgeUpdate,
        ]
    );
}

#[test]
fn unknown_question_type_is_preserved_not_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.json");
    std::fs::write(
        &p,
        r#"[{"question_id":"z","question_type":"brand-new-type","question":"?",
             "haystack_session_ids":["a"],"haystack_dates":["d"],
             "haystack_sessions":[[{"role":"user","content":"c","has_answer":true,"extra":1}]],
             "answer_session_ids":["a"],"unexpected_top_level":true}]"#,
    )
    .unwrap();
    let qs = load(&p).unwrap();
    assert_eq!(qs[0].kind, QuestionType::Other("brand-new-type".into()));
    assert_eq!(qs[0].kind.to_string(), "brand-new-type");
    assert_eq!(qs[0].haystack[0].turns[0].content, "c");
}

#[test]
fn parallel_array_length_mismatch_names_the_question() {
    let err = load(&fixture("longmemeval-bad-lengths.json")).unwrap_err();
    match err {
        Error::Shape { index, field, .. } => {
            assert_eq!(index, 0);
            assert_eq!(field, "haystack_dates");
        }
        other => panic!("expected Shape, got {other:?}"),
    }
    assert!(err.to_string().contains("question 0"), "{err}");
}

#[test]
fn missing_file_is_an_io_error_with_the_path() {
    let err = load(&fixture("does-not-exist.json")).unwrap_err();
    assert!(matches!(err, Error::Io { .. }));
    assert!(err.to_string().contains("does-not-exist.json"));
}

#[test]
fn question_type_display_and_parse_round_trip() {
    for (name, kind) in [
        ("single-session-user", QuestionType::SingleSessionUser),
        (
            "single-session-assistant",
            QuestionType::SingleSessionAssistant,
        ),
        (
            "single-session-preference",
            QuestionType::SingleSessionPreference,
        ),
        ("multi-session", QuestionType::MultiSession),
        ("temporal-reasoning", QuestionType::TemporalReasoning),
        ("knowledge-update", QuestionType::KnowledgeUpdate),
    ] {
        assert_eq!(QuestionType::from(name), kind);
        assert_eq!(kind.to_string(), name);
    }
}
