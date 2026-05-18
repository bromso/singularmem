//! Verifies `--read-only` semantics:
//! - `tools/list` excludes `memory_ingest`.
//! - Direct `tools/call memory_ingest` is rejected with `InvalidParams`.
//! - Read tools (`memory_get`) still work.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

fn singularmem_bin() -> std::path::PathBuf {
    cargo_bin("singularmem")
}

fn mcp_bin() -> std::path::PathBuf {
    cargo_bin("singularmem-mcp")
}

/// Seed one memory via the CLI, then run reindex so the read-only MCP
/// server has something to read.
fn seed_via_cli(store: &Path) -> String {
    let output = Command::new(singularmem_bin())
        .args([
            "--store",
            store.to_str().unwrap(),
            "ingest",
            "--content",
            "seeded memory for read-only test",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .output()
        .expect("singularmem ingest");
    assert!(output.status.success(), "ingest failed");
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let id = stdout.trim().to_string();
    assert_eq!(id.len(), 26, "expected ULID: {id:?}");

    let status = Command::new(singularmem_bin())
        .args([
            "--store",
            store.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .status()
        .expect("reindex");
    assert!(status.success(), "reindex failed");
    id
}

#[test]
fn read_only_mode_excludes_ingest_and_rejects_direct_calls() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("store.db");
    let seeded_id = seed_via_cli(&store);

    // Spawn the MCP server with --read-only.
    let mut child = Command::new(mcp_bin())
        .args(["--read-only"])
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

    // tools/list should return 4 tools (no memory_ingest).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let resp = recv(&mut reader);
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(
        tools.len(),
        4,
        "expected 4 tools in read-only mode, got: {tools:?}"
    );
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        !names.contains(&"memory_ingest"),
        "memory_ingest should be omitted: {names:?}"
    );

    // tools/call memory_ingest should be rejected.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"memory_ingest","arguments":{"content":"should be rejected"}}}"#,
    );
    let resp = recv(&mut reader);
    let err = &resp["error"];
    assert!(err.is_object(), "expected error response: {resp}");
    let msg = err["message"].as_str().expect("error message");
    assert!(
        msg.contains("read-only"),
        "expected 'read-only' in error message: {msg}"
    );

    // tools/call memory_get should still work.
    let get_req = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"memory_get","arguments":{{"id":"{seeded_id}"}}}}}}"#
    );
    send(&mut stdin, &get_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("get response text");
    assert!(
        text.contains("seeded memory"),
        "expected seed content: {text}"
    );

    drop(stdin);
    let _ = child.wait();
    let stderr_text = stderr_handle.join().expect("stderr thread");
    assert!(
        !stderr_text.contains("panic"),
        "panic in stderr: {stderr_text}"
    );
}
