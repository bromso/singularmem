//! Building our hook config entries and merging/removing/inspecting them
//! inside an editor's config file, plus atomic read/write of that file.
//!
//! ## Detecting "ours"
//!
//! The command string we write is `"<bin>" hook <editor> <event>` — the
//! binary path is double-quoted. A naive `contains("singularmem hook ")`
//! check breaks when the binary itself is named `singularmem`, because the
//! closing quote lands between the name and `hook`: `"...singularmem" hook
//! ...` does *not* contain the substring `singularmem hook ` (the quote is
//! in the way). Instead, [`MARKER`] is the closing quote plus `" hook "`,
//! and [`is_ours`] additionally requires the command to mention
//! `singularmem` somewhere (almost always in the binary path), which
//! together are specific enough to avoid matching an unrelated command
//! that happens to invoke some other tool's `hook` subcommand.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::editor::{Editor, Event};
use crate::error::{Error, Result};

/// Substring present in every command we write.
///
/// This is the closing quote around the binary path, followed by ` hook `.
/// Kept public so callers can sanity check a command string directly;
/// prefer [`is_ours`] for classification.
pub const MARKER: &str = "\" hook ";

/// Whether `command` is one of ours.
///
/// Requires both the quoted-`hook` shape ([`MARKER`]) and a mention of
/// `singularmem`, since the marker alone (`" hook "`) could in principle
/// appear in an unrelated command.
#[must_use]
pub fn is_ours(command: &str) -> bool {
    command.contains(MARKER) && command.contains("singularmem")
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

    let mut hooks_obj = result
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

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
    let Some(hooks_obj) = result.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(result);
    };

    let mut new_hooks = Map::new();
    for (event_key, value) in hooks_obj {
        match value.as_array() {
            Some(arr) => {
                let retained: Vec<Value> = arr
                    .iter()
                    .filter(|group| !group_is_ours(editor, group))
                    .cloned()
                    .collect();
                if !retained.is_empty() {
                    new_hooks.insert(event_key, Value::Array(retained));
                }
            }
            None => {
                new_hooks.insert(event_key, value);
            }
        }
    }

    if matches!(editor, Editor::Cursor) || !new_hooks.is_empty() {
        result.insert("hooks".to_string(), Value::Object(new_hooks));
    } else {
        result.remove("hooks");
    }

    Value::Object(result)
}

/// Extract the binary path from a command string: the text between the
/// leading `"` and the next `"`.
fn extract_bin(command: &str) -> Option<PathBuf> {
    let rest = command.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(PathBuf::from(&rest[..end]))
}

/// Find the binary path recorded in the first of our entries in `existing`,
/// scanning every event array under `hooks`.
fn find_bin(editor: Editor, existing: &Value) -> Option<PathBuf> {
    let hooks_obj = existing.get("hooks")?.as_object()?;
    for arr in hooks_obj.values() {
        let Some(arr) = arr.as_array() else {
            continue;
        };
        for group in arr {
            if !group_is_ours(editor, group) {
                continue;
            }
            let command = match editor {
                Editor::Cursor => group.get("command").and_then(Value::as_str),
                Editor::ClaudeCode | Editor::Codex => group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .and_then(|hooks| {
                        hooks
                            .iter()
                            .find_map(|h| h.get("command").and_then(Value::as_str))
                    }),
            };
            if let Some(bin) = command.and_then(extract_bin) {
                return Some(bin);
            }
        }
    }
    None
}

/// Inspect `existing` for our hooks: whether installed, which binary path
/// they point at, and whether that binary still exists on disk.
#[must_use]
pub fn status(editor: Editor, existing: &Value) -> HookStatus {
    let bin = find_bin(editor, existing);
    let bin_exists = bin.as_deref().is_some_and(Path::exists);
    HookStatus {
        installed: bin.is_some(),
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

    std::fs::write(&tmp_path, &text).map_err(|source| Error::Io {
        path: tmp_path.clone(),
        source,
    })?;
    std::fs::rename(&tmp_path, path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}
