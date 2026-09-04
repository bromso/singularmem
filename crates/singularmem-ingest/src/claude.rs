//! Claude Code JSONL transcript source.
//!
//! One `NewItem` per user/assistant message that carries text. Tool
//! payloads, tool results, and thinking blocks are skipped; tool names
//! are recorded in metadata.

use std::cell::Cell;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::Source;

/// A single Claude Code session file.
#[derive(Debug)]
pub struct ClaudeTranscript {
    /// Path to the `.jsonl` file.
    pub path: PathBuf,
    /// Keep messages flagged `isSidechain` (subagent conversations).
    pub include_sidechains: bool,
    /// When set, keep only messages whose `cwd` names this directory
    /// (compared both as raw paths and, when both resolve, canonically —
    /// so `/tmp/x` matches a transcript recorded under `/private/tmp/x`).
    pub project_filter: Option<PathBuf>,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    /// Explicit scope override; wins over the `cwd`-derived default. Left
    /// `None` for [`ClaudeTranscript::default_scope`] to derive one.
    pub scope_override: Option<String>,
    filtered: Cell<usize>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    parent_uuid: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    content: Option<serde_json::Value>,
}

/// A resolved `--project` filter: the raw path, its canonical form when it
/// resolves, and a one-entry memo so a transcript whose thousands of lines
/// share one `cwd` costs a single `canonicalize` call.
#[derive(Debug)]
struct ProjectFilter {
    raw: PathBuf,
    canonical: Option<PathBuf>,
    memo: Option<(String, bool)>,
}

impl ProjectFilter {
    fn new(raw: &Path) -> Self {
        Self {
            raw: raw.to_path_buf(),
            canonical: raw.canonicalize().ok(),
            memo: None,
        }
    }

    /// True when `cwd` names the same directory as the filter: equal as raw
    /// paths, or equal once both sides canonicalize successfully. A `cwd`
    /// that no longer exists on this machine can still match by raw path.
    fn matches(&mut self, cwd: Option<&str>) -> bool {
        let Some(cwd) = cwd else { return false };
        if let Some((seen, verdict)) = &self.memo {
            if seen == cwd {
                return *verdict;
            }
        }
        let verdict = self.raw.as_path() == Path::new(cwd)
            || match (&self.canonical, Path::new(cwd).canonicalize().ok()) {
                (Some(a), Some(b)) => *a == b,
                _ => false,
            };
        self.memo = Some((cwd.to_string(), verdict));
        verdict
    }
}

impl ClaudeTranscript {
    /// Open a transcript file. Fails with `Error::NotFound` if it is missing.
    ///
    /// # Errors
    /// `Error::NotFound` when `path` does not exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(Error::NotFound { path });
        }
        Ok(Self {
            path,
            include_sidechains: false,
            project_filter: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            scope_override: None,
            filtered: Cell::new(0),
        })
    }

    fn file_stem(&self) -> String {
        self.path.file_stem().map_or_else(
            || "unknown".to_string(),
            |s| s.to_string_lossy().into_owned(),
        )
    }

    /// Convert one parsed line into items, or `None` if it is filtered.
    ///
    /// `None` means "structural line, not a message" (not counted as
    /// filtered); `Some(vec![])` means "message deliberately filtered"
    /// (counted). See [`Source::filtered_count`].
    #[allow(
        clippy::too_many_lines,
        reason = "linear per-field extraction; splitting would obscure the single control flow"
    )]
    fn line_to_items(
        &self,
        line: Line,
        filter: Option<&mut ProjectFilter>,
    ) -> Option<Vec<NewItem>> {
        if line.kind != "user" && line.kind != "assistant" {
            return None; // structural line; not counted as filtered
        }
        let role = line.kind;
        if line.is_meta.unwrap_or(false) {
            return Some(Vec::new());
        }
        let sidechain = line.is_sidechain.unwrap_or(false);
        if sidechain && !self.include_sidechains {
            return Some(Vec::new());
        }
        if let Some(filter) = filter {
            if !filter.matches(line.cwd.as_deref()) {
                return Some(Vec::new());
            }
        }
        let Some(uuid) = line.uuid else {
            tracing::warn!(path = %self.path.display(), "message line without uuid; skipped");
            return Some(Vec::new());
        };
        let Some(msg) = line.message else {
            return Some(Vec::new());
        };
        let Some(content) = msg.content.as_ref() else {
            return Some(Vec::new());
        };
        let (text, tool_names) = extract_text(content);
        let text = if role == "user" {
            strip_system_reminders(&text)
        } else {
            text
        };
        let chunks = chunk_text(&text, self.chunk_bytes);
        if chunks.is_empty() {
            return Some(Vec::new());
        }
        let session_id = line.session_id.unwrap_or_else(|| self.file_stem());
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
                    "claude-code".to_string(),
                    format!("role:{role}"),
                    "transcript".to_string(),
                ];
                if sidechain {
                    tags.push("sidechain".to_string());
                }
                tags.sort();
                let external_id = if chunk_count == 1 {
                    format!("claude-code:{session_id}:{uuid}")
                } else {
                    format!("claude-code:{session_id}:{uuid}#{i}")
                };
                NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("claude-code:{session_id}")),
                    metadata: serde_json::json!({
                        "session_id": session_id,
                        "uuid": uuid,
                        "parent_uuid": line.parent_uuid,
                        "role": role,
                        "cwd": line.cwd,
                        "git_branch": line.git_branch,
                        "occurred_at": occurred_at,
                        "tool_names": tool_names,
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

    /// Derive `claude-code/<cwd-basename>` from `item.metadata.cwd`, or
    /// `None` if `cwd` is absent, has no basename, or the basename is not a
    /// valid scope segment.
    fn derived_scope(item: &NewItem) -> Option<String> {
        let cwd = item.metadata.get("cwd")?.as_str()?;
        let base = Path::new(cwd).file_name()?.to_str()?;
        match singularmem_core::scope::validate(&format!("claude-code/{base}")) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(cwd, error = %e, "cwd basename is not a valid scope segment; item left unscoped");
                None
            }
        }
    }
}

