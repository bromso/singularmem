//! Integration tests for the `singularmem` CLI. Each test invokes the binary
//! with `assert_cmd::Command::cargo_bin("singularmem")` and asserts on stdout,
//! stderr, and exit code.
//!
//! Tests use `--store $TEMP/store.db` to keep the user's data dir untouched.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn singularmem() -> Command {
    Command::cargo_bin("singularmem").expect("binary exists")
}

#[test]
fn version_flag_prints_singularmem_and_version() {
    singularmem()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("singularmem "));
}

#[test]
fn help_lists_all_subcommands() {
    singularmem()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("ingest"))
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("revisions"))
        .stdout(predicate::str::contains("export"));
}

#[test]
fn ingest_prints_id_then_get_returns_content() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "hello",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = String::from_utf8(out).unwrap().trim().to_string();
    assert_eq!(id.len(), 26, "ULID is 26 chars");

    singularmem()
        .args(["--store", db.to_str().unwrap(), "get", &id])
        .assert()
        .success()
        .stdout("hello");
}

#[test]
fn list_jsonl_includes_ingested_item() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "x",
            "--tag",
            "greeting",
        ])
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "list",
            "--tag",
            "greeting",
            "--format",
            "jsonl",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"content\":\"x\""));
}

#[test]
fn revisions_walks_chain_newest_first() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    let v1 = String::from_utf8(
        singularmem()
            .args(["--store", db.to_str().unwrap(), "ingest", "--content", "v1"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap()
    .trim()
    .to_string();

    let v2 = String::from_utf8(
        singularmem()
            .args([
                "--store",
                db.to_str().unwrap(),
                "ingest",
                "--content",
                "v2",
                "--supersedes",
                &v1,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap()
    .trim()
    .to_string();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "revisions",
            &v2,
            "--format",
            "ids",
        ])
        .assert()
        .success()
        .stdout(format!("{v2}\n{v1}\n"));
}

#[test]
fn export_first_line_is_meta() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Ingest at least one item so the export has something after the meta.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "ingest", "--content", "x"])
        .assert()
        .success();

    let out = singularmem()
        .args(["--store", db.to_str().unwrap(), "export"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let first = text.lines().next().expect("at least one line");
    assert!(first.contains("\"_singularmem_format\":\"export-v1\""));
}

#[test]
fn ingest_empty_content_exits_1_and_writes_to_stderr() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args(["--store", db.to_str().unwrap(), "ingest", "--content", ""])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("validation"));
}

#[test]
fn get_missing_id_exits_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Use a valid ULID (Crockford base32, no I/L/O/U) that doesn't exist in the store.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "get",
            "00000000000000000000000000",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn ingest_conflicting_input_modes_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "x",
            "--stdin",
        ])
        .assert()
        .failure();
}

// ── Task 10: search verb ────────────────────────────────────────────────────

#[test]
fn search_finds_ingested_item() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "Decision to use SQLite",
        ])
        .assert()
        .success();

    // Give Tantivy reader a moment to reload.
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "decision"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Decision"));
}

#[test]
fn search_errors_when_both_indexes_missing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Create store but never ingest and never run reindex.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "list"])
        .assert()
        .success();

    // With neither .tantivy/ nor .vectors/ on disk, auto mode must error.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "search",
            "anything",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no search index exists"));
}

#[test]
fn search_malformed_query_exits_1() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Ingest first so .tantivy/ exists; auto mode can then reach query parsing.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "test content for malformed query",
        ])
        .assert()
        .success();

    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "tags:"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn reindex_command_succeeds_on_empty_store() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .args(["--store", db.to_str().unwrap(), "list"])
        .assert()
        .success();
    singularmem()
        .args(["--store", db.to_str().unwrap(), "reindex"])
        .assert()
        .success();
}

#[test]
fn auto_wiring_makes_ingest_searchable() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "auto-wired item",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(300));
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "auto-wired",
            "--no-snippets",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("01")); // any ULID prefix in stdout = a hit
}

#[test]
fn no_index_flag_skips_hook_wiring() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "ingest",
            "--content",
            "not searchable",
        ])
        .assert()
        .success();
    // --no-index on ingest skips hook wiring so .tantivy/ is never created.
    // With neither .tantivy/ nor .vectors/ present, auto mode errors (exit 2).
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "not",
            "searchable",
        ])
        .assert()
        .failure()
        .code(2);
}

// ── Task 10 (Phase E): semantic-search verb ────────────────────────────────

fn derive_vectors_path_for_test(db: &std::path::Path) -> std::path::PathBuf {
    let mut s = db.to_path_buf().into_os_string();
    s.push(".vectors");
    std::path::PathBuf::from(s)
}

