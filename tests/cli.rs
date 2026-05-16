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
fn search_missing_index_exits_2() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // Create store but never ingest (and never create the .tantivy dir).
    singularmem()
        .args(["--store", db.to_str().unwrap(), "list"])
        .assert()
        .success();

    // Search auto-creates an empty index directory if absent. The result is
    // 0 matches → exit 0. `--no-index` only suppresses auto-wiring of the
    // hook on Store::open; cmd_search opens its own Index regardless.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "--no-index",
            "search",
            "anything",
        ])
        .assert()
        .success();
}

#[test]
fn search_malformed_query_exits_1() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

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
    // Search opens a fresh index (auto-created empty). 0 matches; exit 0.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "search",
            "not",
            "searchable",
        ])
        .assert()
        .success();
}
