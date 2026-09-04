use std::path::PathBuf;

use serde_json::json;
use singularmem_hooks::{config_path, parse_input, session_start_envelope, Editor, Event};

#[test]
fn parses_each_editor_payload() {
    let c = parse_input(
        Editor::ClaudeCode,
        &json!({"session_id":"s","transcript_path":"/t.jsonl","cwd":"/w/p","hook_event_name":"Stop"}),
    );
    assert_eq!(c.cwd, Some(PathBuf::from("/w/p")));
    assert_eq!(c.transcript_path, Some(PathBuf::from("/t.jsonl")));
    assert_eq!(c.session_id.as_deref(), Some("s"));
    let k = parse_input(
        Editor::Codex,
        &json!({"session_id":"k","transcript_path":null,"cwd":"/w/p"}),
    );
    assert_eq!(k.transcript_path, None);
    assert_eq!(k.session_id.as_deref(), Some("k"));
    let u = parse_input(
        Editor::Cursor,
        &json!({"conversation_id":"c1","workspace_roots":["/w/p","/w/q"],"session_id":"x"}),
    );
    assert_eq!(u.conversation_id.as_deref(), Some("c1"));
    assert_eq!(
        u.workspace_roots,
        vec![PathBuf::from("/w/p"), PathBuf::from("/w/q")]
    );
    assert_eq!(
        u.cwd,
        Some(PathBuf::from("/w/p")),
        "first root doubles as cwd"
    );
    let empty = parse_input(Editor::ClaudeCode, &json!("not an object"));
    assert!(empty.cwd.is_none() && empty.transcript_path.is_none());
}

#[test]
fn envelopes() {
    assert_eq!(
        session_start_envelope(Editor::ClaudeCode, "hi"),
        json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"hi"}})
    );
    assert_eq!(
        session_start_envelope(Editor::Codex, "hi"),
        json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"hi"}})
    );
    assert_eq!(
        session_start_envelope(Editor::Cursor, "hi"),
        json!({"additional_context":"hi"})
    );
}

#[test]
fn parses_editor_and_event_names() {
    assert_eq!("claude-code".parse::<Editor>().unwrap(), Editor::ClaudeCode);
    assert_eq!("pre-compact".parse::<Event>().unwrap(), Event::PreCompact);
    assert!("vim".parse::<Editor>().is_err());
}

#[test]
fn config_paths_follow_env_and_project() {
    let d = tempfile::TempDir::new().unwrap();
    temp_env::with_vars(
        [
            ("HOME", Some(d.path().to_str().unwrap())),
            ("APPDATA", Some(d.path().to_str().unwrap())),
        ],
        || {
            assert_eq!(
                config_path(Editor::ClaudeCode, None).unwrap(),
                d.path().join(".claude/settings.json")
            );
            assert_eq!(
                config_path(Editor::Codex, None).unwrap(),
                d.path().join(".codex/hooks.json")
            );
            assert_eq!(
                config_path(Editor::Cursor, None).unwrap(),
                d.path().join(".cursor/hooks.json")
            );
            let p = PathBuf::from("/repo");
            assert_eq!(
                config_path(Editor::ClaudeCode, Some(&p)).unwrap(),
                PathBuf::from("/repo/.claude/settings.json")
            );
            assert_eq!(
                config_path(Editor::Codex, Some(&p)).unwrap(),
                PathBuf::from("/repo/.codex/hooks.json")
            );
            assert_eq!(
                config_path(Editor::Cursor, Some(&p)).unwrap(),
                PathBuf::from("/repo/.cursor/hooks.json")
            );
        },
    );
}

/// On Windows `HOME` is usually unset and `USERPROFILE` is the home
/// directory; `config_path` must fall back to it rather than failing.
#[test]
fn config_path_falls_back_to_userprofile_when_home_is_unset() {
    let d = tempfile::TempDir::new().unwrap();
    temp_env::with_vars(
        [
            ("HOME", None),
            ("USERPROFILE", Some(d.path().to_str().unwrap())),
        ],
        || {
            assert_eq!(
                config_path(Editor::ClaudeCode, None).unwrap(),
                d.path().join(".claude/settings.json")
            );
        },
    );
}

/// `HOME` wins when both are set.
#[test]
fn config_path_prefers_home_over_userprofile() {
    let home = tempfile::TempDir::new().unwrap();
    let profile = tempfile::TempDir::new().unwrap();
    temp_env::with_vars(
        [
            ("HOME", Some(home.path().to_str().unwrap())),
            ("USERPROFILE", Some(profile.path().to_str().unwrap())),
        ],
        || {
            assert_eq!(
                config_path(Editor::Codex, None).unwrap(),
                home.path().join(".codex/hooks.json")
            );
        },
    );
}

/// With neither variable set and no `--project` override there is no base
/// directory to join onto, so `config_path` reports `NoHome` rather than
/// guessing (or panicking).
#[test]
fn config_path_without_home_or_userprofile_is_no_home() {
    temp_env::with_vars([("HOME", None::<&str>), ("USERPROFILE", None)], || {
        for editor in [Editor::ClaudeCode, Editor::Codex, Editor::Cursor] {
            match config_path(editor, None) {
                Err(singularmem_hooks::Error::NoHome) => {}
                other => panic!("expected Error::NoHome for {editor}, got {other:?}"),
            }
        }
        // An explicit project override still works with no home at all.
        let p = PathBuf::from("/repo");
        assert_eq!(
            config_path(Editor::Cursor, Some(&p)).unwrap(),
            PathBuf::from("/repo/.cursor/hooks.json")
        );
    });
}
