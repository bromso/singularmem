use std::path::Path;

use serde_json::json;
use singularmem_hooks::{entries, merge, remove, status, Editor, MARKER};

const BIN: &str = "/opt/bin/singularmem";

#[test]
fn claude_entries_match_spec() {
    let e = entries(Editor::ClaudeCode, Path::new(BIN));
    let hooks = &e["hooks"];
    assert_eq!(
        hooks["SessionStart"][0]["matcher"],
        "startup|resume|clear|compact"
    );
    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        format!("\"{BIN}\" hook claude-code session-start")
    );
    assert_eq!(hooks["SessionStart"][0]["hooks"][0]["timeout"], 30);
    assert_eq!(hooks["Stop"][0]["hooks"][0]["async"], true);
    assert_eq!(hooks["PreCompact"][0]["hooks"][0]["timeout"], 60);
    assert_eq!(hooks["SessionEnd"][0]["hooks"][0]["timeout"], 60);
    assert!(hooks["Stop"][0].get("matcher").is_none());
}

#[test]
fn codex_and_cursor_entries_match_spec() {
    let c = entries(Editor::Codex, Path::new(BIN));
    assert_eq!(c["hooks"]["SessionStart"][0]["matcher"], "*");
    assert_eq!(c["hooks"]["Stop"][0]["hooks"][0]["async"], true);
    assert!(c["hooks"].get("SessionEnd").is_none());
    let u = entries(Editor::Cursor, Path::new(BIN));
    assert_eq!(u["version"], 1);
    assert_eq!(u["hooks"]["stop"][0]["loop_limit"], 1);
    assert_eq!(
        u["hooks"]["sessionStart"][0]["command"],
        format!("\"{BIN}\" hook cursor session-start")
    );
    assert_eq!(u["hooks"]["sessionStart"][0]["timeout"], 30);
}

#[test]
fn merge_preserves_foreign_entries_and_is_idempotent() {
    let existing = json!({
        "permissions": {"allow": ["Bash(ls)"]},
        "hooks": {
            "Stop": [{"hooks": [{"type": "command", "command": "echo other"}]}],
            "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "lint"}]}]
        }
    });
    let once = merge(Editor::ClaudeCode, &existing, Path::new(BIN));
    assert_eq!(once["permissions"]["allow"][0], "Bash(ls)");
    assert_eq!(
        once["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "lint"
    );
    let stop = once["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert_eq!(stop[0]["hooks"][0]["command"], "echo other");
    assert!(stop[1]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains(MARKER));
    let twice = merge(Editor::ClaudeCode, &once, Path::new(BIN));
    assert_eq!(twice, once);
    // Re-installing with a different binary path replaces ours, not theirs.
    let moved = merge(Editor::ClaudeCode, &once, Path::new("/new/singularmem"));
    let stop = moved["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert!(stop[1]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .starts_with("\"/new/singularmem\""));
}

#[test]
fn remove_leaves_foreign_entries_byte_identical() {
    let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo other"}]}]}, "x": 1});
    let merged = merge(Editor::ClaudeCode, &existing, Path::new(BIN));
    let removed = remove(Editor::ClaudeCode, &merged);
    assert_eq!(removed, existing);
    // Removing from a config that never had ours is a no-op.
    assert_eq!(remove(Editor::ClaudeCode, &existing), existing);
    // Cursor: empty event arrays are dropped; `version` stays.
    let cur = merge(Editor::Cursor, &json!({}), Path::new(BIN));
    let gone = remove(Editor::Cursor, &cur);
    assert_eq!(gone, json!({"version": 1, "hooks": {}}));
}

#[test]
fn status_detects_ours_and_bin_existence() {
    let none = status(Editor::Codex, &json!({}));
    assert!(!none.installed);
    let d = tempfile::TempDir::new().unwrap();
    let bin = d.path().join("singularmem");
    std::fs::write(&bin, "").unwrap();
    let merged = merge(Editor::Codex, &json!({}), &bin);
    let s = status(Editor::Codex, &merged);
    assert!(s.installed && s.bin_exists);
    assert_eq!(s.bin.as_deref(), Some(bin.as_path()));
    let stale = merge(Editor::Codex, &json!({}), Path::new("/nope/singularmem"));
    assert!(!status(Editor::Codex, &stale).bin_exists);
}
