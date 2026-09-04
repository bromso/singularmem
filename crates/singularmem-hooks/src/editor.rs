//! The three supported editors and the hook events they emit, plus the
//! per-editor config file location.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::error::{Error, Result};

/// An editor Singularmem can wire hooks into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Editor {
    /// Anthropic's Claude Code CLI.
    ClaudeCode,
    /// `OpenAI`'s Codex CLI.
    Codex,
    /// The Cursor editor.
    Cursor,
}

impl Editor {
    /// The editor's identifier as used on the command line and in hook
    /// commands, e.g. `"claude-code"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}

impl fmt::Display for Editor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Editor {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "claude-code" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "cursor" => Ok(Self::Cursor),
            other => Err(ParseError::new("editor", other)),
        }
    }
}

/// A hook event an editor can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    /// Fired when a new session begins (startup, resume, clear, or after
    /// a compaction).
    SessionStart,
    /// Fired when the assistant turn ends.
    Stop,
    /// Fired immediately before context compaction.
    PreCompact,
    /// Fired when the session ends entirely.
    SessionEnd,
}

impl Event {
    /// The event's identifier as used on the command line and in hook
    /// commands, e.g. `"session-start"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::Stop => "stop",
            Self::PreCompact => "pre-compact",
            Self::SessionEnd => "session-end",
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Event {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "session-start" => Ok(Self::SessionStart),
            "stop" => Ok(Self::Stop),
            "pre-compact" => Ok(Self::PreCompact),
            "session-end" => Ok(Self::SessionEnd),
            other => Err(ParseError::new("event", other)),
        }
    }
}

/// Returned by [`Editor::from_str`] and [`Event::from_str`] when the input
/// does not match a known identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    kind: &'static str,
    value: String,
}

impl ParseError {
    fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {}: {:?}", self.kind, self.value)
    }
}

impl std::error::Error for ParseError {}

/// The absolute path of the editor's config file: `<base>/.claude/settings.json`,
/// `<base>/.codex/hooks.json`, or `<base>/.cursor/hooks.json`.
///
/// `base` is `project` when given, otherwise the user's home directory
/// (`HOME`, falling back to `USERPROFILE` on Windows), read from the
/// environment at call time.
///
/// # Errors
///
/// Returns [`Error::NoHome`] when `project` is `None` and neither `HOME`
/// nor `USERPROFILE` is set.
pub fn config_path(editor: Editor, project: Option<&Path>) -> Result<PathBuf> {
    let (dir, file) = match editor {
        Editor::ClaudeCode => (".claude", "settings.json"),
        Editor::Codex => (".codex", "hooks.json"),
        Editor::Cursor => (".cursor", "hooks.json"),
    };
    let base = match project {
        Some(p) => p.to_path_buf(),
        None => std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or(Error::NoHome)?,
    };
    Ok(base.join(dir).join(file))
}
