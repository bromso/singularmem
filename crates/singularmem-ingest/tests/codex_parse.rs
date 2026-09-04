use std::path::{Path, PathBuf};

use singularmem_ingest::{default_codex_root, discover_codex_sessions, CodexRollout, Source};

fn fx(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn keeps_only_user_and_assistant_messages() {
    let src = CodexRollout::open(fx("codex-rollout.jsonl")).unwrap();
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for r in src.items() {
        match r {
            Ok(i) => ok.push(i),
            Err(e) => errs.push(e),
        }
    }
    assert_eq!(errs.len(), 1);
    assert!(matches!(
        errs[0],
        singularmem_ingest::Error::Json { line: 8, .. }
    ));
    let ids: Vec<&str> = ok
        .iter()
        .map(|i| i.external_id.as_deref().unwrap())
        .collect();
    assert_eq!(ids, vec!["codex:sess-1:3", "codex:sess-1:6"]);
    assert_eq!(ok[1].content, "Run cargo test.\n\nIt takes a minute.");
    assert_eq!(ok[1].tags, vec!["codex", "role:assistant", "transcript"]);
    assert_eq!(ok[1].source.as_deref(), Some("codex:sess-1"));
    assert_eq!(ok[1].metadata["cwd"], "/home/me/proj");
    assert_eq!(ok[1].metadata["occurred_at"], "2026-09-01T10:00:05Z");
    assert_eq!(ok[1].metadata["line"], 6);
    assert_eq!(src.default_scope(&ok[0]).as_deref(), Some("codex/proj"));
    // function_call, function_call_output count as filtered; turn_context/event_msg are structural.
    assert_eq!(src.filtered_count(), 2);
    assert!(src.session_meta_seen());
}

#[test]
fn legacy_file_without_session_meta_uses_stem_and_no_scope() {
    let src = CodexRollout::open(fx("codex-legacy.jsonl")).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    assert_eq!(
        items[0].external_id.as_deref(),
        Some("codex:codex-legacy:1")
    );
    assert_eq!(items[0].metadata["cwd"], serde_json::Value::Null);
    assert_eq!(src.default_scope(&items[0]), None);
}

#[test]
fn project_filter_and_override() {
    let mut src = CodexRollout::open(fx("codex-rollout.jsonl")).unwrap();
    src.project_filter = Some(PathBuf::from("/home/me/other"));
    assert_eq!(src.items().filter_map(Result::ok).count(), 0);
    src.project_filter = Some(PathBuf::from("/home/me/proj"));
    assert_eq!(src.items().filter_map(Result::ok).count(), 2);
    src.scope_override = Some("Team/X".into());
    let first = src.items().find_map(Result::ok).unwrap();
    assert_eq!(src.default_scope(&first).as_deref(), Some("team/x"));
}

#[test]
fn accepts_plain_string_content() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("rollout-string-content.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"timestamp":"2026-09-01T10:00:00Z","type":"session_meta","payload":{"id":"sess-str","cwd":"/home/me/proj"}}"#,
            "\n",
            r#"{"timestamp":"2026-09-01T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":"plain string content"}}"#,
            "\n",
        ),
    )
    .unwrap();
    let src = CodexRollout::open(&path).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].content, "plain string content");
    assert_eq!(items[0].external_id.as_deref(), Some("codex:sess-str:2"));
}

#[test]
fn warns_once_when_first_parsed_line_is_blank_then_no_session_meta() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("rollout-blank-first.jsonl");
    std::fs::write(
        &path,
        concat!(
            "\n",
            r#"{"timestamp":"2026-09-01T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    let src = CodexRollout::open(&path).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].external_id.as_deref(),
        Some("codex:rollout-blank-first:2")
    );
    assert_eq!(items[0].metadata["cwd"], serde_json::Value::Null);
    // No session_meta line was ever parsed for this file.
    assert!(!src.session_meta_seen());
}

#[test]
fn discover_finds_rollout_files_only() {
    let d = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("2026/09/01")).unwrap();
    std::fs::write(
        d.path()
            .join("2026/09/01/rollout-2026-09-01T10-00-00-abc.jsonl"),
        "",
    )
    .unwrap();
    std::fs::write(d.path().join("2026/09/01/notes.jsonl"), "").unwrap();
    let found = discover_codex_sessions(d.path()).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0]
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("rollout-"));
}

/// On Windows `HOME` is usually unset and `USERPROFILE` is the home
/// directory; `default_codex_root` must fall back to it rather than
/// returning `None`.
#[test]
fn default_codex_root_falls_back_to_userprofile_when_home_is_unset() {
    temp_env::with_vars(
        [
            ("HOME", None::<&str>),
            ("USERPROFILE", Some("/Users/example")),
        ],
        || {
            let root = default_codex_root().unwrap();
            assert!(
                root.ends_with(".codex/sessions"),
                "expected root to end with .codex/sessions, got {}",
                root.display()
            );
        },
    );
}
