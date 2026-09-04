//! Wrapping wake-up text in the shape each editor expects on its
//! session-start hook's stdout.

use serde_json::{json, Value};

use crate::editor::Editor;

/// Build the session-start hook output envelope for `editor`, carrying
/// `text` (typically the rendered wake-up context) as additional context.
#[must_use]
pub fn session_start_envelope(editor: Editor, text: &str) -> Value {
    match editor {
        Editor::ClaudeCode | Editor::Codex => json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": text,
            }
        }),
        Editor::Cursor => json!({ "additional_context": text }),
    }
}
