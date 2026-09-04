//! Building our hook config entries and merging/removing/inspecting them
//! inside an editor's config file, plus atomic read/write of that file.
//!
//! ## Detecting "ours"
//!
//! The command string we write is `"<bin>" hook <editor> <event>` — the
//! binary path is double-quoted. Detection is structural, not a substring
//! heuristic: [`parse_ours`] requires the command to start with `"`, have a
//! closing `"`, and have the remainder (trimmed) parse exactly as `hook
//! <editor> <event>` with a known [`Editor`] and [`Event`]. This can't be
//! fooled by a binary path that happens to contain `singularmem`, nor by an
//! unrelated command that merely mentions `singularmem` or `hook`
//! somewhere in its arguments.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::editor::{Editor, Event};
use crate::error::{Error, Result};

/// Substring present in every command we write.
///
/// This is the closing quote around the binary path, followed by ` hook `.
/// Kept public so callers can sanity check a command string directly;
/// prefer [`is_ours`] (or [`parse_ours`]) for classification.
pub const MARKER: &str = "\" hook ";

/// Parse `command` as one of ours, returning the binary path, editor, and
/// event it encodes when it matches.
///
/// A command is ours iff it starts with `"`, has a closing `"`, and the
/// trimmed remainder is exactly `hook <editor> <event>` where `<editor>`
/// and `<event>` parse as [`Editor`] and [`Event`] respectively.
#[must_use]
pub fn parse_ours(command: &str) -> Option<(PathBuf, Editor, Event)> {
    let rest = command.strip_prefix('"')?;
    let end = rest.find('"')?;
    let bin = PathBuf::from(&rest[..end]);
    let tail = rest[end + 1..].trim();

    let mut parts = tail.split(' ');
    if parts.next()? != "hook" {
        return None;
    }
    let editor: Editor = parts.next()?.parse().ok()?;
    let event: Event = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if tail != format!("hook {editor} {event}") {
        return None;
    }

    Some((bin, editor, event))
}

/// Whether `command` is one of ours.
///
/// See [`parse_ours`] for the structural check this delegates to.
#[must_use]
pub fn is_ours(command: &str) -> bool {
    parse_ours(command).is_some()
}

/// Result of inspecting an editor's config for our hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookStatus {
    /// Whether at least one of our hook entries is present.
    pub installed: bool,
    /// The binary path our installed entries point at, if installed.
    pub bin: Option<PathBuf>,
    /// Whether `bin` exists on disk. `false` when nothing is installed.
    pub bin_exists: bool,
}

/// The literal command string for one event, run through the given binary.
fn command(editor: Editor, event: Event, bin: &Path) -> String {
    format!("\"{}\" hook {editor} {event}", bin.display())
}

/// Build the full hooks object we install for `editor`, pointing every
/// event at `bin`. See the design spec's "Hook config entries" section for
/// the exact shape.
#[must_use]
pub fn entries(editor: Editor, bin: &Path) -> Value {
    match editor {
        Editor::ClaudeCode => json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup|resume|clear|compact",
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::SessionStart, bin),
                        "timeout": 30,
                    }],
                }],
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::Stop, bin),
                        "async": true,
                    }],
                }],
                "PreCompact": [{
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::PreCompact, bin),
                        "timeout": 60,
                    }],
                }],
                "SessionEnd": [{
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::SessionEnd, bin),
                        "timeout": 60,
                    }],
                }],
            }
        }),
        Editor::Codex => json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::SessionStart, bin),
                        "timeout": 30,
                    }],
                }],
                "Stop": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::Stop, bin),
                        "async": true,
                    }],
                }],
                "PreCompact": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": command(editor, Event::PreCompact, bin),
                        "timeout": 60,
                    }],
                }],
            }
        }),
        Editor::Cursor => json!({
            "version": 1,
            "hooks": {
                "sessionStart": [{
                    "command": command(editor, Event::SessionStart, bin),
                    "timeout": 30,
                }],
                "stop": [{
                    "command": command(editor, Event::Stop, bin),
                    "timeout": 60,
                    "loop_limit": 1,
                }],
                "preCompact": [{
                    "command": command(editor, Event::PreCompact, bin),
                    "timeout": 60,
                }],
                "sessionEnd": [{
                    "command": command(editor, Event::SessionEnd, bin),
                    "timeout": 60,
                }],
            }
        }),
    }
}

