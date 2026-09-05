use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/longmemeval-mini.json")
}

fn bench() -> Command {
    let mut c = Command::cargo_bin("singularmem-bench").expect("binary exists");
    c.env("SINGULARMEM_TEST_EMBEDDER", "mock");
    c
}

#[test]
fn lexical_run_prints_markdown_report_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("out.json");
    let assert = bench()
        .args(["longmemeval"])
        .arg(fixture())
        .args(["--modes", "lexical", "--k", "1,5", "--quiet", "--json"])
        .arg(&json)
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        out.starts_with("# LongMemEval retrieval — longmemeval-mini.json"),
        "{out}"
    );
    assert!(
        out.contains("questions 6 (scored 5, abstention 1, errors 0)"),
        "{out}"
    );
    assert!(out.contains("| mode"), "{out}");
    assert!(out.contains("| lexical | 0.800 | 0.800 | 0.800 |"), "{out}");
    assert!(out.contains("## R@5 by question type"), "{out}");
    assert!(out.contains("| single-session-user"), "{out}");
    assert!(
        !out.contains("semantic"),
        "only the requested mode is reported: {out}"
    );

    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(doc["tool"], "singularmem-bench");
    assert_eq!(doc["dataset"]["questions"], 6);
    assert_eq!(doc["config"]["modes"], serde_json::json!(["lexical"]));
    assert_eq!(doc["config"]["ks"], serde_json::json!([1, 5]));
    assert_eq!(doc["questions"].as_array().unwrap().len(), 6);
    assert_eq!(doc["summary"]["abstentions"], 1);
    // Kept as `assert!(... == 64)` (the brief's verbatim assertion form)
    // rather than `assert_eq!`; the lint is purely cosmetic here.
    #[allow(clippy::manual_assert_eq)]
    {
        assert!(doc["dataset"]["sha256"].as_str().unwrap().len() == 64);
    }
    assert!(doc["commit"].is_string());
}

#[test]
fn all_modes_run_with_the_mock_embedder() {
    let out = bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--quiet", "--limit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("| lexical "), "{out}");
    assert!(out.contains("| semantic "), "{out}");
    assert!(out.contains("| hybrid "), "{out}");
    assert!(out.contains("questions 2 "), "{out}");
}

#[test]
#[cfg(unix)]
fn question_type_filter_and_seeded_limit() {
    let out = bench()
        .arg("longmemeval")
        .arg(fixture())
        .args([
            "--modes",
            "lexical",
            "--quiet",
            "--question-type",
            "multi-session",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("questions 1 (scored 1"), "{out}");

    let a = bench()
        .arg("longmemeval")
        .arg(fixture())
        .args([
            "--modes", "lexical", "--quiet", "--limit", "3", "--seed", "7", "--json",
        ])
        .arg("/dev/stdout")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let b = bench()
        .arg("longmemeval")
        .arg(fixture())
        .args([
            "--modes", "lexical", "--quiet", "--limit", "3", "--seed", "7", "--json",
        ])
        .arg("/dev/stdout")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    // Same seed -> same selection (compare the ids in the JSON part).
    let ids = |s: &[u8]| -> Vec<String> {
        let text = String::from_utf8_lossy(s);
        let json_start = text.find("{\"tool\"").expect("json present");
        let doc: serde_json::Value = serde_json::from_str(&text[json_start..]).unwrap();
        doc["questions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&a), ids(&b));
    assert_eq!(ids(&a).len(), 3);
}

#[test]
fn missing_file_exits_1_with_the_path() {
    bench()
        .args(["longmemeval", "/nonexistent/lme.json", "--modes", "lexical"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("/nonexistent/lme.json"));
}

#[test]
fn bad_k_and_bad_mode_are_usage_errors() {
    bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--k", "0"])
        .assert()
        .code(2);
    bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--modes", "fuzzy"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fuzzy"));
    bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--model", "gpt-embed"])
        .assert()
        .code(2);
}

#[test]
fn filter_that_leaves_nothing_exits_1() {
    bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--modes", "lexical", "--question-type", "brand-new-type"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no questions"));
}
