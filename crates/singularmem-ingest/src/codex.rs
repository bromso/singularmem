//! `OpenAI` Codex CLI rollout source (`~/.codex/sessions/**/rollout-*.jsonl`).
//!
//! The line schema is community-documented, not official; this parser is
//! defensive: anything that is not a `response_item` message with text is
//! ignored.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::project_filter::{derive_scope, ProjectFilter};
use crate::Source;

/// One Codex rollout file.
#[derive(Debug)]
pub struct CodexRollout {
    /// Path to the `.jsonl` file.
    pub path: PathBuf,
    /// Keep only messages from sessions whose `cwd` names this directory.
    pub project_filter: Option<PathBuf>,
    /// Explicit scope override; wins over `codex/<cwd basename>`.
    pub scope_override: Option<String>,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
    /// Memoises the last `(cwd, derived scope)` pair so a rollout whose
    /// lines share one `cwd` only warns once on an invalid basename, rather
    /// than once per item.
    derived_memo: RefCell<Option<(String, Option<String>)>>,
    /// Set when a `session_meta` line was parsed during the most recent
    /// `items()` call; reset at the top of `items()`.
    session_meta_seen: Cell<bool>,
}

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    payload: Option<serde_json::Value>,
}

/// Session-level facts read from the `session_meta` line (or fallbacks).
struct Session {
    id: String,
    cwd: Option<String>,
}

impl CodexRollout {
    /// Open a rollout file.
    ///
    /// # Errors
    /// `Error::NotFound` when `path` is not a file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(Error::NotFound { path });
        }
        Ok(Self {
            path,
            project_filter: None,
            scope_override: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
            derived_memo: RefCell::new(None),
            session_meta_seen: Cell::new(false),
        })
    }

    /// Whether a `session_meta` line was parsed during the most recent
    /// [`Source::items`] call. `false` for legacy rollouts (or any rollout
    /// whose `session_meta` line, if present, was never reached).
    #[must_use]
    pub fn session_meta_seen(&self) -> bool {
        self.session_meta_seen.get()
    }

    fn file_stem(&self) -> String {
        self.path.file_stem().map_or_else(
            || "unknown".to_string(),
            |s| s.to_string_lossy().into_owned(),
        )
    }

    /// Build items for one `response_item` message line. `None` = structural
    /// (not counted); `Some(vec![])` = deliberately filtered (counted).
    #[allow(
        clippy::too_many_lines,
        reason = "linear per-field extraction; splitting would obscure the single control flow"
    )]
    fn line_to_items(
        &self,
        line_no: usize,
        line: &Line,
        session: &Session,
        filter: Option<&mut ProjectFilter>,
    ) -> Option<Vec<NewItem>> {
        if line.kind != "response_item" {
            return None;
        }
        let payload = line.payload.as_ref()?;
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
            return Some(Vec::new()); // function_call, function_call_output, reasoning, …
        }
        let role = payload.get("role").and_then(|r| r.as_str())?;
        if role != "user" && role != "assistant" {
            return Some(Vec::new());
        }
        if let Some(f) = filter {
            if !f.matches(session.cwd.as_deref()) {
                return Some(Vec::new());
            }
        }
        let content = payload.get("content");
        let text: String = content.and_then(|c| c.as_str()).map_or_else(
            || {
                content
                    .and_then(|c| c.as_array())
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter(|b| {
                                matches!(
                                    b.get("type").and_then(|t| t.as_str()),
                                    Some("input_text" | "output_text" | "text")
                                )
                            })
                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<&str>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default()
            },
            str::to_string,
        );
        let chunks = chunk_text(&text, self.chunk_bytes);
        if chunks.is_empty() {
            return Some(Vec::new());
        }
        let occurred_at = line
            .timestamp
            .as_deref()
            .and_then(|t| t.parse::<jiff::Timestamp>().ok())
            .map(|t| t.to_string());
        let chunk_count = chunks.len();
        let items = chunks
            .into_iter()
            .enumerate()
            .map(|(i, content)| {
                let mut tags = vec![
                    "codex".to_string(),
                    format!("role:{role}"),
                    "transcript".to_string(),
                ];
                tags.sort();
                let external_id = if chunk_count == 1 {
                    format!("codex:{}:{line_no}", session.id)
                } else {
                    format!("codex:{}:{line_no}#{i}", session.id)
                };
                NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("codex:{}", session.id)),
                    metadata: serde_json::json!({
                        "session_id": session.id,
                        "line": line_no,
                        "role": role,
                        "cwd": session.cwd,
                        "occurred_at": occurred_at,
                        "chunk_index": i,
                        "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                    scope: None,
                }
            })
            .collect();
        Some(items)
    }

    /// Derive `codex/<cwd-basename>` from `item.metadata.cwd`, or `None` if
    /// `cwd` is absent, has no basename, or the basename is not a valid
    /// scope segment. Memoised per `cwd` so a rollout whose lines share one
    /// `cwd` only logs once.
    fn derived_scope(&self, item: &NewItem) -> Option<String> {
        let cwd = item.metadata.get("cwd")?.as_str()?;
        if let Some((seen, result)) = self.derived_memo.borrow().as_ref() {
            if seen == cwd {
                return result.clone();
            }
        }
        let result = derive_scope("codex", cwd);
        *self.derived_memo.borrow_mut() = Some((cwd.to_string(), result.clone()));
        result
    }
}

