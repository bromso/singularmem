use std::path::{Path, PathBuf};

use singularmem_core::NewItem;
use singularmem_ingest::{discover_transcripts, ClaudeTranscript, Source};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session.jsonl")
}

fn parse(t: &ClaudeTranscript) -> (Vec<NewItem>, Vec<singularmem_ingest::Error>) {
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for r in t.items() {
        match r {
            Ok(i) => ok.push(i),
            Err(e) => errs.push(e),
        }
    }
    (ok, errs)
}

#[test]
fn keeps_exactly_the_text_messages() {
    let t = ClaudeTranscript::open(fixture()).unwrap();
    let (items, errs) = parse(&t);
    let ids: Vec<&str> = items
        .iter()
        .map(|i| i.external_id.as_deref().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "claude-code:11111111-2222-3333-4444-555555555555:u1",
            "claude-code:11111111-2222-3333-4444-555555555555:a1",
            "claude-code:11111111-2222-3333-4444-555555555555:u3",
            "claude-code:11111111-2222-3333-4444-555555555555:a4",
        ]
    );
    assert_eq!(errs.len(), 1, "the malformed line");
    assert!(
        matches!(&errs[0], singularmem_ingest::Error::Json { line: 15, .. }),
        "expected Json error on line 15, got {:?}",
        errs[0]
    );
    // filtered: tool_result-only u2, tool_use-only a2, thinking-only a3, meta u4, sidechain u5
    assert_eq!(t.filtered_count(), 5);
}

#[test]
fn item_shape_is_as_specified() {
    let t = ClaudeTranscript::open(fixture()).unwrap();
    let (items, _) = parse(&t);
    let a1 = &items[1];
    assert_eq!(a1.content, "Run cargo test.");
    assert_eq!(
        a1.source.as_deref(),
        Some("claude-code:11111111-2222-3333-4444-555555555555")
    );
    assert_eq!(a1.tags, vec!["claude-code", "role:assistant", "transcript"]);
    let m = &a1.metadata;
    assert_eq!(m["session_id"], "11111111-2222-3333-4444-555555555555");
    assert_eq!(m["uuid"], "a1");
    assert_eq!(m["parent_uuid"], "u1");
    assert_eq!(m["role"], "assistant");
    assert_eq!(m["cwd"], "/home/me/proj");
    assert_eq!(m["git_branch"], "main");
    assert_eq!(m["occurred_at"], "2026-09-01T10:00:01Z");
    assert_eq!(m["tool_names"], serde_json::json!(["Bash"]));
    assert_eq!(m["chunk_index"], 0);
    assert_eq!(m["chunk_count"], 1);

    let u3 = &items[2];
    assert_eq!(
        u3.content, "Thanks, that worked.",
        "system-reminder stripped"
    );
    assert_eq!(u3.metadata["parent_uuid"], "a3");
}

#[test]
fn sidechains_opt_in() {
    let mut t = ClaudeTranscript::open(fixture()).unwrap();
    t.include_sidechains = true;
    let (items, _) = parse(&t);
    let u5 = items
        .iter()
        .find(|i| i.metadata["uuid"] == "u5")
        .expect("sidechain kept");
    assert_eq!(
        u5.tags,
        vec!["claude-code", "role:user", "sidechain", "transcript"]
    );
}

#[test]
fn project_filter_matches_cwd() {
    let mut t = ClaudeTranscript::open(fixture()).unwrap();
    t.project_filter = Some(PathBuf::from("/home/me/proj"));
    let (items, _) = parse(&t);
    assert!(items.iter().all(|i| i.metadata["cwd"] == "/home/me/proj"));
    assert_eq!(items.len(), 3);
}

#[test]
fn long_messages_are_chunked_with_suffixed_ids() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("big.jsonl");
    // A single ~5000-byte paragraph (no blank line) so it hard-splits into
    // exactly 2 pieces under the real DEFAULT_CHUNK_BYTES (4096). Two
    // separate ~5000-byte paragraphs would each exceed 4096 individually
    // and hard-split into 2 pieces apiece (4 total), not 2.
    let content = "word ".repeat(1000); // ~5000 bytes
    let line = serde_json::json!({
        "type":"user","uuid":"big","sessionId":"s","timestamp":"2026-09-01T00:00:00Z",
        "message":{"role":"user","content":content}
    });
    std::fs::write(&p, format!("{line}\n")).unwrap();
    let t = ClaudeTranscript::open(&p).unwrap();
    let (items, _) = parse(&t);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].external_id.as_deref(), Some("claude-code:s:big#0"));
    assert_eq!(items[1].external_id.as_deref(), Some("claude-code:s:big#1"));
    assert_eq!(items[1].metadata["chunk_count"], 2);
}