/// Whether a single hook-array element (a Claude/Codex group, or a Cursor
/// flat entry) is ours.
///
/// A Claude/Codex group is a list of `{"type": ..., "command": ...}`
/// entries; a group is treated as ours if *any* of its commands are ours,
/// and such a group is removed (or replaced) whole rather than having only
/// the matching commands stripped out of it. This is safe because the
/// installer never writes a mixed group: every group we produce contains
/// exactly one command, ours.
fn group_is_ours(editor: Editor, group: &Value) -> bool {
    match editor {
        Editor::Cursor => group
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_ours),
        Editor::ClaudeCode | Editor::Codex => group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks
                    .iter()
                    .filter_map(|h| h.get("command").and_then(Value::as_str))
                    .any(is_ours)
            }),
    }
}

/// Merge our hook entries for `editor` (pointing at `bin`) into `existing`,
/// replacing any of our previous entries and preserving everything else.
///
/// `existing` need not be an object; anything else is treated as `{}`.
/// Idempotent: merging the result again with the same `bin` is a no-op.
///
/// If `existing["hooks"]` is present but is **not** an object (a
/// hand-edited file, or an unrelated `hooks` setting written by something
/// else), it cannot be merged into: it is logged with `tracing::warn!` and
/// replaced by the hooks object we build. Every other key in `existing` is
/// preserved either way.
#[must_use]
pub fn merge(editor: Editor, existing: &Value, bin: &Path) -> Value {
    let mut result = existing.as_object().cloned().unwrap_or_default();
    if matches!(editor, Editor::Cursor) && !result.contains_key("version") {
        result.insert("version".to_string(), json!(1));
    }

    let ours = entries(editor, bin);
    let Some(ours_hooks) = ours.get("hooks").and_then(Value::as_object) else {
        return Value::Object(result);
    };

    let mut hooks_obj = match result.get("hooks") {
        Some(Value::Object(obj)) => obj.clone(),
        Some(other) => {
            tracing::warn!(
                found = %other,
                editor = %editor,
                "existing \"hooks\" value is not an object; replacing it"
            );
            serde_json::Map::new()
        }
        None => serde_json::Map::new(),
    };

    for (event_key, our_group) in ours_hooks {
        let our_arr = our_group.as_array().cloned().unwrap_or_default();
        let mut merged_arr: Vec<Value> = hooks_obj
            .get(event_key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|group| !group_is_ours(editor, group))
            .collect();
        merged_arr.extend(our_arr);
        hooks_obj.insert(event_key.clone(), Value::Array(merged_arr));
    }

    result.insert("hooks".to_string(), Value::Object(hooks_obj));
    Value::Object(result)
}

/// Remove our hook entries for `editor` from `existing`.
///
/// Everything else is left untouched. Event arrays that become empty are
/// dropped; for Claude Code and Codex the `hooks` object itself is dropped
/// once empty, but for Cursor `"hooks": {}` and `"version"` are kept. A
/// no-op when `existing` never had our entries.
#[must_use]
pub fn remove(editor: Editor, existing: &Value) -> Value {
    let Some(mut result) = existing.as_object().cloned() else {
        return existing.clone();
    };
    let Some(mut hooks_obj) = result.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(result);
    };

    // Mutate `hooks_obj` in place (rather than building a fresh map) and
    // use `shift_remove` for keys that go away entirely, so the relative
    // order of whatever event keys survive is left untouched.
    let event_keys: Vec<String> = hooks_obj.keys().cloned().collect();
    for event_key in event_keys {
        let Some(arr) = hooks_obj.get(&event_key).and_then(Value::as_array) else {
            continue;
        };
        let retained: Vec<Value> = arr
            .iter()
            .filter(|group| !group_is_ours(editor, group))
            .cloned()
            .collect();
        if retained.is_empty() {
            hooks_obj.shift_remove(&event_key);
        } else {
            hooks_obj.insert(event_key, Value::Array(retained));
        }
    }

    if matches!(editor, Editor::Cursor) || !hooks_obj.is_empty() {
        result.insert("hooks".to_string(), Value::Object(hooks_obj));
    } else {
        result.shift_remove("hooks");
    }

    Value::Object(result)
}