impl Source for CodexRollout {
    fn name(&self) -> String {
        self.path.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        self.session_meta_seen.set(false);
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(source) => {
                return Box::new(std::iter::once(Err(Error::Io {
                    path: self.path.clone(),
                    source,
                })));
            }
        };
        let mut filter = self.project_filter.as_deref().map(ProjectFilter::new);
        let mut session = Session {
            id: self.file_stem(),
            cwd: None,
        };
        // Tracks whether we've reached the first successfully parsed line yet
        // (blank and malformed lines don't count), so the "no session_meta"
        // warning fires exactly once, based on that line's kind — not on
        // `line_no == 1`, which never fires when line 1 is blank or malformed.
        let mut first_parsed_line_seen = false;
        let iter = BufReader::new(file)
            .lines()
            .enumerate()
            .flat_map(move |(idx, line)| {
                let line_no = idx + 1;
                let raw = match line {
                    Ok(l) => l,
                    Err(source) => {
                        return vec![Err(Error::Io {
                            path: self.path.clone(),
                            source,
                        })]
                    }
                };
                if raw.trim().is_empty() {
                    return Vec::new();
                }
                let parsed: Line = match serde_json::from_str(&raw) {
                    Ok(p) => p,
                    Err(source) => {
                        return vec![Err(Error::Json {
                            path: self.path.clone(),
                            line: line_no,
                            source,
                        })]
                    }
                };
                let is_first_parsed_line = !first_parsed_line_seen;
                first_parsed_line_seen = true;
                if parsed.kind == "session_meta" {
                    self.session_meta_seen.set(true);
                    if let Some(p) = &parsed.payload {
                        if let Some(id) = p.get("id").and_then(|v| v.as_str()) {
                            session.id = id.to_string();
                        }
                        session.cwd = p.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
                    }
                    return Vec::new();
                }
                if is_first_parsed_line {
                    tracing::warn!(
                        path = %self.path.display(),
                        "rollout has no session_meta line; using file stem as session id"
                    );
                }
                self.line_to_items(line_no, &parsed, &session, filter.as_mut())
                    .map_or_else(Vec::new, |items| {
                        if items.is_empty() {
                            self.filtered.set(self.filtered.get() + 1);
                        }
                        items.into_iter().map(Ok).collect()
                    })
            });
        Box::new(iter)
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }

    fn default_scope(&self, item: &NewItem) -> Option<String> {
        if let Some(o) = &self.scope_override {
            match singularmem_core::scope::validate(o) {
                Ok(s) => return Some(s),
                Err(e) => {
                    tracing::warn!(
                        r#override = %o,
                        error = %e,
                        "ignoring invalid scope override; using derived scope"
                    );
                }
            }
        }
        self.derived_scope(item)
    }
}

/// `$HOME/.codex/sessions`, if a home directory is known.
///
/// Falls back to `%USERPROFILE%` when `HOME` is unset, so the default
/// resolves on Windows too — the same `HOME`-then-`USERPROFILE` fallback
/// the hooks crate's `config_path` uses. This is *not* what
/// [`crate::default_cursor_user_dir`] does: that function keys off
/// `target_os` directly and uses `%APPDATA%` (not `USERPROFILE`) on
/// Windows, `HOME` everywhere else, with no fallback between the two.
#[must_use]
pub fn default_codex_root() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".codex").join("sessions"))
}

/// Recursively find `rollout-*.jsonl` files under `root`, sorted.
///
/// # Errors
/// `Error::NotFound` if `root` does not exist; `Error::Io` on read failure.
pub fn discover_codex_sessions(root: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let mut all = crate::claude::discover_transcripts(root)?;
    all.retain(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-"))
    });
    Ok(all)
}
