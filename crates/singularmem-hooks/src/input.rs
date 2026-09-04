//! Parsing an editor's hook stdin payload into a normalised [`HookInput`].

use std::path::PathBuf;

use serde_json::Value;

use crate::editor::Editor;

/// Fields common to every editor's hook payload, normalised to one shape.
///
/// Editors do not all send every field; absent or malformed fields become
/// `None` (or an empty vector for `workspace_roots`) rather than an error —
/// hooks must never block the editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookInput {
    /// The working directory the editor reports, used to derive the
    /// project's default scope. Falls back to the first entry of
    /// `workspace_roots` when `cwd` itself is absent.
    pub cwd: Option<PathBuf>,
    /// Every workspace root the editor reports (Cursor may report several).
    pub workspace_roots: Vec<PathBuf>,
    /// Path to the transcript file to ingest on a save event, when the
    /// editor provides one directly.
    pub transcript_path: Option<PathBuf>,
    /// The editor's session identifier.
    pub session_id: Option<String>,
    /// Cursor's conversation (composer) identifier.
    pub conversation_id: Option<String>,
}

/// Parse an editor's hook JSON payload.
///
/// Field extraction is identical across editors — each editor simply omits
/// whatever fields it does not have — so `editor` is accepted for API
/// symmetry with the rest of this crate and to leave room for per-editor
/// divergence later, but is not consulted today. Non-object `json` (or a
/// missing/wrong-typed field) yields `None`/empty rather than an error.
#[must_use]
pub fn parse_input(_editor: Editor, json: &Value) -> HookInput {
    let Some(obj) = json.as_object() else {
        return HookInput::default();
    };

    let workspace_roots: Vec<PathBuf> = obj
        .get("workspace_roots")
        .and_then(Value::as_array)
        .map(|roots| {
            roots
                .iter()
                .filter_map(Value::as_str)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();

    let cwd = obj
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| workspace_roots.first().cloned());

    let transcript_path = obj
        .get("transcript_path")
        .and_then(Value::as_str)
        .map(PathBuf::from);

    let session_id = obj
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let conversation_id = obj
        .get("conversation_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    HookInput {
        cwd,
        workspace_roots,
        transcript_path,
        session_id,
        conversation_id,
    }
}
