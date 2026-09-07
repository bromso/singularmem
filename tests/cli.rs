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
    assert!(first.contains("\"_singularmem_format\":\"export-v2\""));
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

/// `reindex --with-embeddings` without `--reset-vectors` re-embeds every item
/// into the *existing* vector index. The item's new vector must replace its
/// old one, not sit beside it: before the fix the graph doubled on every
/// reindex and semantic search returned the same id twice.
#[test]
fn reindex_with_embeddings_twice_does_not_double_the_vector_index() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let content = "doubling fixture for the reindex idempotence check";

    singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            content,
        ])
        .assert()
        .success();

    for _ in 0..2 {
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
    }

    // Semantic mode answers straight from the vector index and prints one
    // line per hit, so querying the item's own text — a self-similarity of
    // ~1.0, one line per *key* the id holds — counts its vectors.
    let out = singularmem()
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "--mode",
            "semantic",
            "--limit",
            "20",
            content,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert_eq!(
        stdout.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "a second reindex must replace the item's vector, not add a second one; got:\n{stdout}"
    );
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
            "known adapters: plain, claude, openai, gemini",
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

#[test]
fn retrieve_with_gemini_adapter_emits_source_headers() {
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
            "gemini",
            "fox",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Use the following sources to ground your answer.",
        ))
        .stdout(predicate::str::contains("Source 1"))
        .stdout(predicate::str::contains("the quick brown fox"));
}

fn fixture_transcripts() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts")
}

#[test]
fn ingest_transcript_is_idempotent_and_searchable() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fx = fixture_transcripts();

    // The fixture's malformed line 15 makes failed=1 → exit 1 (see the
    // dry-run test below, which documents the same reasoning).
    singularmem()
        .args(["--store", db_s, "ingest-transcript", fx.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains(
            "ingested 4, skipped 0 existing, 5 filtered, 1 failed across 1 files",
        ));

    // Re-parsing the fixture always fails on the same malformed line, so
    // this idempotent re-run also exits 1 despite ingesting nothing new.
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-transcript",
            fx.to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ingested 0, skipped 4 existing"));

    singularmem()
        .args(["--store", db_s, "search", "cargo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo")); // snippet may wrap the term in <mark>

    singularmem()
        .args([
            "--store",
            db_s,
            "list",
            "--tag",
            "role:user",
            "--format",
            "ids",
        ])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 2));
}

#[test]
fn ingest_transcript_exit_code_reflects_failures_and_dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fx = fixture_transcripts();

    // The fixture contains one malformed line → failed=1 → exit 1.
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-transcript",
            fx.to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ingested 4"));
    singularmem()
        .args(["--store", db_s, "list", "--format", "ids"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn ingest_transcript_project_filter() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fx = fixture_transcripts();
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-transcript",
            fx.to_str().unwrap(),
            "--project",
            "/home/me/proj",
        ])
        .assert()
        .code(1) // malformed line still counts
        .stderr(predicate::str::contains("ingested 3"));
}

#[test]
fn ingest_transcript_missing_path_is_exit_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest-transcript",
            "/definitely/missing",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("path not found"));
}

#[test]
fn ingest_transcript_with_explicit_path_does_not_need_home() {
    // `resolve_ingest_files`'s default root (`$HOME/.claude/projects`) is
    // only computed when `paths` is empty; an explicit path must not require
    // `HOME` to be resolvable at all.
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let fx = fixture_transcripts();
    singularmem()
        .env_remove("HOME")
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest-transcript",
            fx.to_str().unwrap(),
        ])
        .assert()
        .code(1) // malformed line still counts, per the other fixture tests
        .stderr(predicate::str::contains("ingested 4"));
}

#[test]
fn ingest_dir_tracks_changes_via_supersedes() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    std::fs::write(tree.join("src/a.rs"), "fn a() {}").unwrap();
    std::fs::write(tree.join("b.md"), "# b").unwrap();

    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 2, skipped 0 existing"));

    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 0, skipped 2 existing"));

    std::fs::write(tree.join("src/a.rs"), "fn a() { changed() }").unwrap();
    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 1, skipped 1 existing"));

    let ids = singularmem()
        .args([
            "--store", db_s, "list", "--tag", "ext:rs", "--format", "ids",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ids = String::from_utf8(ids).unwrap();
    let newest = ids.lines().last().unwrap().to_string();
    singularmem()
        .args(["--store", db_s, "revisions", &newest, "--format", "ids"])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 2));
}

#[test]
fn ingest_dir_read_only_store_is_exit_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args(["--store", db_s, "ingest", "--content", "seed"])
        .assert()
        .success();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("x.txt"), "x").unwrap();
    singularmem()
        .args([
            "--store",
            db_s,
            "--read-only",
            "ingest-dir",
            tree.to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("read-only"));
}

#[test]
fn ingest_dir_long_path_is_counted_not_fatal() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("ok.txt"), "fine").unwrap();

    // Nest 100-char directories until the leaf file's absolute path is
    // longer than the store's 512-byte external_id cap.
    let mut deep = tree.clone();
    while deep.join("f.txt").as_os_str().len() <= 520 {
        deep = deep.join("a".repeat(100));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("f.txt"), "too deep").unwrap();

    singularmem()
        .args(["--store", db_s, "ingest-dir", tree.to_str().unwrap()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ingested 1"))
        .stderr(predicate::str::contains("1 failed"));

    singularmem()
        .args(["--store", db_s, "list", "--format", "ids"])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 1));
}

#[cfg(unix)]
#[test]
fn ingest_transcript_project_filter_matches_symlinked_cwd() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();

    // A directory reachable by two names. On macOS the TempDir already sits
    // under /var -> /private/var; elsewhere make the symlink explicitly.
    let raw = dir.path().to_path_buf();
    let canon = raw.canonicalize().unwrap();
    let (raw, _canon) = if canon == raw {
        let target = raw.join("real");
        std::fs::create_dir_all(&target).unwrap();
        let link = raw.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        (link, target)
    } else {
        (raw, canon)
    };

    let tx = dir.path().join("s.jsonl");
    let line = serde_json::json!({
        "type": "user",
        "uuid": "u",
        "sessionId": "s",
        "cwd": raw.display().to_string(),
        "message": {"role": "user", "content": "symlinked project message"},
    });
    std::fs::write(&tx, format!("{line}\n")).unwrap();

    // The CLI canonicalises --project; the transcript's cwd is the
    // non-canonical spelling of the same directory.
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-transcript",
            tx.to_str().unwrap(),
            "--project",
            raw.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 1"));
}