/// The commands carried by a single hook-array element (a Claude/Codex
/// group, or a Cursor flat entry).
fn group_commands(editor: Editor, group: &Value) -> Vec<&str> {
    match editor {
        Editor::Cursor => group
            .get("command")
            .and_then(Value::as_str)
            .into_iter()
            .collect(),
        Editor::ClaudeCode | Editor::Codex => group
            .get("hooks")
            .and_then(Value::as_array)
            .map(|hooks| {
                hooks
                    .iter()
                    .filter_map(|h| h.get("command").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Find the binary path recorded in the first of our commands in
/// `existing`, scanning every event array under `hooks`. Foreign commands
/// sharing a group with one of ours (see [`group_is_ours`]) are ignored:
/// only commands for which [`parse_ours`] succeeds are considered.
fn find_bin(editor: Editor, existing: &Value) -> Option<PathBuf> {
    let hooks_obj = existing.get("hooks")?.as_object()?;
    for arr in hooks_obj.values() {
        let Some(arr) = arr.as_array() else {
            continue;
        };
        for group in arr {
            if let Some((bin, ..)) = group_commands(editor, group)
                .into_iter()
                .find_map(parse_ours)
            {
                return Some(bin);
            }
        }
    }
    None
}

/// Whether any of our hook entries are present in `existing`.
fn has_ours(editor: Editor, existing: &Value) -> bool {
    existing
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(|hooks_obj| {
            hooks_obj.values().any(|arr| {
                arr.as_array()
                    .is_some_and(|arr| arr.iter().any(|group| group_is_ours(editor, group)))
            })
        })
}

/// Inspect `existing` for our hooks: whether installed, which binary path
/// they point at, and whether that binary still exists on disk.
///
/// `installed` reflects whether any of our entries are present at all,
/// independent of whether a binary path could be extracted from one of
/// them; `bin` is populated separately via [`parse_ours`] and may be
/// `None` even when `installed` is `true`.
#[must_use]
pub fn status(editor: Editor, existing: &Value) -> HookStatus {
    let installed = has_ours(editor, existing);
    let bin = find_bin(editor, existing);
    let bin_exists = bin.as_deref().is_some_and(Path::exists);
    HookStatus {
        installed,
        bin,
        bin_exists,
    }
}

/// Read a config file as JSON, returning `{}` when it does not exist.
///
/// # Errors
///
/// Returns [`Error::InvalidJson`] when the file exists but is not valid
/// JSON, and [`Error::Io`] for any other I/O failure.
pub fn read_config(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|source| Error::InvalidJson {
            path: path.to_path_buf(),
            source,
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Write `value` to `path` atomically: pretty-printed with a 2-space
/// indent and a trailing newline, written to `<path>.tmp` and renamed into
/// place. Parent directories are created as needed.
///
/// # Errors
///
/// Returns [`Error::Io`] if creating parent directories, writing the
/// temporary file, or renaming it fails, and [`Error::InvalidJson`] in the
/// (practically unreachable) case that `value` cannot be serialized.
pub fn write_config(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let mut text = serde_json::to_string_pretty(value).map_err(|source| Error::InvalidJson {
        path: path.to_path_buf(),
        source,
    })?;
    text.push('\n');

    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);

    let result = std::fs::write(&tmp_path, &text)
        .map_err(|source| Error::Io {
            path: tmp_path.clone(),
            source,
        })
        .and_then(|()| {
            std::fs::rename(&tmp_path, path).map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })
        });

    if result.is_err() {
        // Best-effort: don't leave a stray `.tmp` sibling behind.
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}
