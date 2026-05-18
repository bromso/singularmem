//! End-to-end integration test exercising the full ingest →
//! get → list → revisions → retrieve flow through MCP. Verifies
//! that `memory_ingest`'s auto-wiring populates the indexes so
//! `memory_retrieve` can find newly ingested memories without an
//! external `reindex` step.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

/// Path to the singularmem CLI binary (used to seed sidecar
/// directories via `reindex --with-embeddings` before the MCP
/// server starts — otherwise the first ingest creates only
/// `SQLite` rows, not Tantivy/USearch sidecars).
fn singularmem_bin() -> std::path::PathBuf {
    cargo_bin("singularmem")
}

fn mcp_bin() -> std::path::PathBuf {
    cargo_bin("singularmem-mcp")
}

/// Pre-create the .tantivy / .vectors sidecar directories by running
/// `singularmem reindex --with-embeddings` against an (empty) store.
/// This makes the MCP server's `memory_ingest` auto-wire the hooks on
/// its first invocation.
fn prime_sidecars(store: &Path) {
    let status = Command::new(singularmem_bin())
        .args([
            "--store",
            store.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .status()
        .expect("singularmem reindex");
    assert!(status.success(), "reindex failed");
}

#[test]
#[allow(clippy::too_many_lines)]
fn full_write_read_cycle_end_to_end() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("store.db");

    // Pre-create sidecars so the MCP server's first ingest auto-wires
    // the Tantivy + USearch hooks.
    prime_sidecars(&store);

    // Spawn the MCP server.
    let mut child = Command::new(mcp_bin())
        .env("SINGULARMEM_STORE", store.to_str().unwrap())
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn singularmem-mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let stderr_handle = thread::spawn(move || {
        let mut sink = String::new();
        let mut r = BufReader::new(stderr);
        loop {
            let mut line = String::new();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => sink.push_str(&line),
            }
        }
        sink
    });

    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    };
    let recv = |reader: &mut BufReader<std::process::ChildStdout>| -> serde_json::Value {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read");
        assert!(bytes > 0, "EOF");
        serde_json::from_str(line.trim()).expect("parse")
    };

    // Initialize.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let _ = recv(&mut reader);
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Step 1: Ingest first memory.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_ingest","arguments":{"content":"the quick brown fox jumps over the lazy dog","tags":["fox","animals"],"source":"test"}}}"#,
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(
        text.starts_with("Ingested memory "),
        "ingest response: {text}"
    );
    let first_id = text
        .strip_prefix("Ingested memory ")
        .expect("strip prefix")
        .split(' ')
        .next()
        .expect("split")
        .to_string();
    assert_eq!(first_id.len(), 26, "expected 26-char ULID: {first_id}");

    // Step 2: Get the first memory.
    let get_req = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"memory_get","arguments":{{"id":"{first_id}"}}}}}}"#
    );
    send(&mut stdin, &get_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("the quick brown fox"), "get response: {text}");

    // Step 3: Ingest second memory superseding the first.
    let supersedes_req = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"memory_ingest","arguments":{{"content":"revised fox content","supersedes":"{first_id}"}}}}}}"#
    );
    send(&mut stdin, &supersedes_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    let second_id = text
        .strip_prefix("Ingested memory ")
        .expect("strip prefix")
        .split(' ')
        .next()
        .expect("split")
        .to_string();
    assert_eq!(
        second_id.len(),
        26,
        "expected 26-char ULID for second_id: {second_id}"
    );

    // Step 4: Revisions chain should show 2 items.
    let rev_req = format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"memory_revisions","arguments":{{"id":"{second_id}"}}}}}}"#
    );
    send(&mut stdin, &rev_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(
        text.contains("2 items, newest first"),
        "revisions response: {text}"
    );

    // Step 5: List should show 2 items.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"memory_list","arguments":{}}}"#,
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("Found 2"), "list response: {text}");

    // Step 6: Retrieve should find the new memory (proving hook auto-wiring).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"memory_retrieve","arguments":{"query":"fox"}}}"#,
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("fox"), "retrieve response: {text}");

    // Cleanup.
    drop(stdin);
    let _ = child.wait();
    let stderr_text = stderr_handle.join().expect("stderr thread");
    assert!(
        !stderr_text.contains("panic"),
        "panic in stderr: {stderr_text}"
    );
}