#[test]
fn ingest_dir_missing_path_is_exit_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest-dir",
            "/definitely/missing",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("path not found"));
}

/// Run `search --scope <scope> cargo --json` and return the `hits[].id`
/// values from the JSON output.
fn scoped_search_hit_ids(db_s: &str, scope: &str) -> Vec<String> {
    let args = [
        "--store", db_s, "search", "--scope", scope, "cargo", "--json",
    ];
    let out = singularmem()
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("\"id\""), "expected JSON hits: {out}");
    let parsed: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
    parsed["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["id"].as_str().expect("id string").to_string())
        .collect()
}

#[test]
fn bulk_verbs_apply_default_scopes_and_filters_work() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let tree = dir.path().join("myrepo");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("notes.md"), "cargo notes in a file").unwrap();

    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-transcript",
            fixture_transcripts().to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .code(1);
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-dir",
            tree.to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .success();

    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("claude-code/other\t1\nclaude-code/proj\t3\nfiles/myrepo\t1\n");

    singularmem()
        .args([
            "--store", db_s, "list", "--scope", "files", "--format", "ids",
        ])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 1));
    singularmem()
        .args([
            "--store",
            db_s,
            "list",
            "--scope",
            "files",
            "--scope-exact",
            "--format",
            "ids",
        ])
        .assert()
        .success()
        .stdout("");

    let claude_code_ids = scoped_search_hit_ids(db_s, "claude-code");
    assert!(!claude_code_ids.is_empty());

    // Descendant-style filter (no --scope-exact) also finds the file hit.
    let files_ids = scoped_search_hit_ids(db_s, "files");
    assert!(!files_ids.is_empty());

    // The two scoped searches must return disjoint sets of ids.
    assert!(
        claude_code_ids.iter().all(|id| !files_ids.contains(id)),
        "expected disjoint ids: claude-code={claude_code_ids:?} files={files_ids:?}"
    );

    singularmem()
        .args([
            "--store",
            db_s,
            "search",
            "--scope",
            "files",
            "--scope-exact",
            "cargo",
        ])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn scope_flag_validation_and_move() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let id = String::from_utf8(
        singularmem()
            .args([
                "--store",
                db_s,
                "ingest",
                "--content",
                "x",
                "--scope",
                "Team/One",
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
        .args(["--store", db_s, "get", &id, "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"scope\":\"team/one\""));
    singularmem()
        .args(["--store", db_s, "list", "--scope-exact"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--scope-exact requires --scope"));
    singularmem()
        .args(["--store", db_s, "list", "--scope", "a//b"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("scope"));

    singularmem()
        .args(["--store", db_s, "scope", "move", &id, "team/two"])
        .assert()
        .success()
        .stderr(predicate::str::contains("singularmem reindex"));
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("team/two\t1\n");
    singularmem()
        .args(["--store", db_s, "scope", "move", &id, "-"])
        .assert()
        .success();
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn stale_tantivy_sidecar_exits_2_and_reindex_recovers() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args(["--store", db_s, "ingest", "--content", "zebra"])
        .assert()
        .success();
    // Replace the sidecar with one built on the previous schema.
    let sidecar = dir.path().join("store.db.tantivy");
    std::fs::remove_dir_all(&sidecar).unwrap();
    {
        use tantivy::schema::{SchemaBuilder, FAST, INDEXED, STORED, STRING, TEXT};
        let mut b = SchemaBuilder::new();
        b.add_text_field("content", TEXT | STORED);
        b.add_text_field("tags", STRING | STORED);
        b.add_text_field("source", TEXT | STORED);
        b.add_text_field("id", STRING | STORED);
        b.add_date_field("created_at", INDEXED | STORED | FAST);
        b.add_text_field("supersedes", STRING | STORED);
        std::fs::create_dir_all(&sidecar).unwrap();
        let mmap = tantivy::directory::MmapDirectory::open(&sidecar).unwrap();
        tantivy::Index::open_or_create(mmap, b.build()).unwrap();
    }
    singularmem()
        .args(["--store", db_s, "search", "zebra"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("singularmem reindex"));
    singularmem()
        .args(["--store", db_s, "reindex", "--quiet"])
        .assert()
        .success();
    singularmem()
        .args(["--store", db_s, "search", "zebra"])
        .assert()
        .success()
        .stdout(predicate::str::contains("zebra"));
}

fn fixture_codex() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex")
}

#[test]
fn ingest_codex_is_idempotent_and_scoped() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-codex",
            fixture_codex().to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "ingested 2, skipped 0 existing, 2 filtered, 1 failed across 1 files",
        ));
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-codex",
            fixture_codex().to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ingested 0, skipped 2 existing"));
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("codex/proj\t2\n");
}

#[test]
fn ingest_cursor_reads_a_fixture_user_dir() {
    use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("User");
    write_fixture(
        &user,
        &[FixtureWorkspace {
            hash: "h1",
            folder: Some("/w/proj"),
            composers: vec![(
                "c1",
                "T",
                1_700_000_000_000,
                vec![
                    FixtureBubble {
                        id: "b1",
                        kind: 1,
                        text: "hello cursor",
                    },
                    FixtureBubble {
                        id: "b2",
                        kind: 2,
                        text: "hi",
                    },
                ],
            )],
        }],
    );
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-cursor",
            "--cursor-dir",
            user.to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "ingested 2, skipped 0 existing, 0 filtered, 0 failed across 1 files",
        ));
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-cursor",
            "--cursor-dir",
            user.to_str().unwrap(),
            "--conversation",
            "c1",
            "--quiet",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("ingested 0, skipped 2 existing"));
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("cursor/proj\t2\n");
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest-cursor",
            "--cursor-dir",
            dir.path().join("nope").to_str().unwrap(),
        ])
        .assert()
        .code(2);

    // Without --cursor-dir, SINGULARMEM_CURSOR_DIR supplies the default —
    // the same override the `hook` verb honours (docs/hooks.md).
    let db2 = dir.path().join("store2.db");
    singularmem()
        .env("SINGULARMEM_CURSOR_DIR", user.to_str().unwrap())
        .args(["--store", db2.to_str().unwrap(), "ingest-cursor", "--quiet"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "ingested 2, skipped 0 existing, 0 filtered, 0 failed across 1 files",
        ));
    // An explicit --cursor-dir still wins over the environment variable.
    singularmem()
        .env("SINGULARMEM_CURSOR_DIR", user.to_str().unwrap())
        .args([
            "--store",
            db2.to_str().unwrap(),
            "ingest-cursor",
            "--cursor-dir",
            dir.path().join("nope").to_str().unwrap(),
        ])
        .assert()
        .code(2);
}