/// Concatenate text blocks and collect tool names from a message `content`.
fn extract_text(content: &serde_json::Value) -> (String, Vec<String>) {
    match content {
        serde_json::Value::String(s) => (s.clone(), Vec::new()),
        serde_json::Value::Array(blocks) => {
            let mut texts = Vec::new();
            let mut tools = Vec::new();
            for b in blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                            texts.push(t.to_string());
                        }
                    }
                    Some("tool_use") => {
                        if let Some(n) = b.get("name").and_then(|n| n.as_str()) {
                            tools.push(n.to_string());
                        }
                    }
                    _ => {}
                }
            }
            (texts.join("\n\n"), tools)
        }
        _ => (String::new(), Vec::new()),
    }
}

/// Remove every `<system-reminder>…</system-reminder>` span (an unterminated
/// opening tag removes through end of input) and trim.
#[must_use]
pub fn strip_system_reminders(s: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        match after.find(CLOSE) {
            Some(end) => rest = &after[end + CLOSE.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

impl Source for ClaudeTranscript {
    fn name(&self) -> String {
        self.path.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(source) => {
                return Box::new(std::iter::once(Err(Error::Io {
                    path: self.path.clone(),
                    source,
                })));
            }
        };
        let reader = BufReader::new(file);
        // Resolve the filter once per run, not once per line.
        let mut filter = self.project_filter.as_deref().map(ProjectFilter::new);
        let iter = reader.lines().enumerate().flat_map(move |(idx, line)| {
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
            self.line_to_items(parsed, filter.as_mut())
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
        self.scope_override.as_ref().map_or_else(
            || Self::derived_scope(item),
            |o| singularmem_core::scope::validate(o).ok(),
        )
    }
}

/// Recursively find `*.jsonl` files under `root`, sorted by path.
///
/// # Errors
/// `Error::NotFound` if `root` does not exist; `Error::Io` on read failure.
pub fn discover_transcripts(root: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    if !root.exists() {
        return Err(Error::NotFound {
            path: root.to_path_buf(),
        });
    }
    // A transcript tree is not a source tree: take every `*.jsonl`, hidden
    // or not; no ignore files of any kind, in the tree or its ancestors, are
    // consulted; and never follow a symlink out of the tree.
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .follow_links(false)
        .sort_by_file_path(std::cmp::Ord::cmp)
        .build();
    let mut out = Vec::new();
    for entry in walker {
        let entry = entry.map_err(|e| Error::Io {
            path: crate::dir::walk_error_path(&e, root),
            source: std::io::Error::other(e.to_string()),
        })?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if entry.path().extension().is_some_and(|e| e == "jsonl") {
            out.push(entry.into_path());
        }
    }
    // `sort_by_file_path` only orders siblings within each directory before
    // recursing, not the fully-recursed path lexicographically — e.g. a
    // sibling directory `b` (containing `inner.jsonl`) and a file
    // `b-file.jsonl` sort as `b < b-file.jsonl` at that level (name-prefix
    // rule) and so are visited in that order, but `"b-file.jsonl"` sorts
    // before `"b/inner.jsonl"` as full path strings (`-` < `/`). So the walk
    // order is not guaranteed to match a global path sort; keep this.
    out.sort();
    Ok(out)
}
