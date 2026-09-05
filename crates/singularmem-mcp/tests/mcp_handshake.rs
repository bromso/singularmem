//! Black-box integration test for the MCP server.
//!
//! Spawns the `singularmem-mcp` binary as a subprocess, seeds the store
//! by running the `singularmem` binary first (also as a subprocess),
//! sends JSON-RPC messages over stdin, reads responses from stdout, and
//! asserts on the protocol-level shape.
//!
//! Verifies the most failure-prone properties of an MCP server:
//! - Initialize handshake returns the expected serverInfo and advertises
//!   the `tools`, `prompts`, and `resources` capabilities.
//! - `tools/list` includes all 15 tool descriptors.
//! - tools/call invokes the handler and returns a text block.
//! - `prompts/list` returns the one `wake-up` prompt.
//! - `resources/templates/list` returns the `singularmem://memory/{id}`
//!   template; `resources/list` stays empty by design; `resources/read`
//!   round-trips a seeded item and reports `resource_not_found` for an
//!   unknown id.
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
#[allow(clippy::too_many_lines)]
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
    assert!(
        resp["result"]["capabilities"]["prompts"].is_object(),
        "prompts capability missing: {resp}"
    );
    assert!(
        resp["result"]["capabilities"]["resources"].is_object(),
        "resources capability missing: {resp}"
    );

    // Step 2: initialized notification (no response expected).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Step 3: tools/list — assert 5 tools are registered.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 2);
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(
        tools.len(),
        15,
        "expected 15 tools (6 original + 8 memory_graph_* + memory_wakeup), got: {tools:?}"
    );
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in &[
        "memory_retrieve",
        "memory_get",
        "memory_list",
        "memory_revisions",
        "memory_ingest",
        "memory_scopes",
        "memory_wakeup",
        "memory_graph_add",
        "memory_graph_query",
        "memory_graph_invalidate",
        "memory_graph_supersede",
        "memory_graph_timeline",
        "memory_graph_stats",
        "memory_graph_entities",
        "memory_graph_history",
    ] {
        assert!(
            names.contains(expected),
            "tool '{expected}' missing from list: {names:?}"
        );
    }

    // Step 5: tools/call memory_retrieve.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"memory_retrieve","arguments":{"query":"fox"}}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 3);
    let content = resp["result"]["content"].as_array().expect("content array");
    assert!(!content.is_empty(), "empty content array: {resp}");
    let text = content[0]["text"].as_str().expect("text block");
    assert!(
        text.contains("the quick brown fox"),
        "expected ingested memory in response, got: {text}"
    );

    // Step 5b: prompts/list — one prompt named `wake-up`.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":4,"method":"prompts/list"}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 4);
    let prompts = resp["result"]["prompts"].as_array().expect("prompts array");
    assert_eq!(prompts.len(), 1, "expected exactly one prompt: {prompts:?}");
    assert_eq!(prompts[0]["name"], "wake-up");

    // Step 5c: resources/templates/list — one template for singularmem://memory/{id}.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":5,"method":"resources/templates/list"}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 5);
    let templates = resp["result"]["resourceTemplates"]
        .as_array()
        .expect("resourceTemplates array");
    assert_eq!(
        templates.len(),
        1,
        "expected exactly one resource template: {templates:?}"
    );
    assert_eq!(
        templates[0]["uriTemplate"], "singularmem://memory/{id}",
        "wrong uriTemplate: {resp}"
    );

    // Step 5d: resources/list stays empty by design.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":6,"method":"resources/list"}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 6);
    let resources = resp["result"]["resources"]
        .as_array()
        .expect("resources array");
    assert!(
        resources.is_empty(),
        "expected resources/list to stay empty: {resources:?}"
    );

    // Step 5e: resources/read for the seeded item.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"memory_list","arguments":{}}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 7);
    let list_text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("memory_list text block");
    let item_line = list_text
        .lines()
        .find(|l| l.contains(": "))
        .unwrap_or_else(|| panic!("no item line in memory_list output: {list_text}"));
    let seeded_id = item_line
        .split(':')
        .next()
        .expect("id before ':'")
        .trim()
        .to_string();

    send(
        &mut stdin,
        &format!(
            r#"{{"jsonrpc":"2.0","id":8,"method":"resources/read","params":{{"uri":"singularmem://memory/{seeded_id}"}}}}"#
        ),
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 8);
    let contents = resp["result"]["contents"]
        .as_array()
        .expect("contents array");
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0]["mimeType"], "text/plain");
    let resource_text = contents[0]["text"].as_str().expect("resource text");
    assert!(
        resource_text.starts_with("id: "),
        "wrong resource text: {resource_text}"
    );
    assert!(
        resource_text.contains("the quick brown fox"),
        "wrong resource text: {resource_text}"
    );

    // Step 5f: resources/read for an unknown id is `resource_not_found`.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":9,"method":"resources/read","params":{"uri":"singularmem://memory/01ARZ3NDEKTSV4RRFFQ69G5FAV"}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 9);
    assert!(
        resp.get("error").is_some(),
        "expected an error for unknown resource id: {resp}"
    );

    // Step 6: close stdin, wait for exit, check stderr was clean.
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