/// The Codex hook's fallback scan: no `transcript_path` in the payload, so
/// it searches `SINGULARMEM_CODEX_ROOT` for a rollout whose filename
/// carries the payload's `session_id`.
#[test]
fn hook_codex_stop_scans_the_env_root_for_the_session() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .env("SINGULARMEM_CODEX_ROOT", fixture_codex().to_str().unwrap())
        .args(["--store", db_s, "hook", "codex", "stop"])
        .write_stdin(r#"{"session_id":"sess1","cwd":"/home/me/proj"}"#)
        .assert()
        .success();
    // The fixture rollout holds one user and one assistant message.
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("codex/proj\t2\n");
    singularmem()
        .args(["--store", db_s, "list", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("How do I run the tests?"))
        .stdout(predicate::str::contains("Run cargo test."));
}

/// Build a store with items across `claude-code/proj`, `codex/proj`,
/// `files/proj`, and `claude-code/other`, for the `wake-up` tests below.
fn wake_up_test_store() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    for (c, sc) in [
        ("first note", "claude-code/proj"),
        ("second note", "codex/proj"),
        ("a file", "files/proj"),
        ("elsewhere", "claude-code/other"),
    ] {
        singularmem()
            .args(["--store", db_s, "ingest", "--content", c, "--scope", sc])
            .assert()
            .success();
    }
    (dir, db)
}

#[test]
fn wake_up_project_dot_resolves_via_canonicalize() {
    // `Path::new(".").file_name()` is `None`, so before the fix
    // `ScopeSet::for_project` silently returned an empty scope set for
    // `--project .` — a plausible, common invocation (run from the project
    // root). `--project .` must canonicalise to the real directory and
    // derive its basename, same as passing the absolute path would.
    let dir = TempDir::new().unwrap();
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest",
            "--content",
            "note",
            "--scope",
            "claude-code/proj",
        ])
        .assert()
        .success();

    let out = singularmem()
        .current_dir(&proj)
        .args(["--store", db_s, "wake-up", "--project", "."])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with(
        "# Singularmem wake-up — claude-code/proj, codex/proj, cursor/proj — 1 items, showing last 1\n"
    ));
}

/// `--project` names the directory the editor actually opened, even when
/// that path is a symlink to a differently-named target — e.g. Claude
/// Code's `current` symlink under a Codex-style session layout. Scoping must
/// key off the link's own basename (`current`), not the resolved target's
/// (`real-name`), since that is the name the transcript was ingested under.
#[test]
#[cfg(unix)]
fn wake_up_project_symlink_uses_link_name() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real-name");
    std::fs::create_dir_all(&real).unwrap();
    let link = dir.path().join("current");
    symlink(&real, &link).unwrap();

    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args([
            "--store",
            db_s,
            "ingest",
            "--content",
            "note via symlink",
            "--scope",
            "claude-code/current",
        ])
        .assert()
        .success();

    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--project",
            link.to_str().unwrap(),
            "--limit",
            "1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let header = text.lines().next().unwrap();
    assert!(header.contains("claude-code/current"), "{header}");
    assert!(header.contains("1 items"), "{header}");
}

#[test]
fn wake_up_text_format_defaults_to_project_editor_scopes() {
    let (_dir, db) = wake_up_test_store();
    let db_s = db.to_str().unwrap();

    let out = singularmem()
        .args(["--store", db_s, "wake-up", "--project", "/x/proj"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with(
        "# Singularmem wake-up — claude-code/proj, codex/proj, cursor/proj — 2 items, showing last 2\n"
    ));
    assert!(text.contains("first note") && text.contains("second note"));
    assert!(!text.contains("a file") && !text.contains("elsewhere"));

    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--project",
            "/x/proj",
            "--include-files",
            "--limit",
            "1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("3 items, showing last 1") && text.contains("a file"));
}

#[test]
fn wake_up_hook_and_json_formats() {
    let (_dir, db) = wake_up_test_store();
    let db_s = db.to_str().unwrap();

    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--scope",
            "claude-code/other",
            "--format",
            "claude-hook",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("elsewhere"));

    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--scope",
            "claude-code/other",
            "--format",
            "codex-hook",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert!(v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("elsewhere"));

    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--scope",
            "claude-code/other",
            "--format",
            "cursor-hook",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["additional_context"]
        .as_str()
        .unwrap()
        .contains("elsewhere"));

    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--scope",
            "claude-code/other",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["blocks"][0]["content"], "elsewhere");
}

