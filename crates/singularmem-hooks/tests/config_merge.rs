use std::path::Path;

use serde_json::json;
use singularmem_hooks::{entries, is_ours, merge, remove, status, Editor, MARKER};

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
fn is_ours_is_structural_not_a_substring_heuristic() {
    // The binary path need not mention "singularmem" at all.
    let bin = Path::new("/home/x/bin/sm");
    let merged = merge(Editor::ClaudeCode, &json!({}), bin);
    let cmd = merged["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(is_ours(cmd));
    let twice = merge(Editor::ClaudeCode, &merged, bin);
    assert_eq!(twice, merged);
    let removed = remove(Editor::ClaudeCode, &merged);
    assert_eq!(removed, json!({}));

    // A foreign command that merely mentions `singularmem` in an argument
    // and isn't in the quoted-`hook`-shape is not ours, and survives
    // `remove`.
    let foreign = json!({
        "hooks": {
            "Stop": [{"hooks": [{
                "type": "command",
                "command": "\"/usr/local/bin/backup\" hook --exclude ~/singularmem"
            }]}]
        }
    });
    assert!(!is_ours(
        foreign["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
    ));
    assert_eq!(remove(Editor::ClaudeCode, &foreign), foreign);

    // A command that mentions our binary and `hook <editor> <event>` but
    // doesn't start with the leading quote is not ours either.
    let unquoted = json!({
        "hooks": {
            "Stop": [{"hooks": [{
                "type": "command",
                "command": "env FOO=1 \"/opt/singularmem\" hook codex stop"
            }]}]
        }
    });
    assert!(!is_ours(
        unquoted["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
    ));
    assert_eq!(remove(Editor::Codex, &unquoted), unquoted);
}

#[test]
fn find_bin_ignores_foreign_commands_in_mixed_group() {
    let existing = json!({
        "hooks": {
            "Stop": [{
                "hooks": [
                    {"type": "command", "command": "echo unrelated"},
                    {"type": "command", "command": format!("\"{BIN}\" hook claude-code stop")}
                ]
            }]
        }
    });
    let s = status(Editor::ClaudeCode, &existing);
    assert!(s.installed);
    assert_eq!(s.bin.as_deref(), Some(Path::new(BIN)));
}

#[test]
fn remove_preserves_key_order() {
    let existing = json!({
        "model": "m",
        "hooks": {
            "Stop": [{"hooks": [{
                "type": "command",
                "command": format!("\"{BIN}\" hook claude-code stop")
            }]}]
        },
        "permissions": {}
    });
    let result = remove(Editor::ClaudeCode, &existing);
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"model":"m","permissions":{}}"#
    );
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

/// A config whose `hooks` key holds something other than an object (a
/// hand-edited file, or another tool's unrelated `hooks` setting) cannot be
/// merged into. `merge` replaces it with our object and warns, rather than
/// silently dropping our entries on the floor.
#[test]
fn merge_replaces_a_non_object_hooks_value() {
    for bogus in [json!([]), json!("off"), json!(3), json!(null), json!(true)] {
        let existing = json!({"keep": "me", "hooks": bogus});
        let merged = merge(Editor::ClaudeCode, &existing, Path::new(BIN));
        assert_eq!(merged["keep"], "me", "unrelated keys survive");
        assert!(
            merged["hooks"].is_object(),
            "hooks must be replaced with our object"
        );
        assert_eq!(
            merged["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            format!("\"{BIN}\" hook claude-code session-start")
        );
        assert!(status(Editor::ClaudeCode, &merged).installed);
    }
}