#[test]
fn semantic_search_with_mock_embedder_finds_ingested_item() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Pre-create the .vectors/ dir so auto-wiring fires during ingest.
    // (reindex --with-embeddings creates this in production; we shortcut here.)
    let vectors_path = derive_vectors_path_for_test(&db);
    std::fs::create_dir_all(&vectors_path).unwrap();

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "cat sat on mat",
        ])
        .assert()
        .success();

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "semantic-search",
            "cat sat on mat",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("01")); // any ULID prefix = a hit
}

#[test]
fn semantic_search_missing_index_exits_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    // No .vectors/ dir — EmbedderIndex::open should fail → exit 2.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "semantic-search",
            "anything",
        ])
        .assert()
        .failure()
        .code(2);
}

// ── Task 11 (Phase E): reindex --with-embeddings ──────────────────────────

#[test]
fn reindex_with_embeddings_creates_vectors_dir() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let vectors_path = derive_vectors_path_for_test(&db);

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "first item",
        ])
        .assert()
        .success();

    assert!(
        !vectors_path.exists(),
        ".vectors/ should not exist before reindex --with-embeddings"
    );

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .assert()
        .success();

    assert!(vectors_path.exists(), ".vectors/ should be created");
}

#[test]
fn reset_vectors_without_force_fails() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
            "--reset-vectors",
        ])
        .assert()
        .failure();
}

// ── Task 9: new search flags ──────────────────────────────────────────────

#[test]
fn search_help_lists_mode_flag() {
    singularmem()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("auto"))
        .stdout(predicate::str::contains("lexical"))
        .stdout(predicate::str::contains("semantic"))
        .stdout(predicate::str::contains("hybrid"));
}

#[test]
fn search_help_lists_show_ranks_and_json_flags() {
    singularmem()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--show-ranks"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--fetch-multiplier"))
        .stdout(predicate::str::contains("--rrf-k"));
}

// ── Task 10: mode dispatch tests ─────────────────────────────────────────

#[test]
fn search_default_mode_uses_hybrid_when_vectors_exist() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps over the lazy dog",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Build the vector sidecar so auto mode picks hybrid.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "fox"])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("rrf="));
}

#[test]
fn search_default_mode_falls_back_to_lexical_when_no_vectors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "a memorable phrase about brown foxes",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    // No reindex --with-embeddings, so .vectors/ does not exist.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "search", "foxes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bm25="));
}

#[test]
fn search_mode_lexical_explicit_works() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "lexical mode test fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "lexical",
            "lexical",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("bm25="));
}

#[test]
fn search_mode_semantic_explicit_works() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "semantic mode test fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "semantic",
            "semantic mode test fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("cos="));
}

#[test]
fn search_mode_hybrid_errors_when_vectors_missing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "lexical only fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "hybrid",
            "fixture",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "hybrid search requires both indexes",
        ))
        .stderr(predicate::str::contains("semantic index missing"));
}

#[test]
fn search_mode_hybrid_errors_when_lexical_missing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Ingest with --no-index so .tantivy/ is never created. Then run
    // reindex --with-embeddings only (which currently always builds the
    // tantivy sidecar too). To get a vectors-only state we delete .tantivy/
    // after the reindex.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "ingest",
            "--content",
            "semantic only fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();
    // Delete the tantivy sidecar that reindex built.
    let tantivy_dir = {
        let mut s = db.clone().into_os_string();
        s.push(".tantivy");
        std::path::PathBuf::from(s)
    };
    std::fs::remove_dir_all(&tantivy_dir).expect("remove tantivy dir");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "hybrid",
            "fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "hybrid search requires both indexes",
        ))
        .stderr(predicate::str::contains("lexical index missing"));
}

// ── Task 12 (Phase E): auto-wiring MultiHook ─────────────────────────────

#[test]
fn auto_wiring_writes_to_both_tantivy_and_embedder_after_reindex_with_embeddings() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Trigger .vectors/ creation via reindex --with-embeddings.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .assert()
        .success();

    // Now ingest. Both Tantivy and Embedder hooks should fire because .vectors/ exists.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "auto-wired-both item",
        ])
        .assert()
        .success();

    std::thread::sleep(std::time::Duration::from_millis(300));

    // Lexical search finds it. Both sidecars exist so auto mode picks hybrid;
    // inject mock embedder to stay fast and network-free.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args(["--store", db.to_str().unwrap(), "search", "auto-wired-both"])
        .assert()
        .success();

    // Semantic search finds it.
    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "semantic-search",
            "auto-wired-both",
        ])
        .assert()
        .success();
}

#[test]
fn search_show_ranks_flag_includes_columns() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "show ranks fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--show-ranks",
            "show ranks fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("lex="))
        .stdout(predicate::str::contains("sem="));
}