#[test]
fn wake_up_empty_and_invalid_scope() {
    let (_dir, db) = wake_up_test_store();
    let db_s = db.to_str().unwrap();

    let out = singularmem()
        .args(["--store", db_s, "wake-up", "--scope", "nothing/here"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "# Singularmem wake-up — nothing/here — 0 items, showing last 0\n"
    );
    singularmem()
        .args(["--store", db_s, "wake-up", "--scope", "a//b"])
        .assert()
        .code(1);
}

#[test]
fn wake_up_repeatable_scope_dedups_overlapping_filters() {
    let (_dir, db) = wake_up_test_store();
    let db_s = db.to_str().unwrap();

    // Repeatable --scope with overlapping filters must not duplicate ids
    // (sub-project 13 task 5 follow-up: see wakeup::build's HashSet dedup).
    // This store's items don't tie on created_at, so this test doesn't
    // discriminate the old adjacent-`dedup_by` bug from the fix — that
    // coverage lives in singularmem-retrieve's
    // build_dedups_items_seen_via_overlapping_scope_filters (fixed clock,
    // forced tie). This test instead covers the end-to-end CLI wiring and
    // the union `total`.
    let out = singularmem()
        .args([
            "--store",
            db_s,
            "wake-up",
            "--scope",
            "claude-code",
            "--scope",
            "claude-code/other",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let ids: Vec<&str> = v["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "no duplicate ids: {ids:?}");
    assert!(!ids.is_empty());
    // `claude-code` matches both `claude-code/proj` and `claude-code/other`
    // (2 items); `claude-code/other` matches `claude-code/other` again (1
    // more). The union is 2 distinct items, not the 3 a naive per-filter sum
    // would give.
    assert_eq!(v["total"], 2);
}

#[test]
fn hook_claude_stop_ingests_transcript_and_session_start_prints_envelope() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let transcript = fixture_transcripts().join("proj/session.jsonl");
    let payload = serde_json::json!({"session_id":"s","transcript_path":transcript,"cwd":"/home/me/proj","hook_event_name":"Stop"}).to_string();
    singularmem()
        .args(["--store", db_s, "hook", "claude-code", "stop"])
        .write_stdin(payload.clone())
        .assert()
        .success()
        .stdout("");
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("claude-code/other\t1\nclaude-code/proj\t3\n");
    // Idempotent on the second stop; still exit 0 even though the fixture has a malformed line.
    singularmem()
        .args(["--store", db_s, "hook", "claude-code", "pre-compact"])
        .write_stdin(payload)
        .assert()
        .success();

    let start = serde_json::json!({"session_id":"s","cwd":"/home/me/proj","hook_event_name":"SessionStart","source":"startup"}).to_string();
    let out = singularmem()
        .args(["--store", db_s, "hook", "claude-code", "session-start"])
        .write_stdin(start)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.starts_with(
        "# Singularmem wake-up — claude-code/proj, codex/proj, cursor/proj — 3 items"
    ));
    assert!(ctx.contains("Run cargo test."));
}

/// The save side scopes items by the basename of the **raw** `cwd` the
/// editor reported. When that cwd is a symlink (`<tmp>/current ->
/// <tmp>/real-name`), a `session-start` that canonicalised first would look
/// under `claude-code/real-name` and find nothing. Stop then session-start
/// with the same cwd must round-trip.
#[test]
#[cfg(unix)]
fn hook_round_trips_a_symlinked_project_dir() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real-name");
    std::fs::create_dir_all(&real).unwrap();
    let link = dir.path().join("current");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let link_s = link.to_str().unwrap();

    // A one-line transcript whose `cwd` is the symlink path.
    let transcript = dir.path().join("session.jsonl");
    let user_line = serde_json::json!({
        "type": "user",
        "uuid": "u1",
        "parentUuid": serde_json::Value::Null,
        "sessionId": "11111111-2222-3333-4444-555555555555",
        "timestamp": "2026-09-01T10:00:00.000Z",
        "cwd": link_s,
        "isSidechain": false,
        "message": {"role": "user", "content": "symlinked project question"},
    });
    std::fs::write(&transcript, format!("{user_line}\n")).unwrap();

    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let payload = serde_json::json!({
        "session_id": "s",
        "transcript_path": transcript,
        "cwd": link_s,
        "hook_event_name": "Stop",
    })
    .to_string();
    singularmem()
        .args(["--store", db_s, "hook", "claude-code", "stop"])
        .write_stdin(payload)
        .assert()
        .success();
    singularmem()
        .args(["--store", db_s, "scope", "list"])
        .assert()
        .success()
        .stdout("claude-code/current\t1\n");

    let start = serde_json::json!({
        "session_id": "s",
        "cwd": link_s,
        "hook_event_name": "SessionStart",
        "source": "startup",
    })
    .to_string();
    let out = singularmem()
        .args(["--store", db_s, "hook", "claude-code", "session-start"])
        .write_stdin(start)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        ctx.contains("claude-code/current"),
        "wake-up must use the raw basename: {ctx}"
    );
    assert!(
        ctx.contains("1 items"),
        "wake-up must find the item saved under the symlink's basename: {ctx}"
    );
    assert!(ctx.contains("symlinked project question"));
}

/// Cursor can report several workspace roots for one window; `session-start`
/// unions every root's scopes (spec: "Cursor multi-root union").
#[test]
fn hook_cursor_session_start_unions_workspace_roots() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    for (c, sc) in [("alpha note", "cursor/alpha"), ("beta note", "cursor/beta")] {
        singularmem()
            .args(["--store", db_s, "ingest", "--content", c, "--scope", sc])
            .assert()
            .success();
    }

    let out = singularmem()
        .args(["--store", db_s, "hook", "cursor", "session-start"])
        .write_stdin(r#"{"workspace_roots":["/w/alpha","/w/beta"],"session_id":"q"}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let ctx = v["additional_context"].as_str().unwrap();
    assert!(ctx.contains("cursor/alpha"), "{ctx}");
    assert!(ctx.contains("cursor/beta"), "{ctx}");
    assert!(ctx.contains("2 items"), "{ctx}");
    assert!(
        ctx.contains("alpha note") && ctx.contains("beta note"),
        "{ctx}"
    );
}

#[test]
fn hook_never_fails_the_editor() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .args(["--store", db_s, "hook", "claude-code", "stop"])
        .write_stdin("{not json")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("hook input"));
    singularmem()
        .args(["--store", db_s, "hook", "claude-code", "stop"])
        .write_stdin(r#"{"cwd":"/x"}"#)
        .assert()
        .success()
        .stderr(predicate::str::contains("transcript_path"));
    singularmem()
        .args(["--store", db_s, "hook", "codex", "stop"])
        .write_stdin(
            r#"{"session_id":"k","transcript_path":"/definitely/missing.jsonl","cwd":"/x"}"#,
        )
        .assert()
        .success();
    let out = singularmem()
        .args(["--store", db_s, "hook", "cursor", "session-start"])
        .write_stdin(r#"{"workspace_roots":["/x/proj"],"session_id":"q"}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["additional_context"]
        .as_str()
        .unwrap()
        .contains("0 items"));
}