#[test]
fn session_id_falls_back_to_file_stem() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("abc-123.jsonl");
    std::fs::write(
        &p,
        "{\"type\":\"user\",\"uuid\":\"u\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
    )
    .unwrap();
    let t = ClaudeTranscript::open(&p).unwrap();
    let (items, _) = parse(&t);
    assert_eq!(
        items[0].external_id.as_deref(),
        Some("claude-code:abc-123:u")
    );
}

#[test]
fn message_without_content_counts_as_filtered() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().join("bodyless.jsonl");
    std::fs::write(
        &p,
        concat!(
            "{\"type\":\"user\",\"uuid\":\"u\",\"sessionId\":\"s\"}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a\",\"sessionId\":\"s\",\"message\":{\"role\":\"assistant\"}}\n",
        ),
    )
    .unwrap();
    let t = ClaudeTranscript::open(&p).unwrap();
    let (items, errs) = parse(&t);
    assert!(items.is_empty());
    assert!(errs.is_empty());
    assert_eq!(t.filtered_count(), 2);
}

#[test]
fn strip_system_reminders_cases() {
    use singularmem_ingest::strip_system_reminders;

    assert_eq!(
        strip_system_reminders("a<system-reminder>x</system-reminder>b"),
        "ab"
    );
    assert_eq!(
        strip_system_reminders(
            "a<system-reminder>x</system-reminder><system-reminder>y</system-reminder>b"
        ),
        "ab"
    );
    assert_eq!(
        strip_system_reminders("a<system-reminder>unterminated"),
        "a"
    );
    assert_eq!(strip_system_reminders("  plain  "), "plain");
}

#[test]
fn discover_finds_jsonl_recursively_sorted() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("b")).unwrap();
    std::fs::write(dir.path().join("b/2.jsonl"), "").unwrap();
    std::fs::write(dir.path().join("1.jsonl"), "").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "").unwrap();
    let found = discover_transcripts(dir.path()).unwrap();
    assert_eq!(
        found,
        vec![dir.path().join("1.jsonl"), dir.path().join("b/2.jsonl")]
    );
}

#[test]
fn open_missing_path_is_not_found() {
    assert!(matches!(
        ClaudeTranscript::open("/definitely/missing.jsonl"),
        Err(singularmem_ingest::Error::NotFound { .. })
    ));
}

/// Two paths naming one directory: a non-canonical one and its canonical
/// form. On macOS a `TempDir` already lives under the `/var` → `/private/var`
/// symlink; elsewhere we make the symlink explicitly.
#[cfg(unix)]
fn two_paths_to_one_dir(d: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    let raw = d.path().to_path_buf();
    let canon = raw.canonicalize().unwrap();
    if canon != raw {
        return (raw, canon);
    }
    let target = raw.join("real");
    std::fs::create_dir_all(&target).unwrap();
    let link = raw.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    (link, target)
}

#[cfg(unix)]
fn one_line_transcript(path: &Path, cwd: &Path) {
    let line = serde_json::json!({
        "type": "user",
        "uuid": "u",
        "sessionId": "s",
        "cwd": cwd.display().to_string(),
        "message": {"role": "user", "content": "hello from the symlinked tree"},
    });
    std::fs::write(path, format!("{line}\n")).unwrap();
}

#[cfg(unix)]
#[test]
fn project_filter_matches_across_symlinks_in_both_directions() {
    let d = tempfile::TempDir::new().unwrap();
    let (raw, canon) = two_paths_to_one_dir(&d);
    assert_ne!(raw, canon, "fixture must offer two distinct paths");

    // cwd non-canonical, filter canonical (the CLI's shape: it
    // canonicalises --project before handing it to the parser).
    let a = d.path().join("a.jsonl");
    one_line_transcript(&a, &raw);
    let mut t = ClaudeTranscript::open(&a).unwrap();
    t.project_filter = Some(canon.clone());
    let (items, _) = parse(&t);
    assert_eq!(items.len(), 1, "canonical filter must match symlinked cwd");

    // cwd canonical, filter non-canonical.
    let b = d.path().join("b.jsonl");
    one_line_transcript(&b, &canon);
    let mut t = ClaudeTranscript::open(&b).unwrap();
    t.project_filter = Some(raw);
    let (items, _) = parse(&t);
    assert_eq!(items.len(), 1, "symlinked filter must match canonical cwd");
}

#[cfg(unix)]
#[test]
fn project_filter_still_rejects_a_different_directory() {
    let d = tempfile::TempDir::new().unwrap();
    let (raw, _canon) = two_paths_to_one_dir(&d);
    let a = d.path().join("a.jsonl");
    one_line_transcript(&a, &raw);
    let mut t = ClaudeTranscript::open(&a).unwrap();
    t.project_filter = Some(PathBuf::from("/definitely/elsewhere"));
    let (items, _) = parse(&t);
    assert!(items.is_empty());
}
