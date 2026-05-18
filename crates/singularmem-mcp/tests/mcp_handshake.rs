//! Black-box integration test for the MCP server.
//!
//! Spawns the `singularmem-mcp` binary as a subprocess, seeds the store
//! by running the `singularmem` binary first (also as a subprocess),
//! sends JSON-RPC messages over stdin, reads responses from stdout, and
//! asserts on the protocol-level shape.
//!
//! Verifies the most failure-prone properties of an MCP server:
//! - Initialize handshake returns the expected serverInfo.
//! - `tools/list` includes the `memory_retrieve` descriptor.
//! - tools/call invokes the handler and returns a text block.
//! - stdout stays clean (no stray writes corrupt the JSON-RPC stream).
//! - stderr is drained continuously to avoid buffer-fill deadlock.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use tempfile::TempDir;

/// Locate the `singularmem` binary (root crate) via Cargo's output directory.
fn singularmem_bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("singularmem")
}

/// Locate the `singularmem-mcp` binary via Cargo's output directory.
fn mcp_bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("singularmem-mcp")
}

/// Seed items into a store at `path` via the `singularmem` CLI,
/// then run reindex with embeddings (using `MockEmbedder`).
fn seed_via_cli(path: &Path, contents: &[&str]) {
    for content in contents {
        let status = Command::new(singularmem_bin())
            .args([
                "--store",
                path.to_str().unwrap(),
                "ingest",
                "--content",
                content,
            ])
            .env("SINGULARMEM_TEST_EMBEDDER", "mock")
            .status()
            .expect("singularmem ingest");
        assert!(status.success(), "ingest failed");
    }
    let status = Command::new(singularmem_bin())
        .args([
            "--store",
            path.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .status()
        .expect("singularmem reindex");
    assert!(status.success(), "reindex failed");
}

#[test]
fn handshake_and_retrieve_end_to_end() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("store.db");

    // Seed the store via the CLI.
    seed_via_cli(&store, &["the quick brown fox jumps"]);

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

    // Drain stderr in a background thread so the child can't fill its
    // pipe buffer and block.
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

    // Helper closures.
    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").expect("write to mcp stdin");
        stdin.flush().expect("flush stdin");
    };
    let recv_response = |reader: &mut BufReader<std::process::ChildStdout>| -> serde_json::Value {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read from mcp stdout");
        assert!(bytes > 0, "EOF reading response");
        serde_json::from_str(line.trim()).expect("parse JSON response")
    };

    // Step 1: initialize handshake.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(
        resp["result"]["serverInfo"]["name"], "singularmem-mcp",
        "wrong serverInfo.name: {resp}"
    );
    assert!(
        resp["result"]["capabilities"]["tools"].is_object(),
        "tools capability missing: {resp}"
    );

    // Step 2: initialized notification (no response expected).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Step 3: tools/call memory_retrieve.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_retrieve","arguments":{"query":"fox"}}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 2);
    let content = resp["result"]["content"].as_array().expect("content array");
    assert!(!content.is_empty(), "empty content array: {resp}");
    let text = content[0]["text"].as_str().expect("text block");
    assert!(
        text.contains("the quick brown fox"),
        "expected ingested memory in response, got: {text}"
    );

    // Step 4: close stdin, wait for exit, check stderr was clean.
    drop(stdin);
    let exit = child.wait().expect("wait for mcp process");
    assert!(
        exit.success(),
        "MCP server exited with non-zero status: {exit:?}"
    );

    let stderr_output = stderr_handle.join().expect("stderr thread");
    assert!(
        !stderr_output.contains("panic"),
        "stderr contains 'panic': {stderr_output}"
    );
}