#[test]
fn hook_cursor_stop_ingests_only_that_conversation() {
    use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("User");
    write_fixture(
        &user,
        &[FixtureWorkspace {
            hash: "h1",
            folder: Some("/w/proj"),
            composers: vec![
                (
                    "c1",
                    "A",
                    1_700_000_000_000,
                    vec![FixtureBubble {
                        id: "b1",
                        kind: 1,
                        text: "one",
                    }],
                ),
                (
                    "c2",
                    "B",
                    1_700_000_001_000,
                    vec![FixtureBubble {
                        id: "b2",
                        kind: 1,
                        text: "two",
                    }],
                ),
            ],
        }],
    );
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .env("SINGULARMEM_CURSOR_DIR", user.to_str().unwrap())
        .args(["--store", db_s, "hook", "cursor", "stop"])
        .write_stdin(r#"{"conversation_id":"c2","workspace_roots":["/w/proj"]}"#)
        .assert()
        .success();
    singularmem()
        .args(["--store", db_s, "list", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("two"))
        .stdout(predicate::str::contains("one").not());
}

/// A payload with `conversation_id` but no `cwd` falls back to the first
/// `workspace_roots` entry as `cwd` (see `singularmem_hooks::input::parse_input`).
/// When that entry names the wrong workspace, the `cwd`-filtered scan finds
/// nothing at all — not even an existing item skipped as a duplicate — and
/// the hook must retry once across every workspace rather than losing the
/// transcript.
#[test]
fn hook_cursor_stop_retries_across_workspaces_when_cwd_guess_misses() {
    use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("User");
    write_fixture(
        &user,
        &[FixtureWorkspace {
            hash: "h1",
            folder: Some("/w/other"),
            composers: vec![(
                "c1",
                "A",
                1_700_000_000_000,
                vec![FixtureBubble {
                    id: "b1",
                    kind: 1,
                    text: "found me anyway",
                }],
            )],
        }],
    );
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .env("SINGULARMEM_CURSOR_DIR", user.to_str().unwrap())
        .args(["--store", db_s, "hook", "cursor", "stop"])
        .write_stdin(r#"{"conversation_id":"c1","workspace_roots":["/w/nomatch"]}"#)
        .assert()
        .success();
    singularmem()
        .args(["--store", db_s, "list", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("found me anyway"));
}

#[test]
fn hooks_install_status_uninstall_round_trip() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let original = "{\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(ls)\"\n    ]\n  },\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"echo other\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";
    std::fs::write(&settings, original).unwrap();
    let h = home.path().to_str().unwrap();

    singularmem()
        .env("HOME", h)
        .args(["hooks", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code\tabsent"));
    singularmem()
        .env("HOME", h)
        .args(["hooks", "install", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains(settings.to_str().unwrap()));
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(after["permissions"]["allow"][0], "Bash(ls)");
    assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert!(after["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("hook claude-code session-start"));
    singularmem()
        .env("HOME", h)
        .args(["hooks", "install", "claude-code"])
        .assert()
        .success();
    let again = std::fs::read_to_string(&settings).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&again).unwrap(),
        after,
        "idempotent"
    );
    singularmem()
        .env("HOME", h)
        .args(["hooks", "status", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code\tinstalled"))
        .stdout(predicate::str::contains("bin ok"));
    let printed = singularmem()
        .env("HOME", h)
        .args(["hooks", "install", "codex", "--print"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&printed).unwrap();
    assert!(v["hooks"]["SessionStart"].is_array());
    assert!(
        !home.path().join(".codex/hooks.json").exists(),
        "--print writes nothing"
    );
    singularmem()
        .env("HOME", h)
        .args(["hooks", "uninstall", "claude-code"])
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        original,
        "foreign entries byte-identical"
    );
    std::fs::write(&settings, "{ not json").unwrap();
    singularmem()
        .env("HOME", h)
        .args(["hooks", "install", "claude-code"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("settings.json"));
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{ not json",
        "never overwrites invalid JSON"
    );
}

#[test]
fn hook_exits_zero_when_store_cannot_be_opened() {
    let dir = TempDir::new().unwrap();
    // A directory, not a valid SQLite file: `Store::open_with_options` fails
    // to open it no matter which command tries to use it.
    let bad = dir.path().join("not-a-store");
    std::fs::create_dir_all(&bad).unwrap();
    let bad_s = bad.to_str().unwrap();

    singularmem()
        .args(["--store", bad_s, "hook", "claude-code", "stop"])
        .write_stdin(r#"{"transcript_path":"/does/not/matter.jsonl"}"#)
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains(bad_s));

    let out = singularmem()
        .args(["--store", bad_s, "hook", "claude-code", "session-start"])
        .write_stdin(r#"{"cwd":"/x/proj"}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap(),
        ""
    );
}

#[test]
fn hook_codex_stop_without_session_id_ingests_nothing() {
    let dir = TempDir::new().unwrap();
    let codex_root = dir.path().join("codex_root");
    std::fs::create_dir_all(&codex_root).unwrap();
    let rollout = fixture_codex().join("2026/09/01/rollout-2026-09-01T10-00-00-sess1.jsonl");
    std::fs::copy(
        &rollout,
        codex_root.join("rollout-2026-09-01T10-00-00-sess1.jsonl"),
    )
    .unwrap();

    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .env("SINGULARMEM_CODEX_ROOT", codex_root.to_str().unwrap())
        .args(["--store", db_s, "hook", "codex", "stop"])
        .write_stdin(r#"{"cwd":"/x"}"#)
        .assert()
        .success();

    singularmem()
        .args(["--store", db_s, "list", "--format", "ids"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hook_cursor_stop_without_conversation_or_cwd_ingests_nothing() {
    use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("User");
    write_fixture(
        &user,
        &[FixtureWorkspace {
            hash: "h1",
            folder: Some("/w/proj"),
            composers: vec![(
                "c1",
                "A",
                1_700_000_000_000,
                vec![FixtureBubble {
                    id: "b1",
                    kind: 1,
                    text: "one",
                }],
            )],
        }],
    );
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem()
        .env("SINGULARMEM_CURSOR_DIR", user.to_str().unwrap())
        .args(["--store", db_s, "hook", "cursor", "stop"])
        .write_stdin("{}")
        .assert()
        .success();
    singularmem()
        .args(["--store", db_s, "list", "--format", "ids"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hooks_status_reports_invalid_for_unparsable_config() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, "{ not json").unwrap();
    let h = home.path().to_str().unwrap();

    singularmem()
        .env("HOME", h)
        .args(["hooks", "status", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code\tinvalid"))
        .stderr(predicate::str::contains("settings.json"));
}

#[test]
fn hooks_uninstall_does_not_rewrite_foreign_only_config() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    // 4-space indentation — different from `write_config`'s 2-space output,
    // so a spurious rewrite would be detectable even if the content were
    // otherwise unchanged.
    let original = "{\n    \"permissions\": {\n        \"allow\": [\n            \"Bash(ls)\"\n        ]\n    }\n}\n";
    std::fs::write(&settings, original).unwrap();
    let h = home.path().to_str().unwrap();

    singularmem()
        .env("HOME", h)
        .args(["hooks", "uninstall", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains(settings.to_str().unwrap()));

    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        original,
        "foreign-only config left byte-identical"
    );
}

#[test]
fn hooks_status_project_flag_reads_project_config() {
    let project = TempDir::new().unwrap();
    let project_dir = project.path().to_str().unwrap();
    let settings = project.path().join(".claude/settings.json");

    singularmem()
        .current_dir(project_dir)
        .args(["hooks", "status", "claude-code", "--project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code\tabsent"))
        .stdout(predicate::str::contains(settings.to_str().unwrap()));

    singularmem()
        .current_dir(project_dir)
        .args(["hooks", "install", "claude-code", "--project"])
        .assert()
        .success();

    singularmem()
        .current_dir(project_dir)
        .args(["hooks", "status", "claude-code", "--project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code\tinstalled"));
}

#[test]
fn store_env_var_is_honoured_and_flag_wins() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.db");
    let b = dir.path().join("b.db");

    singularmem()
        .env("SINGULARMEM_STORE", a.to_str().unwrap())
        .args(["ingest", "--content", "via env"])
        .assert()
        .success();
    singularmem()
        .env("SINGULARMEM_STORE", a.to_str().unwrap())
        .args(["list", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("via env"));

    singularmem()
        .env("SINGULARMEM_STORE", a.to_str().unwrap())
        .args([
            "--store",
            b.to_str().unwrap(),
            "ingest",
            "--content",
            "via flag",
        ])
        .assert()
        .success();
    singularmem()
        .env("SINGULARMEM_STORE", a.to_str().unwrap())
        .args(["--store", b.to_str().unwrap(), "list", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("via flag"))
        .stdout(predicate::str::contains("via env").not());
}

/// Run `singularmem --store db_s <args>`.
fn graph_cmd(db: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut full = vec!["--store", db];
    full.extend_from_slice(args);
    singularmem().args(full).assert()
}

/// Run and return trimmed stdout, asserting success.
fn graph_stdout(db: &str, args: &[&str]) -> String {
    String::from_utf8(graph_cmd(db, args).success().get_output().stdout.clone())
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn graph_add_query_supersede_history_and_axes() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let item = graph_stdout(db_s, &["ingest", "--content", "we picked tantivy"]);

    let fact = graph_stdout(
        db_s,
        &[
            "graph",
            "add",
            "Singularmem",
            "uses",
            "Tantivy",
            "--source",
            &item,
            "--scope",
            "claude-code/singularmem",
            "--from",
            "2026-05-16",
        ],
    );
    assert_eq!(fact.len(), 26);
    // Same triple, same window: idempotent, down to the id.
    graph_cmd(
        db_s,
        &[
            "graph",
            "add",
            "singularmem",
            "uses",
            "tantivy",
            "--scope",
            "claude-code/singularmem",
            "--from",
            "2026-05-16",
        ],
    )
    .success()
    .stdout(format!("{fact}\n"));

    graph_cmd(db_s, &["graph", "query", "singularmem", "--with-sources"])
        .success()
        .stdout(predicate::str::contains("Singularmem —uses→ Tantivy"))
        .stdout(predicate::str::contains("open"))
        .stdout(predicate::str::contains("we picked tantivy"));

    graph_cmd(
        db_s,
        &[
            "graph",
            "supersede",
            "singularmem",
            "uses",
            "tantivy",
            "meilisearch",
            "--at",
            "2026-09-01",
            "--scope",
            "claude-code/singularmem",
        ],
    )
    .success();
    graph_cmd(
        db_s,
        &["graph", "query", "singularmem", "--as-of", "2026-08-01"],
    )
    .success()
    .stdout(predicate::str::contains("Tantivy"))
    .stdout(predicate::str::contains("meilisearch").not());
    graph_cmd(
        db_s,
        &["graph", "query", "singularmem", "--as-of", "2026-09-02"],
    )
    .success()
    .stdout(predicate::str::contains("meilisearch"))
    .stdout(predicate::str::contains("Tantivy").not());
    graph_cmd(
        db_s,
        &[
            "graph",
            "query",
            "singularmem",
            "--recorded-at",
            "2020-01-01",
        ],
    )
    .success()
    .stdout("");

    let hist = graph_stdout(db_s, &["graph", "history", &fact, "--format", "ids"]);
    assert_eq!(hist.lines().count(), 2);
    let tl = graph_stdout(db_s, &["graph", "timeline", "singularmem", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&tl).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    graph_cmd(db_s, &["graph", "stats"])
        .success()
        .stdout(predicate::str::contains("open facts: 1"))
        .stdout(predicate::str::contains("closed facts: 1"));
    graph_cmd(db_s, &["graph", "entities", "--json"])
        .success()
        .stdout(predicate::str::contains("\"fact_count\""));
}

#[test]
fn graph_errors_and_read_only() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "add", "a", "bad-pred", "b"])
        .code(1)
        .stderr(predicate::str::contains("predicate"));
    graph_cmd(db_s, &["graph", "invalidate", "a", "p", "b"])
        .code(2)
        .stderr(predicate::str::contains("no open fact"));
    graph_cmd(
        db_s,
        &["graph", "add", "a", "p", "b", "--confidence", "1.5"],
    )
    .code(1);
    graph_cmd(db_s, &["graph", "add", "a", "p", "b"]).success();
    graph_cmd(db_s, &["--read-only", "graph", "add", "a", "p", "c"])
        .code(2)
        .stderr(predicate::str::contains("read-only"));
    graph_cmd(
        db_s,
        &["graph", "add", "a", "p", "Rust 1.80", "--value", "--json"],
    )
    .success()
    .stdout(predicate::str::contains("\"value\":\"Rust 1.80\""));
}

/// Every timestamp flag (`--from`/`--to`/`--at`/`--as-of`/`--recorded-at`)
/// must name itself in a bad-value error, not just say "validation failed
/// for timestamp" — otherwise a script with two time flags cannot tell
/// which one it got wrong.
#[test]
fn graph_timestamp_errors_name_the_flag() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "add", "a", "p", "b"]).success();

    graph_cmd(db_s, &["graph", "add", "x", "p", "y", "--from", "notadate"])
        .code(1)
        .stderr(predicate::str::contains("--from"));
    graph_cmd(db_s, &["graph", "add", "x", "p", "y", "--to", "notadate"])
        .code(1)
        .stderr(predicate::str::contains("--to"));
    graph_cmd(
        db_s,
        &["graph", "invalidate", "a", "p", "b", "--at", "notadate"],
    )
    .code(1)
    .stderr(predicate::str::contains("--at"));
    graph_cmd(
        db_s,
        &["graph", "supersede", "a", "p", "b", "c", "--at", "notadate"],
    )
    .code(1)
    .stderr(predicate::str::contains("--at"));
    graph_cmd(db_s, &["graph", "query", "a", "--as-of", "notadate"])
        .code(1)
        .stderr(predicate::str::contains("--as-of"));
    graph_cmd(db_s, &["graph", "query", "a", "--recorded-at", "notadate"])
        .code(1)
        .stderr(predicate::str::contains("--recorded-at"));
}

/// `graph predicate`, human and `--json`, plus `--as-of` filtering.
#[test]
fn graph_predicate_human_json_and_as_of() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(
        db_s,
        &[
            "graph",
            "add",
            "alice",
            "knows",
            "bob",
            "--from",
            "2026-01-01",
        ],
    )
    .success();
    graph_cmd(db_s, &["graph", "add", "carol", "knows", "alice"]).success();

    graph_cmd(db_s, &["graph", "predicate", "knows"])
        .success()
        .stdout(predicate::str::contains("alice —knows→ bob"))
        .stdout(predicate::str::contains("carol —knows→ alice"));

    let json = graph_stdout(db_s, &["graph", "predicate", "knows", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);

    // alice —knows→ bob only starts being valid on 2026-01-01; as-of a year
    // earlier must exclude it but keep the window-less carol fact.
    graph_cmd(
        db_s,
        &["graph", "predicate", "knows", "--as-of", "2025-01-01"],
    )
    .success()
    .stdout(predicate::str::contains("carol —knows→ alice"))
    .stdout(predicate::str::contains("alice —knows→ bob").not());
}

/// `graph invalidate`'s success path: exit 0, the fact stops showing up in
/// `graph query`, and `graph history --json` grows to two revisions.
#[test]
fn graph_invalidate_closes_fact_and_extends_history() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fact = graph_stdout(db_s, &["graph", "add", "singularmem", "uses", "sqlite"]);

    graph_cmd(db_s, &["graph", "query", "singularmem"])
        .success()
        .stdout(predicate::str::contains("sqlite"));

    graph_cmd(
        db_s,
        &["graph", "invalidate", "singularmem", "uses", "sqlite"],
    )
    .success()
    .stdout(predicate::str::contains("singularmem —uses→ sqlite"));

    graph_cmd(db_s, &["graph", "query", "singularmem"])
        .success()
        .stdout("");

    let json = graph_stdout(db_s, &["graph", "history", &fact, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
}

/// `graph query --direction outgoing|incoming|both` with one incoming and
/// one outgoing fact seeded on the queried entity.
#[test]
fn graph_query_direction_filters_by_side() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "add", "alice", "knows", "bob"]).success();
    graph_cmd(db_s, &["graph", "add", "carol", "knows", "alice"]).success();

    graph_cmd(
        db_s,
        &["graph", "query", "alice", "--direction", "outgoing"],
    )
    .success()
    .stdout(predicate::str::contains("alice —knows→ bob"))
    .stdout(predicate::str::contains("carol —knows→ alice").not());

    graph_cmd(
        db_s,
        &["graph", "query", "alice", "--direction", "incoming"],
    )
    .success()
    .stdout(predicate::str::contains("carol —knows→ alice"))
    .stdout(predicate::str::contains("alice —knows→ bob").not());

    graph_cmd(db_s, &["graph", "query", "alice", "--direction", "both"])
        .success()
        .stdout(predicate::str::contains("alice —knows→ bob"))
        .stdout(predicate::str::contains("carol —knows→ alice"));
}

/// `--scope-exact` on `query`: a fact recorded on a descendant scope is
/// included under the plain (subtree) filter but excluded once `--scope`'s
/// exact match is requested.
#[test]
fn graph_query_scope_exact_excludes_descendant_scope() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(
        db_s,
        &[
            "graph",
            "add",
            "singularmem",
            "uses",
            "tantivy",
            "--scope",
            "claude-code/singularmem",
        ],
    )
    .success();
    graph_cmd(
        db_s,
        &[
            "graph",
            "add",
            "singularmem",
            "uses",
            "meilisearch",
            "--scope",
            "claude-code/singularmem/search",
        ],
    )
    .success();

    graph_cmd(
        db_s,
        &[
            "graph",
            "query",
            "singularmem",
            "--scope",
            "claude-code/singularmem",
        ],
    )
    .success()
    .stdout(predicate::str::contains("tantivy"))
    .stdout(predicate::str::contains("meilisearch"));

    graph_cmd(
        db_s,
        &[
            "graph",
            "query",
            "singularmem",
            "--scope",
            "claude-code/singularmem",
            "--scope-exact",
        ],
    )
    .success()
    .stdout(predicate::str::contains("tantivy"))
    .stdout(predicate::str::contains("meilisearch").not());
}

/// `graph entities --kind` restricts to entities created with that kind.
#[test]
fn graph_entities_kind_filter() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(
        db_s,
        &[
            "graph",
            "add",
            "dave",
            "works_at",
            "acme",
            "--object-kind",
            "company",
        ],
    )
    .success();
    graph_cmd(db_s, &["graph", "add", "dave", "knows", "erin"]).success();

    let out = graph_stdout(db_s, &["graph", "entities", "--kind", "company"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 1);
    let fields: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(fields.len(), 4);
    assert_eq!(fields[1], "acme");
    assert_eq!(fields[2], "company");
    assert_eq!(fields[3], "1");
}

/// `graph history --format table` and `--format ids`.
#[test]
fn graph_history_format_table_and_ids() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fact = graph_stdout(db_s, &["graph", "add", "a", "p", "b"]);

    graph_cmd(db_s, &["graph", "history", &fact, "--format", "table"])
        .success()
        .stdout(predicate::str::contains("a —p→ b"));

    let ids = graph_stdout(db_s, &["graph", "history", &fact, "--format", "ids"]);
    assert_eq!(ids, fact);
}

/// `graph history --json` is a shortcut for `--format json` (spec's literal
/// `graph history <FACT_ID> [--json]`), and conflicts with an explicit
/// `--format ids`.
#[test]
fn graph_history_json_flag_matches_format_json_and_conflicts_with_format() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fact = graph_stdout(db_s, &["graph", "add", "a", "p", "b"]);

    let via_json_flag = graph_stdout(db_s, &["graph", "history", &fact, "--json"]);
    let via_format_json = graph_stdout(db_s, &["graph", "history", &fact, "--format", "json"]);
    assert_eq!(via_json_flag, via_format_json);
    let v: serde_json::Value = serde_json::from_str(&via_json_flag).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);

    graph_cmd(
        db_s,
        &["graph", "history", &fact, "--json", "--format", "ids"],
    )
    .code(2)
    .stderr(predicate::str::contains("cannot be used with"));
}

/// `graph history NOTANID` names the fact ID specifically, distinct from
/// `--source`'s "invalid item ID" — both parse a ULID, but a caller needs
/// to know which argument was rejected.
#[test]
fn graph_history_invalid_fact_id_is_a_distinct_error() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "history", "NOTANID"])
        .code(1)
        .stderr(predicate::str::contains("invalid fact ID"))
        .stderr(predicate::str::contains("invalid item ID").not());
}

/// A second `graph add` for a standing triple is idempotent only when the
/// validity window matches; a differing one is refused rather than dropped.
#[test]
fn graph_add_refuses_a_divergent_window_for_a_standing_fact() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let fact = graph_stdout(
        db_s,
        &["graph", "add", "a", "p", "b", "--from", "2026-05-16"],
    );

    graph_cmd(
        db_s,
        &["graph", "add", "a", "p", "b", "--from", "2026-05-16"],
    )
    .success()
    .stdout(format!("{fact}\n"));

    graph_cmd(
        db_s,
        &["graph", "add", "a", "p", "b", "--from", "2026-06-01"],
    )
    .code(1)
    .stderr(predicate::str::contains("a —p→ b"))
    .stderr(predicate::str::contains("use supersede or invalidate"));
    graph_cmd(
        db_s,
        &["graph", "add", "a", "p", "b", "--confidence", "0.5"],
    )
    .code(1)
    .stderr(predicate::str::contains("use supersede or invalidate"));

    graph_cmd(db_s, &["graph", "stats"])
        .success()
        .stdout(predicate::str::contains("open facts: 1"));
}

/// `graph timeline`'s human output marks the open head `[current]` and every
/// other revision `[closed]`, ordered by `valid_from` with "valid since
/// unknown" (`NULL`) first.
#[test]
fn graph_timeline_human_output_marks_current_and_closed() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "add", "singularmem", "uses", "tantivy"]).success();
    graph_cmd(
        db_s,
        &[
            "graph",
            "supersede",
            "singularmem",
            "uses",
            "tantivy",
            "meilisearch",
        ],
    )
    .success();

    let out = graph_stdout(db_s, &["graph", "timeline", "singularmem"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("[closed] "), "{out}");
    assert!(lines[0].contains("tantivy"), "{out}");
    assert!(lines[1].starts_with("[current] "), "{out}");
    assert!(lines[1].contains("meilisearch"), "{out}");
}