#[test]
fn search_json_flag_emits_valid_json() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "json output fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--json",
            "fixture",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success(), "expected success, got {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let hits = parsed
        .get("hits")
        .expect("hits field")
        .as_array()
        .expect("array");
    assert!(!hits.is_empty(), "expected at least one hit");
    let h0 = &hits[0];
    assert!(h0.get("id").is_some(), "hit must have id");
    assert!(h0.get("score").is_some(), "hit must have score");
    assert!(h0.get("score_kind").is_some(), "hit must have score_kind");
    // lexical_rank/semantic_rank may be null but the keys must exist.
    assert!(h0.get("lexical_rank").is_some());
    assert!(h0.get("semantic_rank").is_some());
}

#[test]
fn retrieve_help_lists_flags_and_default_adapter() {
    singularmem()
        .args(["retrieve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--adapter"))
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--min-score"))
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--fetch-multiplier"))
        .stdout(predicate::str::contains("--rrf-k"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--show-elapsed"))
        .stdout(predicate::str::contains("default: plain"));
}

#[test]
fn semantic_search_deprecated_alias_still_works() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "deprecation alias fixture",
        ])
        .assert()
        .success();
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "semantic-search",
            "deprecation alias fixture",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .assert()
        .success()
        .stdout(predicate::str::contains("cos="))
        .stderr(predicate::str::contains("deprecated"));
}

#[test]
fn retrieve_with_default_adapter_emits_plain_format() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args(["--store", db.to_str().unwrap(), "retrieve", "fox"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## memory 1"))
        .stdout(predicate::str::contains("the quick brown fox"));
}

#[test]
fn retrieve_json_flag_emits_valid_json() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "json output fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--json",
            "fixture",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success(), "expected success, got {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let blocks = parsed
        .get("blocks")
        .expect("blocks field")
        .as_array()
        .expect("array");
    assert!(!blocks.is_empty(), "expected at least one block");
    let b0 = &blocks[0];
    for field in &[
        "id",
        "content",
        "score",
        "score_kind",
        "source",
        "tags",
        "created_at",
    ] {
        assert!(b0.get(field).is_some(), "block missing field {field}: {b0}");
    }
    assert!(parsed.get("query").is_some());
    assert!(parsed.get("elapsed").is_some());
    assert!(parsed.get("total_considered").is_some());
}

#[test]
fn retrieve_unknown_adapter_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // No need to ingest — the unknown-adapter check fails before any I/O.
    // Use a deliberately-fake adapter name; each new cloud adapter
    // (sub-projects 3b/3c/3d) makes its own name a valid choice, so the
    // unknown-adapter test must use something that will never become valid.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "nonexistent",
            "anything",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown adapter 'nonexistent'"))
        .stderr(predicate::str::contains(
            "known adapters: plain, claude, openai",
        ));
}

#[test]
fn retrieve_empty_query_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "anything",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args(["--store", db.to_str().unwrap(), "retrieve", ""])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("query is empty"));
}

#[test]
fn retrieve_no_indexes_errors_like_search() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Create store but never ingest and never run reindex.
    singularmem()
        .args(["--store", db.to_str().unwrap(), "list"])
        .assert()
        .success();

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "retrieve",
            "anything",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no search index exists"));
}

#[test]
fn retrieve_mode_hybrid_errors_like_search() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "lexical only fixture",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--mode",
            "hybrid",
            "fixture",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "hybrid search requires both indexes",
        ));
}

#[test]
fn retrieve_show_elapsed_writes_to_stderr() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--show-elapsed",
            "fox",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        stderr.contains("Retrieved") && stderr.contains("blocks"),
        "expected timing line in stderr, got: {stderr}"
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        !stdout.contains("Retrieved"),
        "timing should not be in stdout, got: {stdout}"
    );
}

#[test]
fn retrieve_limit_caps_block_count() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    for i in 0..10 {
        singularmem()
            .args([
                "--store",
                db.to_str().unwrap(),
                "ingest",
                "--content",
                &format!("repeated word {i}"),
            ])
            .assert()
            .success();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));

    let out = singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--limit",
            "2",
            "repeated",
        ])
        .output()
        .expect("ran");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let heading_count = stdout.matches("## memory").count();
    assert_eq!(
        heading_count, 2,
        "expected exactly 2 memory headings, got {heading_count} in:\n{stdout}"
    );
}

#[test]
fn retrieve_with_claude_adapter_emits_xml_documents() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "claude",
            "fox",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<documents>"))
        .stdout(predicate::str::contains("<document index=\"1\">"))
        .stdout(predicate::str::contains("<document_content>"))
        .stdout(predicate::str::contains("the quick brown fox"))
        .stdout(predicate::str::contains("</document_content>"))
        .stdout(predicate::str::contains("</document>"))
        .stdout(predicate::str::contains("</documents>"));
}

#[test]
fn retrieve_with_openai_adapter_emits_bracket_citations() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "openai",
            "fox",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Use the following retrieved memories. Cite by [N] index.",
        ))
        .stdout(predicate::str::contains("[1]"))
        .stdout(predicate::str::contains("the quick brown fox"));
}