/// `graph supersede`'s human output: the closed fact's line, then the new
/// fact's line.
#[test]
fn graph_supersede_prints_closed_then_new_fact_lines() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "add", "a", "p", "old"]).success();

    let out = graph_stdout(db_s, &["graph", "supersede", "a", "p", "old", "new"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("a —p→ old"));
    assert!(lines[1].contains("a —p→ new"));
}

/// `graph supersede` with no prior open fact: only the new fact is printed
/// (id included via `render_fact`), and the library's warning about the
/// missing old fact reaches stderr.
#[test]
fn graph_supersede_warns_when_no_prior_open_fact() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();

    let out = graph_stdout(db_s, &["graph", "supersede", "a", "p", "old", "new"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("a —p→ new"));

    graph_cmd(db_s, &["graph", "supersede", "x", "p", "old", "new2"])
        .success()
        .stderr(predicate::str::contains("no open fact to close"));
}

/// `graph invalidate`'s human output: exactly one line, the closed fact
/// (its `valid_to` is no longer `open`).
#[test]
fn graph_invalidate_prints_the_closed_fact_line() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    graph_cmd(db_s, &["graph", "add", "a", "p", "b"]).success();

    let out = graph_stdout(db_s, &["graph", "invalidate", "a", "p", "b"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 1);
    // The closed fact — `valid_to` is a real timestamp now, not the
    // open-ended "open".
    assert!(lines[0].contains("a —p→ b"));
    assert!(!lines[0].contains("open"));
}
